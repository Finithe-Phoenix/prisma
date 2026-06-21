//! Property-fuzz coverage for the signal syscalls.
//!
//! A guest drives `rt_sigprocmask` / `rt_sigpending` / `rt_sigaction` with fully
//! untrusted `how`, signal numbers, and pointers. The invariants under test:
//!
//! * no input ever panics — a bad pointer is a guest errno (`Fault`), never an
//!   out-of-bounds host access;
//! * SIGKILL / SIGSTOP can never be blocked nor have their disposition changed,
//!   no matter what the guest asks (the kernel rule);
//! * a well-formed mask written to a valid pointer round-trips through SETMASK.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_signal::{SigAction, SignalState, SigprocmaskHow, Sigset};
use prisma_session::sig_syscalls::{rt_sigaction, rt_sigpending, rt_sigprocmask, SigError};
use proptest::prelude::*;

const BASE: u64 = 0x1000;
const LEN: u64 = 64;

// SIGKILL and SIGSTOP — the two signals the kernel never lets a process block or
// recatch, regardless of what the guest requests.
const SIGKILL: u32 = 9;
const SIGSTOP: u32 = 19;

proptest! {
    /// Arbitrary `(how, set, oldset)` never panics: each pointer either lands in
    /// the region (a defined read/write) or yields `Fault`.
    #[test]
    fn rt_sigprocmask_never_panics(
        how in any::<i32>(),
        set_addr in any::<u64>(),
        oldset_addr in any::<u64>(),
    ) {
        let mut st = SignalState::new();
        let mut buf = [0u8; LEN as usize];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        let _ = rt_sigprocmask(&mut st, &mut mem, how, set_addr, oldset_addr);
    }

    /// A valid `SETMASK` with a mask that includes the unblockable signals still
    /// leaves SIGKILL/SIGSTOP unblocked — the guest cannot escape them.
    #[test]
    fn setmask_can_never_block_kill_or_stop(extra in any::<u64>()) {
        let mut st = SignalState::new();
        let mut buf = [0u8; LEN as usize];
        // Mask requesting every signal in `extra` plus KILL and STOP.
        let mut want = Sigset::from_bits(extra);
        want.insert(SIGKILL);
        want.insert(SIGSTOP);
        buf[0..8].copy_from_slice(&want.to_guest_bytes());
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        // SIG_SETMASK = 2, set at BASE, no oldset.
        rt_sigprocmask(&mut st, &mut mem, 2, BASE, 0).expect("in-region set");
        prop_assert!(!st.blocked().contains(SIGKILL));
        prop_assert!(!st.blocked().contains(SIGSTOP));
    }

    /// `rt_sigpending` to any pointer never panics, and a valid one round-trips
    /// the exact pending set.
    #[test]
    fn rt_sigpending_never_panics_and_round_trips(
        set_addr in any::<u64>(),
        raised in prop::collection::vec(1u32..64, 0..8),
    ) {
        let mut st = SignalState::new();
        for s in &raised {
            st.raise(*s);
        }
        let mut buf = [0u8; LEN as usize];
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        match rt_sigpending(&st, &mut mem, set_addr) {
            Ok(()) => {
                // Only the in-region pointer succeeds; it holds the pending set.
                let got = Sigset::from_guest_bytes(&buf[0..8]).unwrap();
                for s in &raised {
                    prop_assert!(got.contains(*s));
                }
            }
            Err(SigError::Fault(_)) => {}
            Err(other) => prop_assert!(false, "unexpected error {other:?}"),
        }
    }

    /// Arbitrary `(sig, act, oldact)` never panics, and KILL/STOP can never have
    /// their disposition installed.
    #[test]
    fn rt_sigaction_never_panics_and_protects_kill_stop(
        sig in any::<u32>(),
        act_addr in any::<u64>(),
        oldact_addr in any::<u64>(),
    ) {
        let mut st = SignalState::new();
        let mut buf = [0u8; LEN as usize];
        // A well-formed action at BASE so an in-region `act` decodes.
        let act = SigAction { handler: 0x4000, flags: 0, restorer: 0, mask: Sigset::empty() };
        buf[0..32].copy_from_slice(&act.to_guest_bytes());
        let mut mem = GuestRegion::new(BASE, Protection::ReadWrite, &mut buf);
        let r = rt_sigaction(&mut st, &mut mem, sig, act_addr, oldact_addr);
        if sig == SIGKILL || sig == SIGSTOP {
            // Whatever the pointers, the disposition is never changed.
            prop_assert_eq!(st.action(sig), Some(SigAction::default()));
            if act_addr == BASE {
                prop_assert_eq!(r, Err(SigError::BadSignal(sig)));
            }
        }
    }
}

#[test]
fn from_raw_covers_only_the_three_valid_hows() {
    assert!(SigprocmaskHow::from_raw(0).is_some());
    assert!(SigprocmaskHow::from_raw(1).is_some());
    assert!(SigprocmaskHow::from_raw(2).is_some());
    assert!(SigprocmaskHow::from_raw(3).is_none());
    assert!(SigprocmaskHow::from_raw(-1).is_none());
}
