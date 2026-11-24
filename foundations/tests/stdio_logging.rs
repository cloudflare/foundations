#![cfg(all(unix, feature = "telemetry"))]
//! These tests replace stdout and stderr and initialize telemetry, so they assume a separate
//! process is used for each test. Make sure you run with `cargo nextest run`.

#[cfg(target_os = "linux")]
mod common;

use foundations::service_info;
use foundations::telemetry::settings::{LogFormat, LogOutput, LoggingSettings, TelemetrySettings};
use foundations::telemetry::{self, TelemetryConfig, TelemetryDriver};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;

fn init_logging_with_settings(logging: LoggingSettings) -> TelemetryDriver {
    let settings = TelemetrySettings {
        logging,
        ..Default::default()
    };

    telemetry::init(TelemetryConfig {
        service_info: &service_info!(),
        settings: &settings,
        custom_server_routes: Default::default(),
    })
    .unwrap()
}

fn stream_id(fd: BorrowedFd<'_>) -> String {
    let metadata = std::fs::File::from(fd.try_clone_to_owned().unwrap())
        .metadata()
        .unwrap();
    format!("{}:{}", metadata.dev(), metadata.ino())
}

#[tokio::test]
async fn stderr_output_leaves_stderr_open_after_shutdown() {
    let (stream, _reader) = UnixStream::pair().unwrap();
    let original_stderr = io::stderr().as_fd().try_clone_to_owned().unwrap();
    nix::unistd::dup2_stderr(&stream).unwrap();
    let mut driver = init_logging_with_settings(LoggingSettings {
        output: LogOutput::Stderr,
        format: LogFormat::Json,
        ..Default::default()
    });

    telemetry::log::warn!("logged before shutdown");
    driver.shutdown_logger();
    let stderr = io::stderr().as_fd().try_clone_to_owned();

    nix::unistd::dup2_stderr(&original_stderr).unwrap();
    let stderr = stderr.expect("shutting down the logger closed stderr");
    assert_eq!(stream_id(stderr.as_fd()), stream_id(stream.as_fd()));
}

#[tokio::test]
async fn terminal_output_leaves_stdout_open_after_shutdown() {
    let (stream, _reader) = UnixStream::pair().unwrap();
    let original_stdout = io::stdout().as_fd().try_clone_to_owned().unwrap();
    nix::unistd::dup2_stdout(&stream).unwrap();
    let mut driver = init_logging_with_settings(LoggingSettings {
        output: LogOutput::Terminal,
        format: LogFormat::Json,
        ..Default::default()
    });

    telemetry::log::warn!("logged before shutdown");
    driver.shutdown_logger();
    let stdout = io::stdout().as_fd().try_clone_to_owned();

    nix::unistd::dup2_stdout(&original_stdout).unwrap();
    let stdout = stdout.expect("shutting down the logger closed stdout");
    assert_eq!(stream_id(stdout.as_fd()), stream_id(stream.as_fd()));
}

#[cfg(target_os = "linux")]
mod linux_journal {
    use super::*;
    use crate::common::collect_text;
    use foundations::telemetry::settings::JournalStreamBehavior;
    use std::io::{Read, Seek, Write};
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn init_journal_logging(
        output: LogOutput,
        format: LogFormat,
        journal_stream_behavior: JournalStreamBehavior,
    ) -> TelemetryDriver {
        init_logging_with_settings(LoggingSettings {
            output,
            format,
            journal_stream_behavior,
            ..Default::default()
        })
    }

    /// Puts `stream` on stderr and names it as the journal stream, as systemd does.
    ///
    /// Returns the original stderr.
    fn pose_as_journal(stream: &UnixStream) -> OwnedFd {
        let original = io::stderr().as_fd().try_clone_to_owned().unwrap();
        nix::unistd::dup2_stderr(stream).unwrap();
        // SAFETY: nothing else in this test process reads or writes the environment meanwhile.
        unsafe { std::env::set_var("JOURNAL_STREAM", stream_id(stream.as_fd())) };

        original
    }

    fn is_nonblocking(fd: BorrowedFd<'_>) -> bool {
        // SAFETY: F_GETFL only reads the borrowed descriptor's flags.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(flags, -1);
        flags & libc::O_NONBLOCK != 0
    }

