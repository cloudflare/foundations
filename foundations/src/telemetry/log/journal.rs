//! Log output to the systemd journal that never hangs indefinitely on a stalled journald.
//!
//! systemd connects a service's stdout and stderr to a journald socket and names it in
//! `JOURNAL_STREAM`. If journald stops reading the socket while keeping it open, which a bug
//! can leave it doing for good, the socket's send buffer fills and every blocking write to it
//! waits for good: the logging thread, then [`TelemetryDriver::shutdown_logger`], and any thread
//! that panics, because the default panic hook writes the message to stderr.
//!
//! When [`JournalStreamBehavior::NonBlockingLogger`] or
//! [`JournalStreamBehavior::NonBlockingStdio`] is configured, log records go to the journal
//! stream through [`JournalWriter`]. [`JournalWriter`] waits briefly via `poll` when the socket
//! buffer is full so normal bursts and large records aren't dropped or truncated, then switches to
//! immediate nonblocking drops once the stream stays full past the stall timeout until journald
//! drains it again. With [`JournalStreamBehavior::NonBlockingStdio`], the stream is also made
//! nonblocking via `O_NONBLOCK` for every other writer.
//!
//! [`TelemetryDriver::shutdown_logger`]: crate::telemetry::TelemetryDriver::shutdown_logger
//! [`JournalStreamBehavior::NonBlockingLogger`]: crate::telemetry::settings::JournalStreamBehavior::NonBlockingLogger
//! [`JournalStreamBehavior::NonBlockingStdio`]: crate::telemetry::settings::JournalStreamBehavior::NonBlockingStdio

use crate::telemetry::settings::LogOutput;
use slog::{Drain, OwnedKVList, Record};
use std::cell::Cell;
use std::ffi::OsStr;
use std::io;
use std::marker::PhantomData;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// Avoid startup reallocations on the first record by matching other writer's buffer size.
const INITIAL_RECORD_CAPACITY: usize = super::init::BUF_SIZE;

// A record buffer larger than this is freed after use, rather than kept for the next record.
const MAX_RETAINED_RECORD_CAPACITY: usize = 64 * 1024;

// How long to wait for a full journal socket to become writable before declaring it stalled.
// Short enough that shutdown never hangs, long enough to absorb normal bursts and large records.
const STALL_WAIT_TIMEOUT: Duration = Duration::from_millis(50);

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

    /// Returns the stdout or stderr descriptor if `output` writes to the journal stream.
    pub(super) fn fd(&self, output: &LogOutput) -> Option<BorrowedFd<'static>> {
        let fd = match output {
            LogOutput::Terminal if self.stdout => libc::STDOUT_FILENO,
            LogOutput::Stderr if self.stderr => libc::STDERR_FILENO,
            _ => return None,
        };
        // SAFETY: std treats stdout and stderr as open for the life of the process. Borrowing
        // rather than owning them means the logging thread never closes them when it exits, which
        // would let the next file or socket opened take their numbers.
        Some(unsafe { BorrowedFd::borrow_raw(fd) })
    }
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
        && journal_stream == format!("{}:{}", metadata.st_dev, metadata.st_ino).as_str()
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

/// Writes each log record to the journal stream without hanging indefinitely on a stalled
/// journald, and drops records while the stream is stalled.
///
/// Records are buffered until the formatter flushes at the end of each one, then sent with
/// `MSG_DONTWAIT`. On `WouldBlock`, the writer waits up to `stall_timeout` for the socket to
/// become writable so transient bursts and records larger than the socket buffer complete intact.
/// If the wait times out, the stream is marked stalled and subsequent records drop immediately
/// without polling until the socket accepts bytes again. A record that only partly fits is cut
/// short, and the next record that gets through starts with a newline, so journald doesn't join
/// the two into one entry.
pub(super) struct JournalWriter<W> {
    out: W,
    /// A newline, then the record being formatted. The newline is sent only to end a record
    /// that was cut short.
    buf: Vec<u8>,
    cut_short: bool,
    stalled: bool,
    stall_timeout: Duration,
    new_record: Arc<AtomicBool>,
}

