//! Initial guest-thread CPU state.
//!
//! Before the dispatcher runs the entry block, the guest thread needs a valid
//! initial register frame — chiefly a stack pointer. The x64 ABI requires RSP
//! to be 16-byte aligned at a call boundary, and a PE entry point runs as if
//! freshly called, so the loader hands control with an aligned stack. This sets
//! that up; the fuller TEB/PEB + argument setup is a later session milestone.

use crate::executor::{gpr, CpuStateFrame};

/// Builders for a fresh guest thread's [`CpuStateFrame`].
pub struct GuestThread;

impl GuestThread {
    /// Initial state for a fresh thread: all GPRs zero, RSP at the top of the
    /// guest stack rounded **down** to a 16-byte boundary (never above
    /// `stack_top`, so it stays inside the mapped stack region).
    #[must_use]
    pub fn initial(stack_top: u64) -> CpuStateFrame {
        let mut frame = CpuStateFrame::default();
        frame.gpr[gpr::RSP] = align_down_16(stack_top);
        frame
    }

    /// Initial state with the thread's TEB wired in: like [`Self::initial`] but
    /// also points `gs_base` at `teb_addr`, so the guest's `gs:[...]` accesses
    /// (stack bounds, SEH, the Self/PEB pointers) reach the TEB the loader
    /// materialised — the x64 convention is `GS` = the TEB base.
    #[must_use]
    pub fn with_teb(stack_top: u64, teb_addr: u64) -> CpuStateFrame {
        let mut frame = Self::initial(stack_top);
        frame.gs_base = teb_addr;
        frame
    }

    /// Reserve `bytes` on the guest stack, returning the new RSP (the lowest
    /// address of the reserved region). Saturating at 0 — a stack that would
    /// underflow the address space is clamped rather than wrapped, so a caller
    /// writing the reserved region never targets a wrapped high address.
    #[must_use]
    pub fn reserve(rsp: u64, bytes: u64) -> u64 {
        rsp.saturating_sub(bytes)
    }
}

/// Round `addr` down to the nearest multiple of 16.
const fn align_down_16(addr: u64) -> u64 {
    addr & !0xF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_rsp_is_16_aligned_and_not_above_stack_top() {
        for top in [
            0x7FFF_FFFF_0000u64,
            0x1000,
            0x100F,
            0x1010,
            0xFFFF_FFFF_FFFF_FFFF,
        ] {
            let frame = GuestThread::initial(top);
            let rsp = frame.gpr[gpr::RSP];
            assert_eq!(rsp % 16, 0, "RSP must be 16-aligned");
            assert!(rsp <= top, "RSP must not exceed the stack top");
            assert!(top - rsp < 16, "alignment loses at most 15 bytes");
        }
    }

    #[test]
    fn initial_zeroes_all_other_registers() {
        let frame = GuestThread::initial(0x8000_0000);
        for (i, &r) in frame.gpr.iter().enumerate() {
            if i != gpr::RSP {
                assert_eq!(r, 0, "gpr[{i}] should start zero");
            }
        }
    }

    #[test]
    fn with_teb_points_gs_base_at_the_teb_and_keeps_rsp() {
        let frame = GuestThread::with_teb(0x2_0010_0000, 0x7_0000);
        assert_eq!(frame.gs_base, 0x7_0000, "gs_base must point at the TEB");
        // RSP is still seeded exactly as initial() would.
        assert_eq!(
            frame.gpr[gpr::RSP],
            GuestThread::initial(0x2_0010_0000).gpr[gpr::RSP]
        );
        assert_eq!(frame.gpr[gpr::RSP] % 16, 0);
    }

    #[test]
    fn reserve_moves_rsp_down_and_saturates() {
        assert_eq!(GuestThread::reserve(0x2000, 0x100), 0x1F00);
        // Underflow is clamped to 0, never wrapped to a high address.
        assert_eq!(GuestThread::reserve(0x10, 0x100), 0);
    }
}
