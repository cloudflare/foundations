use std::error::Error;
use std::fmt;

// Adapted from prometools' `serde::error::Error`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
/// An error produced while serializing a metric label set.
#[derive(Debug)]
pub struct LabelError {
    message: String,
}

impl LabelError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for LabelError {}

impl serde::ser::Error for LabelError {
    fn custom<T>(message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::new(message.to_string())
    }
}