impl<W: AsFd> JournalWriter<W> {
    /// Builds the formatting drain for records to `out`, and tells its writer where each
    /// record starts, including after a record that failed to format.
    pub(super) fn drain<D>(out: W, build: impl FnOnce(Self) -> D) -> JournalDrain<D> {
        Self::drain_with_timeout(out, STALL_WAIT_TIMEOUT, build)
    }

    fn drain_with_timeout<D>(
        out: W,
        stall_timeout: Duration,
        build: impl FnOnce(Self) -> D,
    ) -> JournalDrain<D> {
        metrics::register();
        let new_record = Arc::new(AtomicBool::new(true));
        let mut buf = Vec::with_capacity(INITIAL_RECORD_CAPACITY);
        buf.push(b'\n');
        let writer = Self {
            out,
            buf,
            cut_short: false,
            stalled: false,
            stall_timeout,
            new_record: Arc::clone(&new_record),
        };

        JournalDrain {
            inner: build(writer),
            new_record,
            _not_sync: PhantomData,
        }
    }

    fn clear(&mut self) {
        self.buf.truncate(1);
        self.buf.shrink_to(MAX_RETAINED_RECORD_CAPACITY);
    }

    #[cold]
    fn start_new_record(&mut self) {
        self.new_record.store(false, Ordering::Relaxed);
        // The JSON formatter doesn't flush a record it fails to finish, so discard what it
        // wrote instead of sending it with the next record. The text formatter flushes such
        // a record anyway; it goes out cut short, and the next record starts a new line.
        self.clear();
    }

