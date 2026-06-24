//! W3C Trace Context `traceparent` header parsing.
//!
//! Format: `{version}-{trace-id}-{parent-id}-{trace-flags}` (55 bytes, all ASCII).
//!
//! Example: `00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01`
//!
//! See <https://www.w3.org/TR/trace-context/#traceparent-header>.
//!
//! This is the inbound counterpart to the outbound formatter
//! [`w3c_traceparent`](crate::telemetry::tracing::user_tracing::w3c_traceparent): it parses the
//! W3C `traceparent` carried in the user-tracing control header so an inbound trace can be
//! continued by [`start_user_trace`](crate::telemetry::tracing::start_user_trace).

/// Parsed components of a valid W3C `traceparent` header value.
///
/// Constructed via [`TraceparentContext::parse`]. Round-trippable via
/// [`TraceparentContext::to_traceparent_string`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceparentContext {
    /// W3C trace-context version. Only `0` (the `00` wire version) is currently accepted.
    pub version: u8,
    /// 16-byte (128-bit) trace identifier.
    pub trace_id: [u8; 16],
    /// 8-byte (64-bit) parent span identifier.
    pub parent_id: [u8; 8],
    /// Trace flags bitfield; bit 0 is the "sampled" flag.
    pub trace_flags: u8,
}

impl TraceparentContext {
    /// Parse a `traceparent` header value.
    ///
    /// Returns `None` on any of:
    /// - Not valid UTF-8
    /// - Wrong total length or missing/extra dashes
    /// - Wrong field widths (version=2, trace-id=32, parent-id=16, flags=2)
    /// - Non-hex characters in any field
    /// - Unsupported version (only `00` accepted per current W3C spec)
    /// - All-zero trace-id or parent-id (invalid per spec)
    pub fn parse(value: &[u8]) -> Option<Self> {
        let s = std::str::from_utf8(value).ok()?;

        let mut parts = s.splitn(4, '-');
        let version_str = parts.next()?;
        let trace_id_str = parts.next()?;
        let parent_id_str = parts.next()?;
        let flags_str = parts.next()?;

        // Trailing content (extra dashes) ends up in `flags_str` since `splitn(4)`
        // packs any remainder into the last segment; the length check below catches it.
        if version_str.len() != 2
            || trace_id_str.len() != 32
            || parent_id_str.len() != 16
            || flags_str.len() != 2
        {
            return None;
        }

        let version = u8::from_str_radix(version_str, 16).ok()?;
        // Only version 00 is supported.
        if version != 0 {
            return None;
        }

        let trace_flags = u8::from_str_radix(flags_str, 16).ok()?;

        let mut trace_id = [0u8; 16];
        hex::decode_to_slice(trace_id_str, &mut trace_id).ok()?;

        let mut parent_id = [0u8; 8];
        hex::decode_to_slice(parent_id_str, &mut parent_id).ok()?;

        // Per W3C spec, all-zero trace-id and parent-id are invalid.
        if trace_id == [0u8; 16] || parent_id == [0u8; 8] {
            return None;
        }

        Some(Self {
            version,
            trace_id,
            parent_id,
            trace_flags,
        })
    }

    /// Returns true if the sampled bit (bit 0) of `trace_flags` is set,
    /// indicating the caller decided this trace should be recorded.
    pub fn is_sampled(&self) -> bool {
        self.trace_flags & 0x01 != 0
    }

