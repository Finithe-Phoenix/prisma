//! Minimal guest `RTL_USER_PROCESS_PARAMETERS`.
//!
//! `PEB->ProcessParameters` points at this block; the guest CRT reads the image
//! path (`GetModuleFileName`) and command line (`GetCommandLine`) from it as
//! `UNICODE_STRING`s at fixed offsets. This places those two descriptors — the
//! pair a startup path actually reads — into the block's on-stack layout; the
//! full structure (handles, environment, current directory, ...) is modelled
//! lazily, the rest left zero.

use crate::windows_string::UNICODE_STRING_SIZE;

/// Byte offsets of the modelled fields in the x64 `RTL_USER_PROCESS_PARAMETERS`.
pub mod offsets {
    /// `ImagePathName` — `UNICODE_STRING` of the main image's full path.
    pub const IMAGE_PATH_NAME: usize = 0x60;
    /// `CommandLine` — `UNICODE_STRING` of the process command line.
    pub const COMMAND_LINE: usize = 0x70;
}

/// Bytes covered by the modelled prefix (through `CommandLine` at 0x70).
pub const PROCESS_PARAMETERS_MIN_SIZE: usize = 0x80;

/// The two `UNICODE_STRING` descriptors a startup path reads. Build each with
/// [`crate::windows_string::unicode_string`] (or
/// [`crate::command_line::materialize`]'s `descriptor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessParameters {
    pub image_path: [u8; UNICODE_STRING_SIZE],
    pub command_line: [u8; UNICODE_STRING_SIZE],
}

impl ProcessParameters {
    /// Serialize the modelled prefix: the two descriptors at their canonical
    /// offsets, everything else zero.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PROCESS_PARAMETERS_MIN_SIZE] {
        let mut b = [0u8; PROCESS_PARAMETERS_MIN_SIZE];
        b[offsets::IMAGE_PATH_NAME..offsets::IMAGE_PATH_NAME + UNICODE_STRING_SIZE]
            .copy_from_slice(&self.image_path);
        b[offsets::COMMAND_LINE..offsets::COMMAND_LINE + UNICODE_STRING_SIZE]
            .copy_from_slice(&self.command_line);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_string::unicode_string;

    #[test]
    fn places_command_line_and_image_path_at_their_offsets() {
        let image_path = unicode_string(0x1000, 10, 12);
        let command_line = unicode_string(0x2000, 6, 8);
        let pp = ProcessParameters {
            image_path,
            command_line,
        };
        let b = pp.to_bytes();
        // The two UNICODE_STRING descriptors land at 0x60 / 0x70 verbatim.
        assert_eq!(&b[0x60..0x70], &image_path);
        assert_eq!(&b[0x70..0x80], &command_line);
        // Their Buffer pointers survive the placement.
        assert_eq!(
            u64::from_le_bytes(b[0x68..0x70].try_into().unwrap()),
            0x1000
        );
        assert_eq!(
            u64::from_le_bytes(b[0x78..0x80].try_into().unwrap()),
            0x2000
        );
        assert_eq!(b.len(), PROCESS_PARAMETERS_MIN_SIZE);
    }

    #[test]
    fn unmodelled_prefix_is_zero() {
        let pp = ProcessParameters {
            image_path: [0; UNICODE_STRING_SIZE],
            command_line: [0; UNICODE_STRING_SIZE],
        };
        let b = pp.to_bytes();
        // The header before ImagePathName (handles, flags, ...) stays zero.
        assert!(b[0..0x60].iter().all(|&x| x == 0));
    }
}