    /// Sends the buffered record, waiting briefly on transient backpressure and dropping
    /// immediately once stalled, and counts any drop.
    fn send(&mut self) -> io::Result<()> {
        let start = usize::from(!self.cut_short);
        let record = &self.buf[start..];
        let mut sent = 0;
        let mut wait_deadline: Option<Instant> = None;

        let res = loop {
            if sent == record.len() {
                break Ok(());
            }
            match send_once(self.out.as_fd(), &record[sent..]) {
                Ok(SendOutcome::Sent(0)) => break Err(io::ErrorKind::WriteZero.into()),
                Ok(SendOutcome::Sent(n)) => {
                    sent += n;
                    self.stalled = false;
                    wait_deadline = None;
                }
                Ok(SendOutcome::NotSocket) => {
                    // Something replaced stdout or stderr with a regular file or pipe after init.
                    // If we haven't sent any bytes of this record yet, don't prepend a stray
                    // newline left over from an earlier cut-short journal record.
                    let fallback_buf = if sent == 0 {
                        self.cut_short = false;
                        self.stalled = false;
                        &self.buf[1..]
                    } else {
                        &record[sent..]
                    };
                    let (written, err) = write_all_raw(self.out.as_fd(), fallback_buf);
                    if let Some(&last) = fallback_buf[..written].last() {
                        self.cut_short = last != b'\n';
                    }
                    return match err {
                        None => Ok(()),
                        Some(err) => Err(err),
                    };
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if self.stalled {
                        break Err(err);
                    }
                    let deadline =
                        *wait_deadline.get_or_insert_with(|| Instant::now() + self.stall_timeout);
                    match wait_writable_until(self.out.as_fd(), deadline) {
                        Ok(true) => {}
                        Ok(false) => {
                            self.stalled = true;
                            break Err(err);
                        }
                        Err(poll_err) => break Err(poll_err),
                    }
                }
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

enum SendOutcome {
    Sent(usize),
    NotSocket,
}

/// Sends without waiting, whatever the descriptor's own blocking mode is.
fn send_once(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<SendOutcome> {
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
        return Ok(SendOutcome::Sent(sent as usize));
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ENOTSOCK) {
        return Ok(SendOutcome::NotSocket);
    }
    Err(err)
}

/// Writes `buf` to a non-socket descriptor, returning the number of bytes written and any error.
fn write_all_raw(fd: BorrowedFd<'_>, buf: &[u8]) -> (usize, Option<io::Error>) {
    let mut written = 0;
    while written < buf.len() {
        let slice = &buf[written..];
        // SAFETY: `slice` is valid for `slice.len()` bytes, and write does not take ownership of `fd`.
        let n = unsafe { libc::write(fd.as_raw_fd(), slice.as_ptr().cast(), slice.len()) };
        if n > 0 {
            written += n as usize;
        } else if n == 0 {
            return (written, Some(io::ErrorKind::WriteZero.into()));
        } else {
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return (written, Some(err));
            }
        }
    }
    (written, None)
}

/// Waits until `deadline` for `fd` to become writable (or hit an error/hangup condition).
///
/// Returns `Ok(true)` if the caller should retry sending, or `Ok(false)` if `deadline` elapsed.
fn wait_writable_until(fd: BorrowedFd<'_>, deadline: Instant) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        // Round sub-millisecond remainders up to 1ms so we don't busy-spin on EINTR.
        let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as libc::c_int;
        pfd.revents = 0;

        // SAFETY: `pfd` points to a valid `pollfd` of length 1, and poll does not close `fd`.
        let ready = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ready > 0 {
            return Ok(true);
        }
        if ready == 0 {
            return Ok(false);
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

impl<W: AsFd> io::Write for JournalWriter<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Only the logging thread touches this flag (JournalDrain is !Sync): the drain sets it
        // before formatting a record, and the first write of the record clears it. So nothing can
        // change it between the load and the store, and the load keeps every later write to a
        // plain read, where a swap would be a read-modify-write each time.
        if self.new_record.load(Ordering::Relaxed) {
            self.start_new_record();
        }
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.new_record.load(Ordering::Relaxed) {
            self.start_new_record();
        }
        self.buf.extend_from_slice(buf);
        Ok(())
    }

    /// Sends the buffered record. The log formatters flush at the end of each record.
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        if self.new_record.load(Ordering::Relaxed) {
            self.start_new_record();
            return Ok(());
        }
        if self.buf.len() == 1 {
            return Ok(());
        }
        let res = self.send();
        self.clear();
        res
    }
}

/// Marks the start of each record for its [`JournalWriter`].
///
/// Intentionally `!Sync` so the shared `new_record` flag can only ever be accessed from a single
/// thread (such as the `slog-async` worker thread).
pub(super) struct JournalDrain<D> {
    inner: D,
    new_record: Arc<AtomicBool>,
    _not_sync: PhantomData<Cell<()>>,
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

    #[inline]
    fn is_enabled(&self, level: slog::Level) -> bool {
        self.inner.is_enabled(level)
    }

    #[inline]
    fn flush(&self) -> Result<(), slog::FlushError> {
        self.inner.flush()
    }
}

/// Why a log record was dropped instead of written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DropReason {
    /// The journal stream stayed full past the stall wait timeout and had no room for the record.
    JournalFull,
    /// Sending to the journal stream failed, for example because journald closed it. Unless
    /// `ignore_io_errors` is set, the logging thread stops after the first one.
    JournalError,
}

#[cfg(feature = "metrics")]
mod metrics {
    use super::DropReason;
    use crate::telemetry::metrics::Counter;
    use std::sync::LazyLock;

    #[crate::telemetry::metrics::metrics(crate_path = "crate", unprefixed)]
    pub(crate) mod logging {
        /// Number of log records dropped instead of written to the systemd journal stream.
        /// Unless `ignore_io_errors` is set, logging stops after the first `journal_error`.
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

    fn stream_id(fd: &impl AsFd) -> String {
        let metadata = std::fs::File::from(fd.as_fd().try_clone_to_owned().unwrap())
            .metadata()
            .unwrap();
        format!("{}:{}", metadata.dev(), metadata.ino())
    }

