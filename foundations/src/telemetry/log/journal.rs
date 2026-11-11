//! Log output to the systemd journal that never waits for journald.
//!
//! systemd connects a service's stdout and stderr to a journald socket and names it in
//! `JOURNAL_STREAM`. If journald stops reading the socket while keeping it open, which a bug
//! can leave it doing for good, the socket's send buffer fills and every blocking write to it
//! waits for good: the logging thread, then [`TelemetryDriver::shutdown_logger`], and any thread
//! that panics, because the default panic hook writes the message to stderr.
//!
//! So log records go to the journal stream through [`JournalWriter`], which never waits, and
//! unless [`LoggingSettings::allow_blocking_journal_stdio`] is on, the stream is made nonblocking
//! for every other writer too.
//!
//! [`TelemetryDriver::shutdown_logger`]: crate::telemetry::TelemetryDriver::shutdown_logger
//! [`LoggingSettings::allow_blocking_journal_stdio`]: crate::telemetry::settings::LoggingSettings::allow_blocking_journal_stdio

use crate::telemetry::settings::LogOutput;
use slog::{Drain, OwnedKVList, Record};
use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// A record buffer larger than this is freed after use, rather than kept for the next record.
const MAX_RETAINED_RECORD_CAPACITY: usize = 64 * 1024;

/// Which of stdout and stderr are the journal stream.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct JournalStreams {
    stdout: bool,
    stderr: bool,
}

impl JournalStreams {
    /// Finds which of stdout and stderr are the journal stream, and makes them nonblocking if
    /// `nonblocking` is set.
    ///
    /// A check that fails counts as not the journal, and a descriptor whose flags can't be set
    /// stays as it was, so this never stops logging from starting.
    pub(super) fn detect(nonblocking: bool) -> Self {
        let Some(journal_stream) = std::env::var_os("JOURNAL_STREAM") else {
            return Self::default();
        };
        let detect = |fd: BorrowedFd<'_>| {
            let is_journal = is_journal(fd, &journal_stream);
            if is_journal && nonblocking {
                set_nonblocking(fd);
            }
            is_journal
        };

        Self {
            stdout: detect(io::stdout().as_fd()),
            stderr: detect(io::stderr().as_fd()),
        }
    }

    /// Whether `output` writes to the journal stream.
    pub(super) fn includes(&self, output: &LogOutput) -> bool {
        match output {
            LogOutput::Terminal => self.stdout,
            LogOutput::Stderr => self.stderr,
            _ => false,
        }
    }
}

/// The stdout or stderr descriptor that `output` writes to.
pub(super) fn stdio_fd(output: &LogOutput) -> BorrowedFd<'static> {
    let fd = if matches!(output, LogOutput::Terminal) {
        libc::STDOUT_FILENO
    } else {
        libc::STDERR_FILENO
    };
    // SAFETY: std treats stdout and stderr as open for the life of the process. Borrowing
    // rather than owning them means the logging thread never closes them when it exits, which
    // would let the next file or socket opened take their numbers.
    unsafe { BorrowedFd::borrow_raw(fd) }
}

/// Whether `fd` is the socket named by `journal_stream`, the way systemd names it.
fn is_journal(fd: BorrowedFd<'_>, journal_stream: &OsStr) -> bool {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `fd` is borrowed, and fstat initializes `metadata` on success.
    if unsafe { libc::fstat(fd.as_raw_fd(), metadata.as_mut_ptr()) } == -1 {
        return false;
    }
    // SAFETY: fstat succeeded.
    let metadata = unsafe { metadata.assume_init() };

    metadata.st_mode & libc::S_IFMT == libc::S_IFSOCK
        && journal_stream.as_encoded_bytes()
            == format!("{}:{}", metadata.st_dev, metadata.st_ino).as_bytes()
}

/// Sets `O_NONBLOCK` on `fd`, leaving it as it was if that fails.
fn set_nonblocking(fd: BorrowedFd<'_>) {
    // SAFETY: F_GETFL takes no argument and does not close `fd`.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags != -1 && flags & libc::O_NONBLOCK == 0 {
        // SAFETY: F_SETFL takes an integer argument and does not close `fd`.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
}

/// Writes each log record to the journal stream without ever waiting, and drops the record if
/// the stream has no room for it.
///
/// Records are buffered until the formatter flushes at the end of each one, then sent at once.
/// Sends pass `MSG_DONTWAIT`, so they don't wait even if something sharing the descriptor makes
/// it blocking. A record that only partly fits is cut short, and the next record that gets
/// through starts with a newline, so journald doesn't join the two into one entry.
pub(super) struct JournalWriter<W> {
    out: W,
    /// A newline, then the record being formatted. The newline is sent only to end a record
    /// that was cut short.
    buf: Vec<u8>,
    cut_short: bool,
    new_record: Arc<AtomicBool>,
}

