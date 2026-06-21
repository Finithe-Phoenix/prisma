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
use prisma_runtime::guest_signal::{SigAction, SignalState, SigprocmaskHow, Sigset};

/// Why a signal syscall failed (each maps to a guest errno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigError {
    /// `how` is not SIG_BLOCK/UNBLOCK/SETMASK — guest `EINVAL`.
    BadHow(i32),
    /// The signal number is invalid or its disposition cannot be changed
    /// (SIGKILL/SIGSTOP) — guest `EINVAL`.
    BadSignal(u32),
    /// A `set`/`oldset`/`act` pointer is not accessible guest memory — `EFAULT`.
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

/// `rt_sigpending(set)`: write the set of currently-pending (raised but not yet
/// delivered) signals to the guest `set` pointer.
///
/// # Errors
/// [`SigError::Fault`] if `set` is not writable guest memory.
pub fn rt_sigpending(
    state: &SignalState,
    mem: &mut GuestRegion,
    set_addr: u64,
) -> Result<(), SigError> {
    mem.write(set_addr, &state.pending_set().to_guest_bytes())
        .map_err(SigError::Fault)
}

/// `rt_sigaction(sig, act, oldact)`: read the current disposition for `sig` into
/// `oldact` (when non-null), then install the disposition at `act` (when
/// non-null). Both pointers go through the range-checked [`GuestRegion`].
///
/// `sig` is validated first, so an invalid signal — or SIGKILL/SIGSTOP, whose
/// disposition the kernel never lets a process change — yields `EINVAL` with no
/// effect.
///
/// # Errors
/// [`SigError::BadSignal`] for an invalid/unchangeable signal,
/// [`SigError::Fault`] if a non-null `act`/`oldact` is not accessible.
pub fn rt_sigaction(
    state: &mut SignalState,
    mem: &mut GuestRegion,
    sig: u32,
    act_addr: u64,
    oldact_addr: u64,
) -> Result<(), SigError> {
    // The current disposition; `None` means `sig` is out of range.
    let current = state.action(sig).ok_or(SigError::BadSignal(sig))?;
    if oldact_addr != 0 {
        mem.write(oldact_addr, &current.to_guest_bytes())
            .map_err(SigError::Fault)?;
    }
    if act_addr != 0 {
        let bytes = mem
            .read(act_addr, SigAction::SIZE)
            .map_err(SigError::Fault)?;
        let new =
            SigAction::from_guest_bytes(bytes).ok_or(SigError::Fault(RangeError::Unmapped))?;
        if !state.set_action(sig, new) {
            // In range but unchangeable (SIGKILL/SIGSTOP).
            return Err(SigError::BadSignal(sig));
        }
    }
    Ok(())
}

/// Deliver signal `sig` to the (single, self) guest process — the primitive
/// behind `tkill` / `tgkill` / `kill` directed at the guest itself. `sig` `0` is
/// the kill family's existence-probe no-op (no signal is sent); a number outside
/// `1..=64` is rejected. A valid signal is marked pending for later delivery.
///
/// The caller (dispatcher) is responsible for resolving the target pid/tid and
/// answering `ESRCH` for anything other than the guest itself; this owns only
/// the signal-number validation and the raise.
///
/// # Errors
/// [`SigError::BadSignal`] if `sig` is out of the `1..=64` range.
pub fn send_signal(state: &mut SignalState, sig: u32) -> Result<(), SigError> {
    if sig == 0 {
        return Ok(());
    }
    if !(1..=64).contains(&sig) {
        return Err(SigError::BadSignal(sig));
    }
    state.raise(sig);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{rt_sigprocmask, send_signal, SigError};
    use prisma_orchestrator::address_space::{Protection, RangeError};
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_signal::{SigAction, SignalState, Sigset};

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

    #[test]
    fn rt_sigpending_writes_the_pending_set() {
        use super::rt_sigpending;
        let mut st = SignalState::new();
        st.raise(11);
        st.raise(17);
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        rt_sigpending(&st, &mut mem, 0x1000).expect("write ok");
        let reported = Sigset::from_guest_bytes(&buf).unwrap();
        assert!(reported.contains(11) && reported.contains(17));
        assert!(!reported.contains(12));
    }

    #[test]
    fn rt_sigpending_to_a_bad_pointer_is_efault() {
        use super::rt_sigpending;
        let st = SignalState::new();
        let mut buf = [0u8; 4]; // too short for an 8-byte mask
        let mut mem = region(&mut buf);
        assert_eq!(
            rt_sigpending(&st, &mut mem, 0x1000),
            Err(SigError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn rt_sigaction_installs_and_reports_a_disposition() {
        use super::rt_sigaction;
        let mut st = SignalState::new();
        // A new handler for signal 11 written at guest 0x1000.
        let act = SigAction {
            handler: 0x1_4000_7000,
            flags: 4,
            restorer: 0,
            mask: Sigset::empty(),
        };
        let mut buf = [0u8; 64];
        buf[0..32].copy_from_slice(&act.to_guest_bytes());
        let mut mem = region(&mut buf);
        // act=0x1000, oldact=0x1020 — install the new action, report the old.
        rt_sigaction(&mut st, &mut mem, 11, 0x1000, 0x1020).expect("ok");
        assert_eq!(st.action(11), Some(act));
        // oldact holds the previous (default) disposition.
        let old = SigAction::from_guest_bytes(&buf[32..64]).unwrap();
        assert_eq!(old, SigAction::default());
    }

    #[test]
    fn rt_sigaction_rejects_kill_and_invalid_signals() {
        use super::rt_sigaction;
        let mut st = SignalState::new();
        let act = SigAction::default();
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&act.to_guest_bytes());
        let mut mem = region(&mut buf);
        // SIGKILL (9) disposition can't be changed -> EINVAL, no effect.
        assert_eq!(
            rt_sigaction(&mut st, &mut mem, 9, 0x1000, 0),
            Err(SigError::BadSignal(9))
        );
        // An out-of-range signal is EINVAL.
        assert_eq!(
            rt_sigaction(&mut st, &mut mem, 99, 0x1000, 0),
            Err(SigError::BadSignal(99))
        );
        // A bad act pointer is EFAULT.
        assert_eq!(
            rt_sigaction(&mut st, &mut mem, 11, 0x1010, 0),
            Err(SigError::Fault(RangeError::Unmapped))
        );
    }

    #[test]
    fn send_signal_raises_a_valid_signal_pending() {
        let mut st = SignalState::new();
        // A normal signal becomes pending.
        send_signal(&mut st, 11).expect("valid signal");
        assert!(st.pending_set().contains(11));
        // SIGKILL can be *sent* (only blocking/recatching it is forbidden).
        send_signal(&mut st, 9).expect("sigkill is sendable");
        assert!(st.pending_set().contains(9));
        // A second signal accumulates without disturbing the first.
        send_signal(&mut st, 17).expect("another");
        assert!(st.pending_set().contains(11) && st.pending_set().contains(17));
    }

    #[test]
    fn send_signal_zero_is_a_noop_and_out_of_range_is_einval() {
        let mut st = SignalState::new();
        // sig 0 is the existence-probe: it sends nothing.
        send_signal(&mut st, 0).expect("sig 0 is a no-op");
        assert_eq!(st.pending_count(), 0);
        // Signals past the 64-signal range are rejected.
        assert_eq!(send_signal(&mut st, 65), Err(SigError::BadSignal(65)));
        assert_eq!(send_signal(&mut st, 1000), Err(SigError::BadSignal(1000)));
        assert_eq!(st.pending_count(), 0);
    }
}