    /// Re-serialize the parsed context back to a canonical W3C `traceparent` string.
    pub fn to_traceparent_string(self) -> String {
        let mut buf = vec![0u8; 55];
        hex::encode_to_slice([self.version], &mut buf[0..2]).expect("size=2");
        buf[2] = b'-';
        hex::encode_to_slice(self.trace_id, &mut buf[3..35]).expect("size=32");
        buf[35] = b'-';
        hex::encode_to_slice(self.parent_id, &mut buf[36..52]).expect("size=16");
        buf[52] = b'-';
        hex::encode_to_slice([self.trace_flags], &mut buf[53..55]).expect("size=2");
        String::from_utf8(buf).expect("valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01";

    #[test]
    fn parse_valid() {
        let tp = TraceparentContext::parse(VALID.as_bytes()).expect("valid");
        assert_eq!(tp.version, 0);
        assert_eq!(
            tp.trace_id,
            [
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0x00, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ]
        );
        assert_eq!(
            tp.parent_id,
            [0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]
        );
        assert_eq!(tp.trace_flags, 0x01);
        assert!(tp.is_sampled());
    }

    #[test]
    fn roundtrip() {
        let tp = TraceparentContext::parse(VALID.as_bytes()).expect("valid");
        assert_eq!(tp.to_traceparent_string(), VALID);
    }

    #[test]
    fn uppercase_hex_accepted() {
        // Uppercase input accepted; output is always lowercase.
        let uppercase = "00-11223344556677889900AABBCCDDEEFF-A1B2C3D4E5F60718-01";
        let tp = TraceparentContext::parse(uppercase.as_bytes()).expect("uppercase accepted");
        assert_eq!(tp.to_traceparent_string(), VALID);
    }

    #[test]
    fn sampled_with_extra_flags() {
        // Bit 0 = sampled; extra bits (e.g. 0x02) are preserved and propagated.
        let s = "00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-03";
        let tp = TraceparentContext::parse(s.as_bytes()).expect("valid");
        assert_eq!(tp.trace_flags, 0x03);
        assert!(tp.is_sampled());
    }

    #[test]
    fn unsampled_is_valid() {
        let s = "00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-00";
        let tp = TraceparentContext::parse(s.as_bytes()).expect("valid");
        assert_eq!(tp.trace_flags, 0x00);
        assert!(!tp.is_sampled());
    }

    #[test]
    fn empty() {
        assert!(TraceparentContext::parse(b"").is_none());
    }

    #[test]
    fn degenerate() {
        assert!(TraceparentContext::parse(b"---").is_none());
        assert!(TraceparentContext::parse(b"00-1-1-00").is_none());
    }

    #[test]
    fn wrong_total_length() {
        // Too short (flags truncated)
        assert!(
            TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-0")
                .is_none()
        );
        // Too long (extra char on flags)
        assert!(
            TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-012")
                .is_none()
        );
    }

    #[test]
    fn wrong_field_sizes() {
        // version too short
        assert!(
            TraceparentContext::parse(b"0-11223344556677889900aabbccddeeff0-a1b2c3d4e5f60718-01")
                .is_none()
        );
        // trace-id too long
        assert!(
            TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff0-1b2c3d4e5f60718-01")
                .is_none()
        );
        // trace-id too short
        assert!(
            TraceparentContext::parse(b"00-11223344556677889900aabbccddee-a1b2c3d4e5f6071800-01")
                .is_none()
        );
        // parent-id too long
        assert!(
            TraceparentContext::parse(b"00-1223344556677889900aabbccddeeff-a1b2c3d4e5f607180-01")
                .is_none()
        );
        // parent-id too short
        assert!(
            TraceparentContext::parse(b"00-112233445566778899900aabbccddeeff-1b2c3d4e5f6071-01")
                .is_none()
        );
    }

    #[test]
    fn empty_fields() {
        assert!(TraceparentContext::parse(b"00--a1b2c3d4e5f60718-01").is_none());
        assert!(TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff--01").is_none());
    }

    #[test]
    fn bad_hex() {
        assert!(
            TraceparentContext::parse(b"0g-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01")
                .is_none()
        );
        assert!(
            TraceparentContext::parse(b"00-x1223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01")
                .is_none()
        );
        assert!(
            TraceparentContext::parse(b"00-11223344556677889900aabbccddeeff-a1b2c3d4e5f6071x-01")
                .is_none()
        );
    }

    #[test]
    fn unsupported_version() {
        // Future versions rejected: current spec only defines version 00.
        assert!(
            TraceparentContext::parse(b"01-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01")
                .is_none()
        );
        assert!(
            TraceparentContext::parse(b"ff-11223344556677889900aabbccddeeff-a1b2c3d4e5f60718-01")
                .is_none()
        );
    }

    #[test]
    fn all_zero_trace_id_rejected() {
        let s = "00-00000000000000000000000000000000-a1b2c3d4e5f60718-01";
        assert!(TraceparentContext::parse(s.as_bytes()).is_none());
    }

    #[test]
    fn all_zero_parent_id_rejected() {
        let s = "00-11223344556677889900aabbccddeeff-0000000000000000-01";
        assert!(TraceparentContext::parse(s.as_bytes()).is_none());
    }

    #[test]
    fn non_utf8() {
        assert!(TraceparentContext::parse(&[0xff, 0xfe, 0xfd]).is_none());
    }
}
