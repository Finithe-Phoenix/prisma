//! Minimal guest Thread Environment Block (TEB).
//!
//! A Windows x64 thread reads its TEB through the GS segment: the C runtime and
//! SEH machinery look up the stack bounds (`NtTib.StackBase`/`StackLimit`), the
//! self-pointer (`NtTib.Self`), and the PEB at fixed offsets. This builds a TEB
//! with those canonical fields so a guest that dereferences `gs:[...]` at
//! startup sees sane values; the session points the thread's `gs_base` at the
//! TEB's guest address. The full ~0x1800-byte TEB is modelled lazily — only the
//! fields a startup path actually reads are populated, the rest left zero.

/// Byte offsets of the modelled fields in the x64 TEB (Microsoft `winnt.h` /
/// the documented `NT_TIB` layout).
pub mod offsets {
    /// `NtTib.StackBase` — the high address (top) of the thread stack.
    pub const STACK_BASE: usize = 0x08;
    /// `NtTib.StackLimit` — the low address (bottom) of the thread stack.
    pub const STACK_LIMIT: usize = 0x10;
    /// `NtTib.Self` — a pointer to the TEB itself.
    pub const SELF: usize = 0x30;
    /// `ProcessEnvironmentBlock` — pointer to the PEB.
    pub const PEB: usize = 0x60;
}

/// Bytes covered by the modelled TEB prefix (through the PEB pointer at 0x60).
pub const TEB_MIN_SIZE: usize = 0x100;

/// The fields needed to materialise a minimal TEB in guest memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Teb {
    /// Guest address the TEB is mapped at (what `Self` and `gs_base` point to).
    pub addr: u64,
    /// Stack top (high address).
    pub stack_base: u64,
    /// Stack bottom (low address).
    pub stack_limit: u64,
    /// Guest address of the PEB (0 if none yet).
    pub peb: u64,
}

impl Teb {
    /// Serialize the modelled prefix to its little-endian on-stack layout. The
    /// `Self` field points back at `addr`, so the guest's `gs:[0x30]` round-trips
    /// to the TEB base as Windows expects.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; TEB_MIN_SIZE] {
        let mut b = [0u8; TEB_MIN_SIZE];
        b[offsets::STACK_BASE..offsets::STACK_BASE + 8]
            .copy_from_slice(&self.stack_base.to_le_bytes());
        b[offsets::STACK_LIMIT..offsets::STACK_LIMIT + 8]
            .copy_from_slice(&self.stack_limit.to_le_bytes());
        b[offsets::SELF..offsets::SELF + 8].copy_from_slice(&self.addr.to_le_bytes());
        b[offsets::PEB..offsets::PEB + 8].copy_from_slice(&self.peb.to_le_bytes());
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
    fn teb_lays_out_canonical_fields() {
        let teb = Teb {
            addr: 0x7_0000,
            stack_base: 0x2_0010_0000,
            stack_limit: 0x2_0000_0000,
            peb: 0x8_0000,
        };
        let b = teb.to_bytes();
        assert_eq!(le_u64(&b, offsets::STACK_BASE), 0x2_0010_0000);
        assert_eq!(le_u64(&b, offsets::STACK_LIMIT), 0x2_0000_0000);
        // Self points back at the TEB base — gs:[0x30] resolves to gs_base.
        assert_eq!(le_u64(&b, offsets::SELF), teb.addr);
        assert_eq!(le_u64(&b, offsets::PEB), 0x8_0000);
    }

    #[test]
    fn unmodelled_fields_are_zero() {
        let teb = Teb {
            addr: 0x1000,
            stack_base: 0x2000,
            stack_limit: 0x1500,
            peb: 0,
        };
        let b = teb.to_bytes();
        // The ExceptionList slot (0x00) and a gap (0x38) are left zero.
        assert_eq!(le_u64(&b, 0x00), 0);
        assert_eq!(le_u64(&b, 0x38), 0);
        assert_eq!(b.len(), TEB_MIN_SIZE);
    }

    #[test]
    fn stack_bounds_come_from_a_guest_stack_shape() {
        // StackBase is the top, StackLimit the bottom: base > limit, and the
        // span is the usable stack.
        let teb = Teb {
            addr: 0x9_0000,
            stack_base: 0x3_0010_0000,
            stack_limit: 0x3_0000_0000,
            peb: 0,
        };
        assert!(teb.stack_base > teb.stack_limit);
        assert_eq!(teb.stack_base - teb.stack_limit, 0x10_0000); // 1 MiB
    }
}
