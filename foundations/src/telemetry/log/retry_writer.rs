use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::{fs, io};

/// A writer which retries if an [`io::ErrorKind::BrokenPipe`] is returned.
///
/// Intended for use when writing to a pipe (fifo) with a reader which may randomly close and then
/// reopen read side.
///
/// *WARNING*: This may cause duplication of data as it's not possible to know what bytes
/// where actually processed by the read side.
///
/// Gives up and return an error after the max number of attempts or if any other errors are encountered.
pub(crate) struct RetryPipeWriter {
    path: PathBuf,
    pipe_file: Box<File>,
    max_attempts: i32,
}

impl RetryPipeWriter {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let meta = fs::metadata(&path)?;
        if !meta.file_type().is_fifo() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a fifo"));
        }
        let writer = Self::open_file(&path)?;
        Ok(Self {
            path,
            pipe_file: writer,
            max_attempts: 1,
        })
    }

    fn open_file(path: &PathBuf) -> io::Result<Box<File>> {
        Ok(Box::new(OpenOptions::new().write(true).open(path)?))
    }

    fn reopen_file(&mut self) -> Result<(), io::Error> {
        let new_writer = Self::open_file(&self.path)?;
        let _ = std::mem::replace(&mut self.pipe_file, new_writer);
        Ok(())
    }
}

impl Write for RetryPipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut attempts = 0;
        // TODO: Need to understand if this is an appropriate retry strategy
        // we may actually want this to be unlimited with a backoff. But if we assume
        // that an open blocks until both sides are open then a backoff doesn't do anything.
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
    use nanoid::nanoid;
    use nix::sys::stat;
    use nix::unistd;
    use std::fs::OpenOptions;
    use std::io::{BufWriter, Read, Write};
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_retry_file() {
        let tmp_dir = tempdir().unwrap();
        let fifo_path = tmp_dir.path().join(format!("{}.pipe", nanoid!()));
        unistd::mkfifo(&fifo_path, stat::Mode::S_IRWXU).unwrap();

        let path_copy = fifo_path.clone();
        // reader should get 10 bytes
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
                i += 1;
            }
        });

        let mut retrying_file =
            BufWriter::with_capacity(2, RetryPipeWriter::new(fifo_path.clone()).unwrap());
        let mut i = 0;
        // writer should send 10 bytes
        while i < 10 {
            let _ = retrying_file.write(format!("{i}").as_bytes()).unwrap();
            retrying_file.flush().unwrap();
            i += 1;
        }
        // and then the reader thread should exit and complete the test
        // if not then this test hangs and the runner can complain about it.
        handler.join().unwrap();
    }
}
