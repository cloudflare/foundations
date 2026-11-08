#![cfg(all(target_os = "linux", feature = "telemetry"))]
//! These tests replace stdout and stderr and initialize telemetry, so they assume a separate
//! process is used for each test. Make sure you run with `cargo nextest run`.

mod common;

use common::collect_text;
use foundations::service_info;
use foundations::telemetry::settings::{LogFormat, LogOutput, LoggingSettings, TelemetrySettings};
use foundations::telemetry::{self, TelemetryConfig, TelemetryDriver};
use std::io;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn init_logging(output: LogOutput, format: LogFormat) -> TelemetryDriver {
    init_logging_with_settings(LoggingSettings {
        output,
        format,
        ..Default::default()
    })
}

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

fn stream_id(fd: BorrowedFd<'_>) -> String {
    let metadata = std::fs::File::from(fd.try_clone_to_owned().unwrap())
        .metadata()
        .unwrap();
    format!("{}:{}", metadata.dev(), metadata.ino())
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
            panic!("the logging thread did not drop records for {reason}");
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
async fn stalled_journal_does_not_hang_logging_panics_or_shutdown() {
    let (stream, mut journald) = UnixStream::pair().unwrap();
    let original_stderr = pose_as_journal(&stream);
    let mut driver = init_logging(LogOutput::Stderr, LogFormat::Json);
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
async fn logging_does_not_wait_for_a_blocking_journal_stream() {
    let (stream, _journald) = UnixStream::pair().unwrap();
    let original_stderr = pose_as_journal(&stream);
    let mut driver = init_logging_with_settings(LoggingSettings {
        output: LogOutput::Stderr,
        format: LogFormat::Json,
        allow_blocking_journal_stdio: true,
        ..Default::default()
    });
    let nonblocking = is_nonblocking(io::stderr().as_fd());
    fill_stream(&stream);

    telemetry::log::warn!("written while the stream blocks");
    wait_for_drops("journal_full", 1, &original_stderr);
    let shutdown_finished = finishes_in_time(move || driver.shutdown_logger());
    nix::unistd::dup2_stderr(&original_stderr).unwrap();

    assert!(!nonblocking, "the setting didn't leave the stream alone");
    assert!(shutdown_finished, "logger shutdown waited for the journal");
}

#[tokio::test]
async fn records_dropped_after_journald_closes_the_stream_are_counted() {
    let (stream, journald) = UnixStream::pair().unwrap();
    let original_stderr = pose_as_journal(&stream);
    let mut driver = init_logging_with_settings(LoggingSettings {
        output: LogOutput::Stderr,
        format: LogFormat::Json,
        ignore_io_errors: true,
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
    let mut driver = init_logging(LogOutput::Stderr, LogFormat::Text);

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
    let mut driver = init_logging(LogOutput::Terminal, LogFormat::Json);

    telemetry::log::warn!("to stdout");
    driver.shutdown_logger();

    nix::unistd::dup2_stdout(&original_stdout).unwrap();
    nix::unistd::dup2_stderr(&original_stderr).unwrap();
    assert!(is_nonblocking(stream.as_fd()));
    let received = read_available(&mut journald);
    assert!(received.contains("\"msg\":\"to stdout\""), "{received}");
}

#[tokio::test]
async fn stderr_does_not_block_when_logs_go_to_a_file() {
    let (stream, _journald) = UnixStream::pair().unwrap();
    let original_stderr = pose_as_journal(&stream);
    let dir = tempfile::tempdir().unwrap();
    let _driver = init_logging(LogOutput::File(dir.path().join("log")), LogFormat::Json);

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
    let _driver = init_logging(LogOutput::Stderr, LogFormat::Json);

    let nonblocking = is_nonblocking(io::stderr().as_fd());
    nix::unistd::dup2_stderr(&original_stderr).unwrap();
    assert!(!nonblocking);
}

#[tokio::test]
async fn journal_output_leaves_stderr_open_after_shutdown() {
    let (stream, _journald) = UnixStream::pair().unwrap();
    let original_stderr = pose_as_journal(&stream);
    let mut driver = init_logging(LogOutput::Stderr, LogFormat::Json);

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
