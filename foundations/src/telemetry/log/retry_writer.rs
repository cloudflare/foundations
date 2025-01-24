use std::fs::{File, OpenOptions};
use std::io;
use std::io::Write;
use std::path::PathBuf;

/// A writer which retries if an [`io::ErrorKind::BrokenPipe`] is returned.
///
/// Gives up and return an error after the max number of attempts or if any other errors are encountered.
///
/// Intended for use when writing to a pipe (fifo) with a reader which may randomly close and then
/// reopen read side.
///
/// *WARNING*: This may cause duplication of data as it's not possible to know what bytes
/// where actually processed by the read side.
pub(crate) struct RetryPipeWriter {
    path: PathBuf,
    pipe_file: Box<File>,
    max_attempts: i32,
}

impl RetryPipeWriter {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let file = Self::open_file(&path)?;
        Ok(Self {
            path,
            pipe_file: file,
            // This number was selected by casually observing unit test failures.
            // It's assumed that this simple approach will cover most cases.
            // Further observation may show the need to either make this configurable
            // or use an entirely differeing retry strategy.
            max_attempts: 10,
        })
    }

    fn open_file(path: &PathBuf) -> io::Result<Box<File>> {
        Ok(Box::new(OpenOptions::new().write(true).open(path)?))
    }

    fn reopen_file(&mut self) -> Result<(), io::Error> {
        let _ = std::mem::replace(&mut self.pipe_file, Self::open_file(&self.path)?);
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
        Err(io::Error::new(
            io::ErrorKind::Other,
            "retry attempts exhausted",
        ))
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
    fn test_retry_file() {
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