impl<W: AsFd> JournalWriter<W> {
    /// Builds the formatting drain for records to `out`, and tells its writer where each
    /// record starts, including after a record that failed to format.
    pub(super) fn drain<D>(out: W, build: impl FnOnce(Self) -> D) -> JournalDrain<D> {
        metrics::register();
        let new_record = Arc::new(AtomicBool::new(true));
        let writer = Self {
            out,
            buf: vec![b'\n'],
            cut_short: false,
            new_record: Arc::clone(&new_record),
        };

        JournalDrain {
            inner: build(writer),
            new_record,
        }
    }

    fn clear(&mut self) {
        self.buf.truncate(1);
        self.buf.shrink_to(MAX_RETAINED_RECORD_CAPACITY);
    }

    /// Sends the buffered record from `start`, in as many sends as the stream takes without
    /// waiting, and counts it if it's dropped.
    fn send(&mut self, start: usize) -> io::Result<()> {
        let record = &self.buf[start..];
        let mut sent = 0;
        let res = loop {
            match send_nonblocking(self.out.as_fd(), &record[sent..]) {
                Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    sent += n;
                    if sent == record.len() {
                        break Ok(());
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => break Err(err),
            }
        };
        if let Some(&last) = record[..sent].last() {
            self.cut_short = last != b'\n';
        }

        match res {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                metrics::count(DropReason::JournalFull);
                Ok(())
            }
            Err(err) => {
                metrics::count(DropReason::JournalError);
                Err(err)
            }
            Ok(()) => Ok(()),
        }
    }
}

/// Sends without waiting, whatever the descriptor's own blocking mode is.
///
/// If something replaced stdout or stderr with a file that isn't a socket after logging
/// started, this writes to it the way any other writer would, which waits if that file does.
fn send_nonblocking(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: `buf` is valid for `buf.len()` bytes, and send does not take ownership of `fd`.
    let sent = unsafe {
        libc::send(
            fd.as_raw_fd(),
            buf.as_ptr().cast(),
            buf.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    if sent >= 0 {
        return Ok(sent as usize);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::ENOTSOCK) {
        return Err(err);
    }

    // SAFETY: `buf` is valid for `buf.len()` bytes, and write does not take ownership of `fd`.
    let written = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(written as usize)
}

impl<W: AsFd> io::Write for JournalWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.new_record.load(Ordering::Relaxed) {
            self.new_record.store(false, Ordering::Relaxed);
            // A formatter that fails part way through a record doesn't flush it. Discard what
            // it wrote instead of sending it with the next record.
            self.clear();
        }
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    /// Sends the buffered record. The log formatters flush at the end of each record.
    fn flush(&mut self) -> io::Result<()> {
        if self.buf.len() == 1 {
            return Ok(());
        }
        let res = self.send(usize::from(!self.cut_short));
        self.clear();
        res
    }
}

/// Marks the start of each record for its [`JournalWriter`].
pub(super) struct JournalDrain<D> {
    inner: D,
    new_record: Arc<AtomicBool>,
}

impl<D: Drain> Drain for JournalDrain<D> {
    type Ok = D::Ok;
    type Err = D::Err;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        // The formatter and writer run on the same logging thread. The shared flag only
        // connects the drain to its writer; it does not synchronize threads.
        self.new_record.store(true, Ordering::Relaxed);
        self.inner.log(record, values)
    }
}

/// Why a log record was dropped instead of written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DropReason {
    /// The journal stream had no room for the record.
    JournalFull,
    /// Sending to the journal stream failed, for example because journald closed it.
    JournalError,
}

#[cfg(feature = "metrics")]
mod metrics {
    use super::DropReason;
    use crate::telemetry::metrics::Counter;
    use std::sync::LazyLock;

    #[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
    pub(crate) mod logging {
        /// Number of log records dropped instead of written.
        pub fn records_dropped(reason: DropReason) -> Counter;
    }

    struct Counters {
        journal_full: Counter,
        journal_error: Counter,
    }

    static COUNTERS: LazyLock<Counters> = LazyLock::new(|| Counters {
        journal_full: logging::records_dropped(DropReason::JournalFull),
        journal_error: logging::records_dropped(DropReason::JournalError),
    });

    /// Creates the counters at zero, so they exist before the first drop.
    pub(super) fn register() {
        LazyLock::force(&COUNTERS);
    }

    pub(super) fn count(reason: DropReason) {
        match reason {
            DropReason::JournalFull => &COUNTERS.journal_full,
            DropReason::JournalError => &COUNTERS.journal_error,
        }
        .inc();
    }
}

