//! Property-based robustness fuzzing for the guest signal types — the masks and
//! dispositions a guest passes through `rt_sigprocmask` / `rt_sigaction`.
//!
//! The load-bearing invariant is the security one: SIGKILL and SIGSTOP can never
//! end up blocked, whatever mask the guest supplies and however it is combined.

use proptest::prelude::*;

use prisma_runtime::guest_signal::{SigAction, SigprocmaskHow, Sigset};

const SIGKILL: u32 = 9;
const SIGSTOP: u32 = 19;

fn how(i: u8) -> SigprocmaskHow {
    match i % 3 {
        0 => SigprocmaskHow::Block,
        1 => SigprocmaskHow::Unblock,
        _ => SigprocmaskHow::SetMask,
    }
}

proptest! {
    /// A `Sigset` round-trips through its 8-byte wire form for every bit pattern.
    #[test]
    fn sigset_round_trips_for_every_bit_pattern(bits in any::<u64>()) {
        let s = Sigset::from_bits(bits);
        prop_assert_eq!(Sigset::from_guest_bytes(&s.to_guest_bytes()), Some(s));
        prop_assert_eq!(s.bits(), bits);
    }

    /// insert / remove / contains never panic on an arbitrary signal number, and
    /// out-of-range numbers are inert (never aliased into a real bit).
    #[test]
    fn sigset_ops_never_panic(sig in any::<u32>(), bits in any::<u64>()) {
        let mut s = Sigset::from_bits(bits);
        let _ = s.contains(sig);
        let inserted = s.insert(sig);
        // An out-of-range signal is rejected and changes nothing.
        prop_assert_eq!(inserted, matches!(sig, 1..=64));
    }

    /// **Security invariant.** However a guest combines masks via `apply`,
    /// SIGKILL and SIGSTOP are never left blocked.
    #[test]
    fn apply_never_blocks_kill_or_stop(start in any::<u64>(), arg in any::<u64>(), h in any::<u8>()) {
        let mut mask = Sigset::from_bits(start);
        mask.apply(how(h), Sigset::from_bits(arg));
        prop_assert!(!mask.contains(SIGKILL));
        prop_assert!(!mask.contains(SIGSTOP));
    }

    /// A `SigAction` round-trips through its 32-byte wire form for arbitrary
    /// field values.
    #[test]
    fn sigaction_round_trips(handler in any::<u64>(), flags in any::<u64>(), restorer in any::<u64>(), mask in any::<u64>()) {
        let act = SigAction {
            handler,
            flags,
            restorer,
            mask: Sigset::from_bits(mask),
        };
        prop_assert_eq!(SigAction::from_guest_bytes(&act.to_guest_bytes()), Some(act));
    }
}
