//! Pointer-checked signal syscalls.
//!
//! `rt_sigprocmask` reads/writes the guest's blocked-signal mask through guest
//! pointers. It exercises both directions of checked memory access: write the
//! old mask to `oldset`, read the new mask from `set` — each through a
//! [`GuestRegion`] so a bad pointer is a guest `EFAULT`, never an out-of-bounds
//! host access. The mask itself lives in [`SignalState`], which enforces the
//! kernel rule that SIGKILL/SIGSTOP can never be blocked.

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_signal::{SignalState, SigprocmaskHow, Sigset};

/// Why an `rt_sigprocmask` call failed (each maps to a guest errno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigError {
    /// `how` is not SIG_BLOCK/UNBLOCK/SETMASK — guest `EINVAL`.
    BadHow(i32),
    /// A `set`/`oldset` pointer is not accessible guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `rt_sigprocmask(how, set, oldset)`: optionally report the current blocked
/// mask to `oldset`, then optionally apply `how` with the mask read from `set`.
/// A null pointer (address 0) skips that half, per the kernel convention.
///
/// `how` is validated up front when `set` is non-null, so an invalid `how`
/// yields `EINVAL` before any guest memory is written — matching the kernel,
/// which applies no change on error.
///
/// # Errors
/// [`SigError::BadHow`] for an unknown `how` (with a non-null `set`),
/// [`SigError::Fault`] if a non-null `set`/`oldset` is not accessible.
pub fn rt_sigprocmask(
    state: &mut SignalState,
    mem: &mut GuestRegion,
    how: i32,
    set_addr: u64,
    oldset_addr: u64,
) -> Result<(), SigError> {
    // Decode `how` before any effect, so a bad `how` is EINVAL with no write.
    let how_decoded = if set_addr == 0 {
        None
    } else {
        Some(SigprocmaskHow::from_raw(how).ok_or(SigError::BadHow(how))?)
    };

    if oldset_addr != 0 {
        mem.write(oldset_addr, &state.blocked().to_guest_bytes())
            .map_err(SigError::Fault)?;
    }

    if let Some(how) = how_decoded {
        let bytes = mem.read(set_addr, Sigset::SIZE).map_err(SigError::Fault)?;
        let new = Sigset::from_guest_bytes(bytes).ok_or(SigError::Fault(RangeError::Unmapped))?;
        state.set_mask(how, new);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{rt_sigprocmask, SigError};
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_signal::{SignalState, Sigset};

    // SIG_BLOCK = 0, SIG_SETMASK = 2.
    const SIG_BLOCK: i32 = 0;
    const SIG_SETMASK: i32 = 2;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn block_applies_the_new_mask() {
        let mut st = SignalState::new();
        let mut buf = [0u8; 16];
        // Put a mask blocking signal 11 at guest 0x1000.
        let mut set = Sigset::empty();
        set.insert(11);
        buf[0..8].copy_from_slice(&set.to_guest_bytes());
        let mut mem = region(&mut buf);
        rt_sigprocmask(&mut st, &mut mem, SIG_BLOCK, 0x1000, 0).expect("ok");
        assert!(st.blocked().contains(11));
    }

    #[test]
    fn oldset_reports_the_previous_mask() {
        let mut st = SignalState::new();
        let mut pre = Sigset::empty();
        pre.insert(15);
        st.set_mask(prisma_runtime::guest_signal::SigprocmaskHow::SetMask, pre);

        let mut buf = [0u8; 16];
        // New mask (empty) at 0x1000; oldset out at 0x1008.
        buf[0..8].copy_from_slice(&Sigset::empty().to_guest_bytes());
        let mut mem = region(&mut buf);
        rt_sigprocmask(&mut st, &mut mem, SIG_SETMASK, 0x1000, 0x1008).expect("ok");
        // oldset holds the pre-change mask (signal 15 blocked).
        let old = Sigset::from_guest_bytes(&buf[8..16]).unwrap();
        assert!(old.contains(15));
    }

    #[test]
    fn null_set_only_reports_old_and_changes_nothing() {
        let mut st = SignalState::new();
        let mut m = Sigset::empty();
        m.insert(12);
        st.set_mask(prisma_runtime::guest_signal::SigprocmaskHow::SetMask, m);
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // set == NULL (0): only oldset reported, mask unchanged.
        rt_sigprocmask(&mut st, &mut mem, 999, 0, 0x1000).expect("null set ignores how");
        assert!(st.blocked().contains(12)); // unchanged
        assert!(Sigset::from_guest_bytes(&buf[0..8]).unwrap().contains(12));
    }

    #[test]
    fn bad_how_with_a_real_set_is_einval_before_any_write() {
        let mut st = SignalState::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        assert_eq!(
            rt_sigprocmask(&mut st, &mut mem, 7, 0x1000, 0x1008),
            Err(SigError::BadHow(7))
        );
        // No write happened: oldset region is still zero.
        assert_eq!(&buf[8..16], &[0u8; 8]);
    }

    #[test]
    fn an_unmapped_pointer_is_efault() {
        let mut st = SignalState::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        // oldset at 0x100A leaves only 6 bytes — too short for an 8-byte mask.
        assert_eq!(
            rt_sigprocmask(&mut st, &mut mem, SIG_BLOCK, 0x1000, 0x100A),
            Err(SigError::Fault(RangeError::Unmapped))
        );
    }
}
