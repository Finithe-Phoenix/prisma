//! Minimal guest Process Environment Block (PEB).
//!
//! The PEB holds per-process state the guest reads at startup: chiefly
//! `ImageBaseAddress` (what `GetModuleHandle(NULL)` returns and the CRT uses to
//! find the headers), plus pointers to the loader data (`Ldr`) and the process
//! parameters. The [`crate::teb`]'s `ProcessEnvironmentBlock` field points here.
//! Like the TEB, only the fields a startup path actually reads are modelled; the
//! full PEB is far larger and filled lazily.

/// Byte offsets of the modelled fields in the x64 PEB (the documented layout).
pub mod offsets {
    /// `ImageBaseAddress` — the guest base the main image was mapped at.
    pub const IMAGE_BASE_ADDRESS: usize = 0x10;
    /// `Ldr` — pointer to the `PEB_LDR_DATA` module list.
    pub const LDR: usize = 0x18;
    /// `ProcessParameters` — pointer to `RTL_USER_PROCESS_PARAMETERS`.
    pub const PROCESS_PARAMETERS: usize = 0x20;
}

/// Bytes covered by the modelled PEB prefix (through ProcessParameters).
pub const PEB_MIN_SIZE: usize = 0x80;

/// The fields needed to materialise a minimal PEB in guest memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peb {
    /// Guest base address the main image was mapped at.
    pub image_base: u64,
    /// Guest address of the loader data (0 until the loader is modelled).
    pub ldr: u64,
    /// Guest address of the process parameters (0 until modelled).
    pub process_parameters: u64,
}

impl Peb {
    /// A PEB carrying just the image base — the minimum a typical CRT startup
    /// reads (`GetModuleHandle(NULL)` / header lookup).
    #[must_use]
    pub const fn with_image_base(image_base: u64) -> Self {
        Self {
            image_base,
            ldr: 0,
            process_parameters: 0,
        }
    }

    /// Serialize the modelled prefix to its little-endian on-stack layout.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PEB_MIN_SIZE] {
        let mut b = [0u8; PEB_MIN_SIZE];
        b[offsets::IMAGE_BASE_ADDRESS..offsets::IMAGE_BASE_ADDRESS + 8]
            .copy_from_slice(&self.image_base.to_le_bytes());
        b[offsets::LDR..offsets::LDR + 8].copy_from_slice(&self.ldr.to_le_bytes());
        b[offsets::PROCESS_PARAMETERS..offsets::PROCESS_PARAMETERS + 8]
            .copy_from_slice(&self.process_parameters.to_le_bytes());
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_u64(b: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
    }

    #[test]
    fn image_base_lands_at_the_canonical_offset() {
        let peb = Peb::with_image_base(0x1_4000_0000);
        let b = peb.to_bytes();
        assert_eq!(le_u64(&b, offsets::IMAGE_BASE_ADDRESS), 0x1_4000_0000);
        // Unset pointers are zero, the size covers ProcessParameters.
        assert_eq!(le_u64(&b, offsets::LDR), 0);
        assert_eq!(le_u64(&b, offsets::PROCESS_PARAMETERS), 0);
        assert_eq!(b.len(), PEB_MIN_SIZE);
    }

    #[test]
    fn all_modelled_pointers_serialize() {
        let peb = Peb {
            image_base: 0x1_4000_0000,
            ldr: 0x7_0000,
            process_parameters: 0x8_0000,
        };
        let b = peb.to_bytes();
        assert_eq!(le_u64(&b, offsets::IMAGE_BASE_ADDRESS), 0x1_4000_0000);
        assert_eq!(le_u64(&b, offsets::LDR), 0x7_0000);
        assert_eq!(le_u64(&b, offsets::PROCESS_PARAMETERS), 0x8_0000);
        // The bytes before ImageBaseAddress (flags at 0x00..0x10) stay zero.
        assert_eq!(le_u64(&b, 0x00), 0);
        assert_eq!(le_u64(&b, 0x08), 0);
    }
}