    /// A writer to a blocking journal stream, and journald's end of it.
    fn journal_pair() -> (JournalWriter<UnixStream>, UnixStream) {
        let (stream, journald) = UnixStream::pair().unwrap();
        let drain = JournalWriter::drain_with_timeout(stream, Duration::from_millis(20), |w| w);
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

    /// Writes and flushes `record` on another thread, failing the test if that waits too long.
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
    fn journal_streams_fd_only_matches_detected_stdio_variants() {
        let streams = JournalStreams {
            stdout: true,
            stderr: false,
        };
        assert_eq!(
            streams.fd(&LogOutput::Terminal).map(|fd| fd.as_raw_fd()),
            Some(libc::STDOUT_FILENO)
        );
        assert!(streams.fd(&LogOutput::Stderr).is_none());
        assert!(streams.fd(&LogOutput::File("/tmp/log".into())).is_none());
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
    fn drops_records_that_do_not_fit_and_does_not_re_wait_while_stalled() {
        let (writer, mut journald) = journal_pair();
        fill(&writer);

        // First record times out after `stall_timeout` and marks the stream stalled.
        let (writer, res) = log_without_waiting(writer, b"dropped record 1\n");
        res.unwrap();
        assert!(writer.stalled);

        // Subsequent records while stalled drop immediately without waiting `stall_timeout` each.
        let start = Instant::now();
        let mut writer = writer;
        for _ in 0..100 {
            let (next, res) = log_without_waiting(writer, b"dropped while stalled\n");
            res.unwrap();
            writer = next;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "stalled drops waited per record: {:?}",
            start.elapsed()
        );

        let filled = read_available(&mut journald);
        assert!(filled.iter().all(|&b| b == b'\n'));

        let (writer, res) = log_without_waiting(writer, b"next record\n");
        res.unwrap();
        assert!(!writer.stalled);
        assert_eq!(read_available(&mut journald), b"next record\n");
    }

    #[test]
    fn ends_a_record_cut_short_before_the_next_one() {
        let (writer, mut journald) = journal_pair();
        // Shrink SO_SNDBUF so a modest record exceeds it cleanly without allocating 32 MiB.
        let sndbuf: libc::c_int = 16 * 1024;
        // SAFETY: valid sockopt pointer and length on a borrowed socket fd.
        unsafe {
            libc::setsockopt(
                writer.out.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&raw const sndbuf).cast(),
                std::mem::size_of_val(&sndbuf) as libc::socklen_t,
            );
        }
        let big = format!("{}\n", "x".repeat(2 * 1024 * 1024));

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
    fn keeps_the_buffer_of_a_normal_record_and_frees_a_huge_one() {
        let (writer, _journald) = journal_pair();

        let (writer, res) = log_without_waiting(writer, &[b'x'; 8 * 1024]);
        res.unwrap();
        let capacity = writer.buf.capacity();
        assert!(capacity > 8 * 1024, "{capacity}");

        let (writer, res) = log_without_waiting(writer, &vec![b'x'; 1024 * 1024]);
        res.unwrap();
        let capacity = writer.buf.capacity();
        assert!(capacity <= 64 * 1024, "{capacity}");
    }

    #[test]
    fn reports_a_closed_journal_as_an_error() {
        let (writer, journald) = journal_pair();
        drop(journald);

        let (_writer, res) = log_without_waiting(writer, b"record\n");
        assert_eq!(res.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn flush_after_new_record_without_write_discards_stale_bytes() {
        let (mut writer, mut journald) = journal_pair();

        writer.new_record.store(true, Ordering::Relaxed);
        writer.write_all(b"partial stale record").unwrap();

        writer.new_record.store(true, Ordering::Relaxed);
        writer.flush().unwrap();

        assert_eq!(read_available(&mut journald), b"");
    }
}
