//! F2-SY-037 — syscall fuzz harness.
//!
//! Property fuzzing over arbitrary syscall numbers AND argument vectors,
//! asserting the invariants the dispatch layer must hold for every input. The
//! load-bearing one is the security invariant: a deny-by-default policy must
//! never admit an unmodelled syscall, whatever its number or arguments.

use prisma_runtime::syscall_handler::{
    classify, is_known, SyscallClass, SyscallError, SyscallHandler, SyscallPolicy,
};
use proptest::prelude::*;

/// Every modelled syscall number (mirrors `classify`'s registry). Sampling from
/// this set is how the "known number" properties get a known input — a random
/// `u32` lands on one of these ~40 values far too rarely to drive a proptest.
const KNOWN: &[u32] = &[
    0, 1, 2, 3, 4, 5, 6, 8, 16, 17, 18, 19, 20, 32, 33, 72, 257, 262, // Io
    9, 10, 11, 12, 25, 28, // Memory
    39, 60, 61, 110, 111, 186, 231, // Process
    13, 14, 15, 127, 128, // Signal
    56, 58, 158, 202, 218, 273, // Thread
];

proptest! {
    // classify is total — it returns for every u32 without panicking — and
    // is_known is exactly "classified into a real category".
    #[test]
    fn classify_is_total_and_consistent_with_is_known(number in any::<u32>()) {
        let class = classify(number);
        prop_assert_eq!(is_known(number), class != SyscallClass::Unknown);
    }

    // Security invariant: under deny-by-default an UNKNOWN number is always
    // rejected as Unsupported(number) — never admitted — for any arguments.
    #[test]
    fn deny_by_default_never_admits_unknown(
        number in any::<u32>(),
        args in any::<[u64; 6]>(),
    ) {
        prop_assume!(!is_known(number));
        let handler = SyscallHandler::new(); // deny-by-default
        prop_assert_eq!(
            handler.dispatch(number, &args),
            Err(SyscallError::Unsupported(number))
        );
    }

    // A modelled syscall is admitted regardless of policy or arguments (the
    // deny switch only governs UNKNOWN numbers).
    #[test]
    fn known_syscalls_are_admitted_under_any_policy(
        number in prop::sample::select(KNOWN),
        args in any::<[u64; 6]>(),
    ) {
        prop_assert!(is_known(number));
        for policy in [SyscallPolicy::deny_by_default(), SyscallPolicy::allow_by_default()] {
            let handler = SyscallHandler::with_policy(policy);
            prop_assert!(handler.dispatch(number, &args).is_ok());
        }
    }

    // allow-by-default never reports Unsupported — it admits every number.
    #[test]
    fn allow_by_default_admits_everything(
        number in any::<u32>(),
        args in any::<[u64; 6]>(),
    ) {
        let handler = SyscallHandler::with_policy(SyscallPolicy::allow_by_default());
        prop_assert!(handler.dispatch(number, &args).is_ok());
    }

    // getpid (39) / gettid (186) report the host pid for any args/policy — the
    // guest runs inside the host process, so that is the value it must observe.
    #[test]
    fn identity_calls_report_host_pid(args in any::<[u64; 6]>(), deny in any::<bool>()) {
        let policy = if deny {
            SyscallPolicy::deny_by_default()
        } else {
            SyscallPolicy::allow_by_default()
        };
        let handler = SyscallHandler::with_policy(policy);
        let pid = u64::from(std::process::id());
        prop_assert_eq!(handler.dispatch(39, &args), Ok(pid));
        prop_assert_eq!(handler.dispatch(186, &args), Ok(pid));
    }

    // dispatch never panics on any (number, args, policy) — the whole point of
    // fuzzing the host<->guest syscall boundary.
    #[test]
    fn dispatch_never_panics(
        number in any::<u32>(),
        args in any::<[u64; 6]>(),
        deny in any::<bool>(),
    ) {
        let policy = if deny {
            SyscallPolicy::deny_by_default()
        } else {
            SyscallPolicy::allow_by_default()
        };
        let handler = SyscallHandler::with_policy(policy);
        let _ = handler.dispatch(number, &args);
    }
}