#[cfg(not(feature = "metrics"))]
mod metrics {
    pub(super) fn register() {}

    pub(super) fn count(_reason: super::DropReason) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    fn stream_id(fd: &impl AsFd) -> String {
        let metadata = std::fs::File::from(fd.as_fd().try_clone_to_owned().unwrap())
            .metadata()
            .unwrap();
        format!("{}:{}", metadata.dev(), metadata.ino())
    }

    /// A writer to a blocking journal stream, and journald's end of it.
    fn journal_pair() -> (JournalWriter<UnixStream>, UnixStream) {
        let (stream, journald) = UnixStream::pair().unwrap();
        let drain = JournalWriter::drain(stream, |writer| writer);
        (drain.inner, journald)
    }

    /// Fills `writer`'s stream so that nothing more fits, and leaves it blocking.
    fn fill(writer: &JournalWriter<UnixStream>) {
        let stream = &writer.out;
        stream.set_nonblocking(true).unwrap();
        loop {
            match (&*stream).write(&[b'\n'; 8192]) {
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => panic!("filling the journal stream: {err}"),
            }
        }
        stream.set_nonblocking(false).unwrap();
    }

    fn read_available(journald: &mut UnixStream) -> Vec<u8> {
        journald.set_nonblocking(true).unwrap();
        let mut received = Vec::new();
        let mut buf = [0; 64 * 1024];
        loop {
            match journald.read(&mut buf) {
                Ok(0) => return received,
                Ok(n) => received.extend_from_slice(&buf[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return received,
                Err(err) => panic!("read failed: {err}"),
            }
        }
    }

    /// Writes and flushes `record` on another thread, failing the test if that waits.
    fn log_without_waiting(
        mut writer: JournalWriter<UnixStream>,
        record: &[u8],
    ) -> (JournalWriter<UnixStream>, io::Result<()>) {
        let record = record.to_vec();
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            writer.new_record.store(true, Ordering::Relaxed);
            writer.write_all(&record).unwrap();
            let res = writer.flush();
            done_tx.send((writer, res)).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the write waited for journald")
    }

    #[test]
    fn only_the_named_socket_is_the_journal() {
        let (stream, _journald) = UnixStream::pair().unwrap();

        assert!(!is_journal(stream.as_fd(), OsStr::new("1:2")));
        assert!(is_journal(stream.as_fd(), OsStr::new(&stream_id(&stream))));
    }

    #[test]
    fn a_file_is_not_the_journal() {
        let file = tempfile::tempfile().unwrap();

        assert!(!is_journal(file.as_fd(), OsStr::new(&stream_id(&file))));
    }

    #[test]
    fn sends_each_record_when_flushed() {
        let (mut writer, mut journald) = journal_pair();

        writer.write_all(b"first ").unwrap();
        writer.write_all(b"record\n").unwrap();
        assert!(read_available(&mut journald).is_empty());
        writer.flush().unwrap();
        // The text formatter flushes a second time when it finishes a record.
        writer.flush().unwrap();
        writer.write_all(b"second record\n").unwrap();
        writer.flush().unwrap();

        assert_eq!(
            read_available(&mut journald),
            b"first record\nsecond record\n"
        );
    }

    #[test]
    fn drops_records_that_do_not_fit_without_waiting() {
        let (writer, mut journald) = journal_pair();
        fill(&writer);

        let (writer, res) = log_without_waiting(writer, b"dropped record\n");
        res.unwrap();
        let filled = read_available(&mut journald);
        assert!(filled.iter().all(|&b| b == b'\n'));

        let (_writer, res) = log_without_waiting(writer, b"next record\n");
        res.unwrap();
        assert_eq!(read_available(&mut journald), b"next record\n");
    }

    #[test]
    fn ends_a_record_cut_short_before_the_next_one() {
        let (writer, mut journald) = journal_pair();
        // Far larger than a socket send buffer normally is, so only part of it fits.
        let big = format!("{}\n", "x".repeat(32 * 1024 * 1024));

        let (writer, res) = log_without_waiting(writer, big.as_bytes());
        res.unwrap();
        let received = read_available(&mut journald);
        assert!(!received.is_empty() && received.len() < big.len());
        assert!(!received.contains(&b'\n'));

        let (_writer, res) = log_without_waiting(writer, b"next record\n");
        res.unwrap();
        assert_eq!(read_available(&mut journald), b"\nnext record\n");
    }

    #[test]
    fn reports_a_closed_journal_as_an_error() {
        let (writer, journald) = journal_pair();
        drop(journald);

        let (_writer, res) = log_without_waiting(writer, b"record\n");
        assert_eq!(res.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
}
