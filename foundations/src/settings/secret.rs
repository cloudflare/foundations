//! Wrappers around [`String`] and [`Vec<u8>`] to protect them from being printed accidentally.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::borrow::{Borrow, BorrowMut};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A [`String`] wrapper for settings fields which redacts its content when formatted.
///
/// This should be used for fields that must not be exposed by accident, for example in logs.
/// Access the underlying value explicitly using `.expose()`, `.expose_mut()`, or (as a last
/// resort) `.0`.
#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Secret(pub String);

impl Secret {
    /// Expose the secret explicitly to code expecting a [`String`].
    #[inline]
    pub fn expose(&self) -> &String {
        &self.0
    }

    /// Expose the secret explicitly to code expecting a mutable [`String`].
    #[inline]
    pub fn expose_mut(&mut self) -> &mut String {
        &mut self.0
    }
}

impl AsRef<str> for Secret {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<[u8]> for Secret {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        AsRef::as_ref(&self.0)
    }
}

impl AsRef<std::ffi::OsStr> for Secret {
    #[inline]
    fn as_ref(&self) -> &std::ffi::OsStr {
        AsRef::as_ref(&self.0)
    }
}

impl AsRef<std::path::Path> for Secret {
    #[inline]
    fn as_ref(&self) -> &std::path::Path {
        AsRef::as_ref(&self.0)
    }
}

impl AsMut<str> for Secret {
    #[inline]
    fn as_mut(&mut self) -> &mut str {
        &mut self.0
    }
}

impl Borrow<str> for Secret {
    #[inline]
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl BorrowMut<str> for Secret {
    #[inline]
    fn borrow_mut(&mut self) -> &mut str {
        &mut self.0
    }
}

impl fmt::Debug for Secret {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Secret")
    }
}

impl fmt::Display for Secret {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("REDACTED")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_none()
    }
}

impl super::Settings for Secret {}

impl Zeroize for Secret {
    #[inline]
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for Secret {}

/// A [`Vec<u8>`] wrapper for settings fields which redacts its content when formatted.
///
/// This should be used for fields that must not be exposed by accident, for example in logs.
/// Access the underlying value explicitly using `.expose()`, `.expose_mut()`, or (as a last
/// resort) `.0`.
#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RawSecret(pub Vec<u8>);

impl RawSecret {
    /// Expose the secret explicitly to code expecting a [`Vec<u8>`].
    #[inline]
    pub fn expose(&self) -> &Vec<u8> {
        &self.0
    }

    /// Expose the secret explicitly to code expecting a mutable [`Vec<u8>`].
    #[inline]
    pub fn expose_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

impl AsRef<[u8]> for RawSecret {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsMut<[u8]> for RawSecret {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl Borrow<[u8]> for RawSecret {
    #[inline]
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl BorrowMut<[u8]> for RawSecret {
    #[inline]
    fn borrow_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl fmt::Debug for RawSecret {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("SecretBytes")
    }
}

impl fmt::Display for RawSecret {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("REDACTED")
    }
}

impl Serialize for RawSecret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_none()
    }
}

// `<Vec<u8>>::deserialize` is not specialized, so doesn't implement `visit_bytes`
impl<'de> Deserialize<'de> for RawSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> de::Visitor<'de> for BytesVisitor {
            type Value = RawSecret;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a byte array")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(RawSecret(v.as_bytes().to_vec()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(RawSecret(v.into_bytes()))
            }

            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(RawSecret(v.to_vec()))
            }

            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(RawSecret(v))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(b) = seq.next_element()? {
                    bytes.push(b);
                }
                Ok(RawSecret(bytes))
            }
        }

        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

impl super::Settings for RawSecret {}

impl Zeroize for RawSecret {
    fn zeroize(&mut self) {
        // The Zeroize impl for Vec<Z> is generic, so it needs to zeroize each element
        // individually. This is not necessary for Vec<u8>, we can just overwrite the buffer.
        self.0.clear();
        self.0.spare_capacity_mut().zeroize();
    }
}

impl Drop for RawSecret {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for RawSecret {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted() {
        const SECRET: &str = "SUPER_SECRET_VALUE";
        let secret = Secret(SECRET.to_owned());
        let raw_secret = RawSecret(SECRET.as_bytes().to_vec());

        let formatted = format!(
            "plain: {secret}\nplain dbg: {secret:?}\nraw: {raw_secret}\nraw dbg: {raw_secret:?}\n"
        );
        let leaks_plaintext = formatted.contains(SECRET);
        let leaks_bytes = formatted.contains(&format!("{:?}", SECRET.as_bytes()));

        assert!(
            !leaks_plaintext && !leaks_bytes,
            "(Raw)Secret leaked data:\n\n{formatted}"
        );
    }
}
