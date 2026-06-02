use std::cell::Cell;
use std::io::{self, Write};

#[derive(Default, Clone, Copy)]
pub(crate) enum RewindTo {
    #[default]
    None,
    LastNewline,
}

pub(crate) struct RewindState {
    active: Cell<bool>,
    rewind_to: Cell<RewindTo>,
}

impl RewindState {
    pub(crate) const fn new() -> Self {
        Self {
            active: Cell::new(false),
            rewind_to: Cell::new(RewindTo::None),
        }
    }

    #[track_caller]
    pub(crate) fn activate<'a>(&'a self, buf: &'a mut Vec<u8>) -> RewindableWriter<'a> {
        let was_active = self.active.replace(true);
        assert!(!was_active, "this state already has an associated writer");

        self.rewind_to.set(RewindTo::None);
        RewindableWriter {
            out: buf,
            state: self,
        }
    }

    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        self.active.get()
    }

    #[inline]
    pub(crate) fn rewind_to(&self, to: RewindTo) {
        debug_assert!(self.is_active(), "rewind attempted without writer");
        self.rewind_to.set(to);
    }

    fn reset(&self) {
        self.active.set(false);
        self.rewind_to.set(RewindTo::None);
    }
}

pub(crate) struct RewindableWriter<'a> {
    out: &'a mut Vec<u8>,
    state: &'a RewindState,
}

impl<'a> RewindableWriter<'a> {
    fn apply_rewind(&mut self) {
        match self.state.rewind_to.take() {
            RewindTo::None => {}
            RewindTo::LastNewline => {
                if let Some(newline_idx) = self.out.iter().rposition(|v| *v == b'\n') {
                    // Keep the newline itself in the buffer
                    self.out.truncate(newline_idx + 1);
                }
            }
        }
    }
}

impl Write for RewindableWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.apply_rewind();
        Write::write(self.out, buf)
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice]) -> io::Result<usize> {
        self.apply_rewind();
        Write::write_vectored(self.out, bufs)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.apply_rewind();
        Write::write_all(self.out, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.apply_rewind();
        Write::flush(self.out)
    }
}

impl Drop for RewindableWriter<'_> {
    fn drop(&mut self) {
        self.apply_rewind();
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    #[should_panic(expected = "this state already has an associated writer")]
    fn second_writer_panics() {
        let state = RewindState::new();
        let mut buf = Vec::new();
        let mut other_buf = Vec::new();

        let _writer = state.activate(&mut buf);
        let _other_writer = state.activate(&mut other_buf);
        // The second activate call should panic
    }

    #[test]
    fn rewind_newline() {
        let state = RewindState::new();
        let mut buf = b"line 1\nline 2".to_vec();

        {
            // No rewind initially
            let mut writer = state.activate(&mut buf);
            writer.write_all(b" before rewind").unwrap();
            assert_eq!(writer.out, b"line 1\nline 2 before rewind");

            // Rewind is applied on next write
            state.rewind_to(RewindTo::LastNewline);
            writer.write_all(b"different line\nafter rewind").unwrap();
            assert_eq!(writer.out, b"line 1\ndifferent line\nafter rewind");

            // Rewind is applied on flush
            state.rewind_to(RewindTo::LastNewline);
            writer.flush().unwrap();
            assert_eq!(writer.out, b"line 1\ndifferent line\n");

            // Rewind is cleared after being applied above
            writer.write_all(b"after clear").unwrap();
        }

        assert_eq!(buf, b"line 1\ndifferent line\nafter clear");
    }

    #[test]
    fn rewind_newline_on_drop() {
        let state = RewindState::new();
        let mut buf = Vec::new();

        {
            let mut writer = state.activate(&mut buf);
            writer.write_all(b"line 1\nline 2").unwrap();

            // Rewind is also applied when writer drops
            state.rewind_to(RewindTo::LastNewline);
        }

        assert_eq!(buf, b"line 1\n");
    }
}
