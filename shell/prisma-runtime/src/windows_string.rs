//! Windows `UNICODE_STRING` + UTF-16 helpers for guest string fields.
//!
//! Windows stores strings the guest reads at startup — the command line, image
//! path, environment — as UTF-16 buffers referenced by a `UNICODE_STRING`
//! descriptor `{ Length, MaximumLength, Buffer }`. Building
//! `PEB->ProcessParameters` means placing the UTF-16 bytes somewhere in guest
//! memory and pointing a `UNICODE_STRING` at them. These are the two primitives
//! for that: encode the buffer, and lay out the descriptor.

/// Encode `s` to UTF-16 little-endian bytes — the buffer a [`unicode_string`]
/// points at.
///
/// No trailing NUL (a `UNICODE_STRING`'s `Length` is explicit); add one yourself
/// if a caller wants NUL-terminated storage.
#[must_use]
pub fn to_utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Serialized size of an x64 `UNICODE_STRING`.
///
/// Length(2) + MaximumLength(2) + pad(4) + Buffer(8) = 16 bytes.
pub const UNICODE_STRING_SIZE: usize = 16;

/// Lay out an x64 `UNICODE_STRING` pointing at `buffer_addr` with `byte_len`
/// valid bytes (`Length`), capacity `max_byte_len` (`MaximumLength`).
///
/// `Length`/`MaximumLength` are byte counts (not character counts), per the
/// Windows ABI. Both are 16-bit, so an over-long string is the caller's problem;
/// this saturates rather than truncating silently.
#[must_use]
pub fn unicode_string(
    buffer_addr: u64,
    byte_len: usize,
    max_byte_len: usize,
) -> [u8; UNICODE_STRING_SIZE] {
    let mut b = [0u8; UNICODE_STRING_SIZE];
    let len = u16::try_from(byte_len).unwrap_or(u16::MAX);
    let max = u16::try_from(max_byte_len).unwrap_or(u16::MAX);
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&max.to_le_bytes());
    // bytes 4..8 are alignment padding (Buffer is 8-aligned).
    b[8..16].copy_from_slice(&buffer_addr.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_encodes_little_endian_without_nul() {
        // "AB" -> 0x0041 0x0042, little-endian, no terminator.
        assert_eq!(to_utf16le("AB"), vec![0x41, 0x00, 0x42, 0x00]);
        assert_eq!(to_utf16le("").len(), 0);
        // A BMP non-ASCII char encodes to one 16-bit unit.
        assert_eq!(to_utf16le("\u{00E9}"), vec![0xE9, 0x00]); // é
    }

    #[test]
    fn unicode_string_lays_out_length_and_buffer() {
        // A command line "AB" is 4 UTF-16 bytes at some guest address.
        let buf_addr = 0x2_0000_0000;
        let utf16 = to_utf16le("AB");
        let us = unicode_string(buf_addr, utf16.len(), utf16.len() + 2);
        assert_eq!(u16::from_le_bytes([us[0], us[1]]), 4); // Length
        assert_eq!(u16::from_le_bytes([us[2], us[3]]), 6); // MaximumLength (+NUL room)
        assert_eq!(u64::from_le_bytes(us[8..16].try_into().unwrap()), buf_addr);
        assert_eq!(us.len(), 16);
    }

    #[test]
    fn over_long_lengths_saturate_not_wrap() {
        let us = unicode_string(0x1000, 0x1_0000, 0x2_0000);
        assert_eq!(u16::from_le_bytes([us[0], us[1]]), u16::MAX);
        assert_eq!(u16::from_le_bytes([us[2], us[3]]), u16::MAX);
    }
}