    /// Fills `stream` so that nothing more fits, leaving its blocking mode as it was.
    fn fill_stream(stream: &UnixStream) {
        let nonblocking = is_nonblocking(stream.as_fd());
        stream.set_nonblocking(true).unwrap();
        loop {
            match (&*stream).write(&[b'\n'; 8192]) {
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => panic!("filling the journal stream: {err}"),
            }
        }
        stream.set_nonblocking(nonblocking).unwrap();
    }

    fn read_available(journald: &mut UnixStream) -> String {
        journald.set_nonblocking(true).unwrap();
        let mut received = Vec::new();
        let mut buf = [0; 64 * 1024];
        loop {
            match journald.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buf[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => panic!("read failed: {err}"),
            }
        }
        String::from_utf8(received).unwrap()
    }

    /// Spawns a thread that drains `journald` continuously after an optional initial delay.
    fn spawn_journal_reader(
        mut journald: UnixStream,
        initial_delay: Duration,
    ) -> (mpsc::Sender<()>, std::thread::JoinHandle<String>) {
        let (stop_tx, stop_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            if !initial_delay.is_zero() {
                std::thread::sleep(initial_delay);
            }
            journald
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            let mut received = Vec::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                match journald.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        if stop_rx.try_recv().is_ok() {
                            received.extend_from_slice(read_available(&mut journald).as_bytes());
                            break;
                        }
                    }
                    Err(err) => panic!("reader failed: {err}"),
                }
            }
            String::from_utf8(received).unwrap()
        });
        (stop_tx, handle)
    }

    fn dropped(reason: &str) -> Option<u64> {
        let series = format!("logging_records_dropped{{reason=\"{reason}\"}} ");
        collect_text(&Default::default())
            .lines()
            .find_map(|line| line.strip_prefix(&series)?.parse().ok())
    }

    /// Waits until at least `count` records were dropped for `reason`.
    fn wait_for_drops(reason: &str, count: u64, original_stderr: &OwnedFd) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while dropped(reason) < Some(count) {
            if Instant::now() >= deadline {
                nix::unistd::dup2_stderr(original_stderr).unwrap();
                fail_without_panicking(&format!(
                    "the logging thread did not drop records for {reason}"
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn finishes_in_time(f: impl FnOnce() + Send + 'static) -> bool {
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            f();
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(Duration::from_secs(10)).is_ok()
    }

    /// Fails the test without going through the panic hook.
    ///
    /// A panic stuck writing to the journal holds the default hook's backtrace lock, so the
    /// panic of a failed assertion would wait for that lock forever instead of failing the test.
    fn fail_without_panicking(message: &str) -> ! {
        let _ = nix::unistd::write(io::stderr(), format!("{message}\n").as_bytes());
        std::process::exit(101);
    }

    #[tokio::test]
    async fn default_blocking_behavior_leaves_journal_stream_untouched() {
        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);

        let mut driver = init_logging_with_settings(LoggingSettings {
            output: LogOutput::Stderr,
            format: LogFormat::Json,
            ..Default::default()
        });

        let nonblocking = is_nonblocking(io::stderr().as_fd());
        telemetry::log::warn!("default blocking record");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        assert!(!nonblocking, "default behavior must not set O_NONBLOCK");
        assert_eq!(
            dropped("journal_full"),
            None,
            "default behavior must not activate JournalWriter"
        );
        let received = read_available(&mut journald);
        let record: serde_json::Value = serde_json::from_str(received.trim()).unwrap();
        assert_eq!(record["msg"], "default blocking record");
    }

    #[cfg(feature = "security")]
    #[tokio::test]
    async fn default_blocking_behavior_does_not_require_sendto_under_seccomp() {
        use foundations::security::{Rule, Syscall, ViolationAction, enable_syscall_sandboxing};

        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);

        // Allow everything except `sendto`, which returns EPERM. Default logging must still
        // succeed via `write()` without touching `sendto`.
        let rules = vec![Rule::ReturnError(
            Syscall::sendto,
            libc::EPERM as u16,
            vec![],
        )];
        enable_syscall_sandboxing(ViolationAction::AllowAndLog, &rules).unwrap();

        let mut driver = init_logging_with_settings(LoggingSettings {
            output: LogOutput::Stderr,
            format: LogFormat::Json,
            ..Default::default()
        });

        telemetry::log::warn!("seccomp default ok");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        let received = read_available(&mut journald);
        let record: serde_json::Value = serde_json::from_str(received.trim()).unwrap();
        assert_eq!(record["msg"], "seccomp default ok");
    }

    #[tokio::test]
    async fn stalled_journal_does_not_hang_logging_panics_or_shutdown() {
        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );
        let nonblocking = is_nonblocking(io::stderr().as_fd());

        // journald isn't reading, and the stream is full.
        fill_stream(&stream);
        for _ in 0..3 {
            telemetry::log::warn!("dropped record");
        }
        wait_for_drops("journal_full", 3, &original_stderr);
        let panic_finished = finishes_in_time(|| {
            let _ = std::thread::spawn(|| panic!("panic while the journal is stalled")).join();
        });
        // Once journald reads again, the logging thread writes the next record.
        read_available(&mut journald);
        telemetry::log::warn!("after the stall");
        let shutdown_finished = finishes_in_time(move || driver.shutdown_logger());

        // Restore stderr, so a failure can report itself.
        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        if !nonblocking {
            fail_without_panicking("the journal stream is still blocking");
        }
        if !panic_finished {
            fail_without_panicking("the panic waited for the journal");
        }
        if !shutdown_finished {
            fail_without_panicking("logger shutdown waited for the journal");
        }

        let received = read_available(&mut journald);
        let last = received.lines().last().unwrap_or_default();
        let record: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(record["msg"], "after the stall", "{received}");
    }

    #[tokio::test]
    async fn stalled_journal_text_format_does_not_hang_logging_or_shutdown() {
        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Text,
            JournalStreamBehavior::NonBlockingStdio,
        );

        fill_stream(&stream);
        for _ in 0..3 {
            telemetry::log::warn!("dropped text record");
        }
        wait_for_drops("journal_full", 3, &original_stderr);
        read_available(&mut journald);
        telemetry::log::warn!("text after the stall");
        let shutdown_finished = finishes_in_time(move || driver.shutdown_logger());

        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        if !shutdown_finished {
            fail_without_panicking("logger shutdown waited for the journal");
        }

        let received = read_available(&mut journald);
        let last = received.lines().last().unwrap_or_default();
        assert!(last.contains("text after the stall"), "{received}");
    }

    #[tokio::test]
    async fn nonblocking_logger_leaves_stdio_blocking_and_does_not_hang_shutdown() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingLogger,
        );
        let nonblocking = is_nonblocking(io::stderr().as_fd());
        fill_stream(&stream);

        telemetry::log::warn!("written while the stream blocks");
        wait_for_drops("journal_full", 1, &original_stderr);
        let shutdown_finished = finishes_in_time(move || driver.shutdown_logger());
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        // The stream blocks here, so a panic on the logging thread can be stuck in the panic hook.
        if nonblocking {
            fail_without_panicking("NonBlockingLogger must leave the stream's O_NONBLOCK alone");
        }
        if !shutdown_finished {
            fail_without_panicking("logger shutdown waited for the journal");
        }
    }

    #[tokio::test]
    async fn large_json_record_is_not_truncated_when_journald_is_reading() {
        let (stream, journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingLogger,
        );
        let (stop_tx, reader) = spawn_journal_reader(journald, Duration::ZERO);

        // Larger than typical Linux AF_UNIX SO_SNDBUF (~208 KiB).
        let payload = "a".repeat(1024 * 1024);
        telemetry::log::warn!("large record"; "payload" => payload.as_str());
        driver.shutdown_logger();
        let _ = stop_tx.send(());
        let received = reader.join().unwrap();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        assert_eq!(dropped("journal_full"), Some(0));
        let record: serde_json::Value = serde_json::from_str(received.trim())
            .expect("large JSON record must not be truncated on healthy journald");
        assert_eq!(record["msg"], "large record");
        assert_eq!(record["payload"].as_str().unwrap().len(), payload.len());
    }

    #[tokio::test]
    async fn transient_journald_pause_does_not_drop_burst_records() {
        let (stream, journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingLogger,
        );
        // Simulate a brief 15ms journald pause, well under the 50ms stall wait timeout.
        let (stop_tx, reader) = spawn_journal_reader(journald, Duration::from_millis(15));

        let pad = "x".repeat(1024);
        for i in 0..400 {
            telemetry::log::warn!("burst"; "seq" => i, "pad" => pad.as_str());
        }
        driver.shutdown_logger();
        let _ = stop_tx.send(());
        let received = reader.join().unwrap();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        assert_eq!(dropped("journal_full"), Some(0));
        let lines: Vec<_> = received.lines().collect();
        assert_eq!(lines.len(), 400);
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).expect("corrupt JSON line");
        }
    }

    #[tokio::test]
    async fn records_dropped_after_journald_closes_the_stream_are_counted() {
        let (stream, journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_logging_with_settings(LoggingSettings {
            output: LogOutput::Stderr,
            format: LogFormat::Json,
            ignore_io_errors: true,
            journal_stream_behavior: JournalStreamBehavior::NonBlockingStdio,
            ..Default::default()
        });
        assert_eq!(dropped("journal_error"), Some(0));
        drop(journald);

        telemetry::log::warn!("first record after journald closed the stream");
        telemetry::log::warn!("second record after journald closed the stream");
        wait_for_drops("journal_error", 2, &original_stderr);
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();
    }

    #[tokio::test]
    async fn journal_error_stops_logging_after_first_record_when_not_ignoring_io_errors() {
        let (stream, journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );
        drop(journald);

        telemetry::log::warn!("first error");
        telemetry::log::warn!("second error");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        assert_eq!(dropped("journal_error"), Some(1));
    }

    #[tokio::test]
    async fn failed_json_records_are_discarded_before_the_next_record() {
        struct FailingContext {
            fail: Arc<AtomicBool>,
            failures: Arc<AtomicUsize>,
        }

        impl slog::Value for FailingContext {
            fn serialize(
                &self,
                _record: &slog::Record,
                key: slog::Key,
                serializer: &mut dyn slog::Serializer,
            ) -> slog::Result {
                serializer.emit_str(key, &"x".repeat(256))?;
                if self.fail.load(Ordering::Relaxed) {
                    self.failures.fetch_add(1, Ordering::Relaxed);
                    return Err(io::Error::other("context serialization failed").into());
                }
                Ok(())
            }
        }

        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_logging_with_settings(LoggingSettings {
            output: LogOutput::Stderr,
            format: LogFormat::Json,
            ignore_io_errors: true,
            journal_stream_behavior: JournalStreamBehavior::NonBlockingStdio,
            ..Default::default()
        });
        let fail = Arc::new(AtomicBool::new(true));
        let failures = Arc::new(AtomicUsize::new(0));
        telemetry::log::add_fields!("context" => FailingContext {
            fail: Arc::clone(&fail),
            failures: Arc::clone(&failures),
        });
        for _ in 0..128 {
            telemetry::log::warn!("failed record");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while failures.load(Ordering::Relaxed) < 128 {
            if Instant::now() >= deadline {
                nix::unistd::dup2_stderr(&original_stderr).unwrap();
                panic!("the logging thread did not process the failed records");
            }
            std::thread::yield_now();
        }
        fail.store(false, Ordering::Relaxed);
        telemetry::log::warn!("successful record");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        let received = read_available(&mut journald);
        let record: serde_json::Value = serde_json::from_str(&received).unwrap();
        assert_eq!(record["msg"], "successful record");
        assert_eq!(received.lines().count(), 1, "{received}");
    }

    #[tokio::test]
    async fn text_records_reach_the_journal_one_per_line() {
        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Text,
            JournalStreamBehavior::NonBlockingStdio,
        );

        telemetry::log::warn!("first record");
        telemetry::log::warn!("second record");
        driver.shutdown_logger();

        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        let received = read_available(&mut journald);
        let lines: Vec<_> = received.lines().collect();
        assert_eq!(lines.len(), 2, "{received}");
        assert!(lines[0].contains("first record"), "{received}");
        assert!(lines[1].contains("second record"), "{received}");
        // The drop counters exist from the start, so alerts can use them.
        assert_eq!(dropped("journal_full"), Some(0));
        assert_eq!(dropped("journal_error"), Some(0));
    }

    #[tokio::test]
    async fn terminal_output_to_the_journal_does_not_block() {
        let (stream, mut journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let original_stdout = io::stdout().as_fd().try_clone_to_owned().unwrap();
        nix::unistd::dup2_stdout(&stream).unwrap();
        let mut driver = init_journal_logging(
            LogOutput::Terminal,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );

        telemetry::log::warn!("to stdout");
        driver.shutdown_logger();

        nix::unistd::dup2_stdout(&original_stdout).unwrap();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        assert!(is_nonblocking(stream.as_fd()));
        let received = read_available(&mut journald);
        assert!(received.contains("\"msg\":\"to stdout\""), "{received}");
    }

    #[tokio::test]
    async fn stderr_does_not_block_when_logs_go_to_a_file_with_nonblocking_stdio() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let dir = tempfile::tempdir().unwrap();
        let _driver = init_journal_logging(
            LogOutput::File(dir.path().join("log")),
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );

        let nonblocking = is_nonblocking(io::stderr().as_fd());
        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        assert!(nonblocking);
    }

    #[tokio::test]
    async fn stderr_that_is_not_the_journal_is_left_alone() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let (other, _other_reader) = UnixStream::pair().unwrap();
        nix::unistd::dup2_stderr(&other).unwrap();
        let _driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );

        let nonblocking = is_nonblocking(io::stderr().as_fd());
        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        assert!(!nonblocking);
    }

    #[tokio::test]
    async fn logs_follow_stderr_when_something_replaces_the_journal() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );
        let mut file = tempfile::tempfile().unwrap();
        nix::unistd::dup2_stderr(&file).unwrap();

        telemetry::log::warn!("logged after stderr was replaced");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        let mut written = String::new();
        file.seek(io::SeekFrom::Start(0)).unwrap();
        file.read_to_string(&mut written).unwrap();
        assert!(
            written.contains("\"msg\":\"logged after stderr was replaced\""),
            "{written}"
        );
        assert_eq!(dropped("journal_error"), Some(0));
    }

    #[tokio::test]
    async fn replacing_journal_after_cut_short_record_does_not_prepend_newline() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );

        let big = "x".repeat(2 * 1024 * 1024);
        telemetry::log::warn!("cut short"; "big" => big.as_str());
        wait_for_drops("journal_full", 1, &original_stderr);

        let mut file = tempfile::tempfile().unwrap();
        nix::unistd::dup2_stderr(&file).unwrap();

        telemetry::log::warn!("first record in new file");
        driver.shutdown_logger();
        nix::unistd::dup2_stderr(&original_stderr).unwrap();

        let mut written = String::new();
        file.seek(io::SeekFrom::Start(0)).unwrap();
        file.read_to_string(&mut written).unwrap();
        assert!(
            written.starts_with('{'),
            "stray leading newline prepended to fresh file: {written:?}"
        );
        let record: serde_json::Value = serde_json::from_str(written.trim()).unwrap();
        assert_eq!(record["msg"], "first record in new file");
    }

    #[tokio::test]
    async fn journal_output_leaves_stderr_open_after_shutdown() {
        let (stream, _journald) = UnixStream::pair().unwrap();
        let original_stderr = pose_as_journal(&stream);
        let mut driver = init_journal_logging(
            LogOutput::Stderr,
            LogFormat::Json,
            JournalStreamBehavior::NonBlockingStdio,
        );

        telemetry::log::warn!("logged before shutdown");
        driver.shutdown_logger();
        let stderr = io::stderr().as_fd().try_clone_to_owned();

        nix::unistd::dup2_stderr(&original_stderr).unwrap();
        let stderr = stderr.expect("shutting down the logger closed stderr");
        assert_eq!(
            stream_id(stderr.as_fd()),
            stream_id(stream.as_fd()),
            "stderr no longer refers to the journal stream"
        );
    }
}
