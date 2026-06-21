//! Guest string materialization (command line, image path).
//!
//! `PEB->ProcessParameters` references the command line and image path as
//! `UNICODE_STRING`s pointing at UTF-16 buffers. Placing one in guest memory
//! means writing the UTF-16 bytes at some address and pointing a descriptor at
//! them. This composes [`crate::windows_string`] into that one step: encode the
//! buffer (NUL-terminated, as Windows stores it) and build the descriptor.

use crate::windows_string::{to_utf16le, unicode_string, UNICODE_STRING_SIZE};

/// A guest string ready to place in memory: the UTF-16 buffer to write at the
/// chosen address, plus the `UNICODE_STRING` descriptor pointing at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestString {
    /// The `UNICODE_STRING` descriptor (`Length`/`MaximumLength`/`Buffer`).
    pub descriptor: [u8; UNICODE_STRING_SIZE],
    /// The UTF-16LE buffer, NUL-terminated, to write at `buffer_addr`.
    pub utf16: Vec<u8>,
}

/// Materialize `s` as a guest string whose buffer lives at `buffer_addr`.
///
/// The buffer is UTF-16LE with a trailing NUL (Windows stores command lines
/// NUL-terminated); the descriptor's `Length` excludes the NUL, `MaximumLength`
/// includes it — exactly what `RtlInitUnicodeString` would produce.
#[must_use]
pub fn materialize(s: &str, buffer_addr: u64) -> GuestString {
    let mut utf16 = to_utf16le(s);
    let content_len = utf16.len();
    utf16.extend_from_slice(&[0, 0]); // UTF-16 NUL terminator
    let descriptor = unicode_string(buffer_addr, content_len, utf16.len());
    GuestString { descriptor, utf16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_command_line_buffer_and_descriptor() {
        let gs = materialize("cmd", 0x2_0000_0000);
        // "cmd" = 3 UTF-16 units (6 bytes) + a 2-byte NUL = 8 bytes.
        assert_eq!(gs.utf16, vec![b'c', 0, b'm', 0, b'd', 0, 0, 0]);
        // Length excludes the NUL (6), MaximumLength includes it (8).
        assert_eq!(u16::from_le_bytes([gs.descriptor[0], gs.descriptor[1]]), 6);
        assert_eq!(u16::from_le_bytes([gs.descriptor[2], gs.descriptor[3]]), 8);
        // Buffer points at the chosen guest address.
        assert_eq!(
            u64::from_le_bytes(gs.descriptor[8..16].try_into().unwrap()),
            0x2_0000_0000
        );
    }

    #[test]
    fn empty_string_is_just_a_nul() {
        let gs = materialize("", 0x1000);
        assert_eq!(gs.utf16, vec![0, 0]);
        assert_eq!(u16::from_le_bytes([gs.descriptor[0], gs.descriptor[1]]), 0); // Length
        assert_eq!(u16::from_le_bytes([gs.descriptor[2], gs.descriptor[3]]), 2);
        // MaximumLength
    }

    #[test]
    fn a_realistic_image_path_round_trips_its_length() {
        let path = r"C:\Windows\System32\notepad.exe";
        let gs = materialize(path, 0x3000);
        // Each ASCII char is one UTF-16 unit (2 bytes); Length = chars * 2.
        let expected_len = u16::try_from(path.chars().count() * 2).unwrap();
        assert_eq!(
            u16::from_le_bytes([gs.descriptor[0], gs.descriptor[1]]),
            expected_len
        );
        assert_eq!(gs.utf16.len(), usize::from(expected_len) + 2);
    }
}
