//! Helper struct to load plain data from external sources referenced in a settings file.

use super::secret::{RawSecret, Secret};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

trait DeserializeExternal: for<'de> Deserialize<'de> {
    fn load_from_env(var_name: &str) -> Result<Self, std::env::VarError>;
    fn load_from_file(path: &str) -> std::io::Result<Self>;

    fn deserialize_from_env<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let var_name = String::deserialize(deserializer)?;
        Self::load_from_env(&var_name).map_err(|e| {
            D::Error::custom(format!(
                "failed to read external data from ${var_name}: {e}"
            ))
        })
    }

    fn deserialize_from_file<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let path = String::deserialize(deserializer)?;
        Self::load_from_file(&path).map_err(|e| {
            D::Error::custom(format!("failed to read external data from `{path}`: {e}"))
        })
    }
}

impl DeserializeExternal for String {
    #[inline]
    fn load_from_env(var_name: &str) -> Result<Self, std::env::VarError> {
        std::env::var(var_name)
    }

    #[inline]
    fn load_from_file(path: &str) -> std::io::Result<Self> {
        std::fs::read_to_string(path)
    }
}

impl DeserializeExternal for Vec<u8> {
    #[inline]
    fn load_from_env(var_name: &str) -> Result<Self, std::env::VarError> {
        // We don't use the `OsString` interface here since its encoding is OS- and
        // version-specific. If the data can't be represented as UTF-8, it's safer
        // to return an error.
        std::env::var(var_name).map(|v| v.into_bytes())
    }

    #[inline]
    fn load_from_file(path: &str) -> std::io::Result<Self> {
        std::fs::read(path)
    }
}

impl DeserializeExternal for Secret {
    #[inline]
    fn load_from_env(var_name: &str) -> Result<Self, std::env::VarError> {
        std::env::var(var_name).map(Self)
    }

    #[inline]
    fn load_from_file(path: &str) -> std::io::Result<Self> {
        std::fs::read_to_string(path).map(Self)
    }
}

impl DeserializeExternal for RawSecret {
    #[inline]
    fn load_from_env(var_name: &str) -> Result<Self, std::env::VarError> {
        // We don't use the `OsString` interface here since its encoding is OS- and
        // version-specific. If the data can't be represented as UTF-8, it's safer
        // to return an error.
        std::env::var(var_name).map(|v| Self(v.into_bytes()))
    }

    #[inline]
    fn load_from_file(path: &str) -> std::io::Result<Self> {
        std::fs::read(path).map(Self)
    }
}

// We don't remember the env var/path from which we loaded a `MaybeExternal`, so we can't
// serialize them back again. Output a None instead.
fn serialize_as_none<S: Serializer, T>(_: &T, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_none()
}

/// A helper to load plain values (strings, bytes, and secrets) from external sources like
/// environment variables and the file system.
///
/// The user can select which source to use in their configuration file, or whether to read
/// an inline value from the config itself.
///
/// The following data types are currently supported:
/// - [`String`]
/// - [`Vec<u8>`]
/// - [`Secret`]
/// - [`RawSecret`]
///
/// # Example
/// ```rust
/// use foundations::settings::from_yaml_str;
/// use foundations::settings::external::MaybeExternal;
///
/// // Inline value
/// let data: MaybeExternal<String> = from_yaml_str("data: asdf").unwrap();
/// assert_eq!(data.as_ref(), "asdf");
///
/// # #[cfg(unix)] {
/// // Environment variable
/// let env: MaybeExternal<String> = from_yaml_str("env: HOME").unwrap();
/// assert!(env.as_ref().starts_with("/"));
///
/// // File on disk
/// let file: MaybeExternal<String> = from_yaml_str("file: /dev/null").unwrap();
/// assert_eq!(file.as_ref(), "");
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    bound(deserialize = "T: DeserializeExternal")
)]
#[cfg_attr(
    feature = "settings_deny_unknown_fields_by_default",
    serde(deny_unknown_fields)
)]
pub enum MaybeExternal<T> {
    /// Deserializes directly from inline data, without consulting external sources.
    Data(T),
    /// Deserializes into an environment variable name, which is then read from the environment.
    #[serde(
        serialize_with = "serialize_as_none",
        deserialize_with = "DeserializeExternal::deserialize_from_env"
    )]
    Env(T),
    /// Deserializes into a file path, which is then read from disk.
    #[serde(
        serialize_with = "serialize_as_none",
        deserialize_with = "DeserializeExternal::deserialize_from_file"
    )]
    File(T),
}

impl<T: Default> Default for MaybeExternal<T> {
    #[inline]
    fn default() -> Self {
        Self::Data(T::default())
    }
}

impl<T> AsRef<T> for MaybeExternal<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        let (Self::Data(v) | Self::Env(v) | Self::File(v)) = self;
        v
    }
}

impl<T> AsMut<T> for MaybeExternal<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        let (Self::Data(v) | Self::Env(v) | Self::File(v)) = self;
        v
    }
}

impl<T: fmt::Display> fmt::Display for MaybeExternal<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self.as_ref(), f)
    }
}

impl<T: DeserializeExternal + super::Settings> super::Settings for MaybeExternal<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_deserializes_from_env() {
        if std::env::var("NEXTEST").as_deref() != Ok("1") {
            return;
        }

        // SAFETY: We are running under nextest, which means each test runs in a
        // separate process. This fulfills set_var's single-threadedness requirement.
        unsafe {
            std::env::set_var("MY_CUSTOM_ENV_VAR", "my custom value");
        }

        let yaml = "env: MY_CUSTOM_ENV_VAR\n";
        let data: MaybeExternal<String> = crate::settings::from_yaml_str(yaml).unwrap();

        assert_eq!(data.as_ref(), "my custom value");
    }

    #[test]
    fn external_deserializes_from_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut tmp, b"hello\n").unwrap();

        let yaml = format!("file: {}\n", tmp.path().display());
        let data: MaybeExternal<String> = crate::settings::from_yaml_str(&yaml).unwrap();

        assert_eq!(data.as_ref(), "hello\n");
    }
}
