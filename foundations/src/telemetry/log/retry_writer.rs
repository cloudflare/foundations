use std::fs::File;
use std::io;
use std::io::Write;
use std::path::PathBuf;

/// A writer which retries if an [`io::ErrorKind::BrokenPipe`] is returned.
///
/// Intended for use when writing to a pipe (fifo) with a reader which may randomly close and then
/// reopen read side it will give and return an error after the max number of attempts or if any
/// other type of error occures during reconnect or subsequent write operations.
///
/// Writes will be blocked while re-opening the pipe and once connected the next writes
/// may block until the reader has connected. Blocking is prefered over dropping since this will provide
/// backpressure to wrappers like [`slog_async::Async`] which by default will drop and report
/// records once the internal channel is overflowed. Dropped record reports may also be dropped if
/// the reader does not recover in a timely manner.
pub(crate) struct RetryPipeWriter {
    path: PathBuf,
    pipe_file: File,
    max_attempts: i32,
}

impl RetryPipeWriter {
    pub(super) fn new(path: PathBuf) -> io::Result<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            path,
            pipe_file: file,
            // This number was selected by casually observing unit test failures.
            // It's assumed that this simple approach will cover most cases but
            // further usage and observation may show the need to either make this configurable
            // or use an entirely different retry strategy.
            max_attempts: 10,
        })
    }

    fn reopen_file(&mut self) -> Result<(), io::Error> {
        let _ = std::mem::replace(&mut self.pipe_file, File::create(&self.path)?);
        Ok(())
    }
}

impl Write for RetryPipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut attempts = 0;
        while attempts <= self.max_attempts {
            let result = self.pipe_file.write(buf);
            match result {
                Ok(n) => return Ok(n),
                Err(err) => {
                    if err.kind() == io::ErrorKind::BrokenPipe {
                        // Reconnecting immediately assumes that
                        // the pipe still exists and that the open call
                        // will block until the other end is open.
                        self.reopen_file()?;
                    } else {
                        return Err(err);
                    }
                }
            }
            attempts += 1;
        }
        Err(io::Error::other("retry attempts exhausted"))
    }

    /// Flushes the file. On *nix this does nothing.
    fn flush(&mut self) -> io::Result<()> {
        self.pipe_file.flush()
    }
}

#[cfg(test)]
mod tests {
    use crate::telemetry::log::retry_writer::RetryPipeWriter;
    use nix::sys::stat;
    use nix::unistd;
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Write};
    use std::thread;
    use tempfile::NamedTempFile;

    #[test]
    fn test_regular_file() {
        let tmp_path = NamedTempFile::new().unwrap().into_temp_path();
        let file_path = tmp_path.to_path_buf();
        tmp_path.close().unwrap();
        // The tmpfile should now be gone and
        // the retry writer can create it.
        assert!(!file_path.exists());
        const TEST_MSG: &[u8] = "test log message".as_bytes();
        {
            let mut retrying_file = RetryPipeWriter::new(file_path.clone()).unwrap();
            let _ = retrying_file.write(TEST_MSG).unwrap();
        }
        let mut reader = OpenOptions::new()
            .read(true)
            .open(file_path.clone())
            .unwrap();
        let mut buffer = [0; TEST_MSG.len()];
        let _ = reader.read(&mut buffer[..]).unwrap();
        assert_eq!(TEST_MSG, buffer);
    }

    #[test]
    fn test_retry_pipe() {
        let tmp_path = NamedTempFile::new().unwrap().into_temp_path();
        let fifo_path = tmp_path.to_path_buf();
        tmp_path.close().unwrap();

        unistd::mkfifo(&fifo_path, stat::Mode::S_IRWXU).unwrap();

        let path_copy = fifo_path.clone();
        // This reader thread should read 1 byte 10 times,
        // the pipe is closed and re-opened after each read.
        let handler = thread::spawn(move || {
            let mut buffer = [0; 1];
            let mut i = 0;
            while i < 10 {
                {
                    let mut reader = OpenOptions::new()
                        .read(true)
                        .open(path_copy.clone())
                        .unwrap();
                    let _ = reader.read(&mut buffer[..]).unwrap();
                }
                i += 1
            }
        });

        let mut retrying_file = RetryPipeWriter::new(fifo_path.clone()).unwrap();
        let mut i = 0;
        // Send 1 bytes 10 times, unwrap the result to surface any errors not handled by the retry.
        while i < 10 {
            let _ = retrying_file.write(format!("{i}").as_bytes()).unwrap();
            i += 1;
        }
        // If all goes to plan, the reader thread should see all 10 bytes, exit, and complete the test.
        // If not then this test hangs and the runner can complain about it.
        handler.join().unwrap();
        fs::remove_file(fifo_path).unwrap();
    }
}
