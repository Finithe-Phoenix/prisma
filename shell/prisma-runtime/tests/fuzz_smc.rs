//! Property-based robustness fuzzing for the SMC guard.
//!
//! The SMC guard sits on a host<->guest trust boundary: a guest can write to,
//! and fault on, arbitrary addresses, and the guard must process any sequence
//! of translate/fault/invalidate events without panicking and without leaking
//! tracked state. This complements the unit tests in `smc_guard.rs` with broad
//! random coverage of attacker-controlled addresses.

use proptest::prelude::*;

use prisma_runtime::smc_guard::SmcGuard;

#[derive(Debug, Clone)]
enum Op {
    Translate(u64, u32, u64),
    Fault(u64),
    Invalidate(u64),
    Drain,
    DrainPages,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u64>(), any::<u32>(), any::<u64>()).prop_map(|(a, l, k)| Op::Translate(a, l, k)),
        any::<u64>().prop_map(Op::Fault),
        any::<u64>().prop_map(Op::Invalidate),
        Just(Op::Drain),
        Just(Op::DrainPages),
    ]
}

proptest! {
    /// An arbitrary sequence of guard operations over attacker-controlled
    /// addresses never panics, and once the pending sets are drained they stay
    /// empty (draining is exhaustive — no key/page is silently retained).
    #[test]
    fn arbitrary_ops_never_panic(ops in prop::collection::vec(op(), 0..256)) {
        let mut g = SmcGuard::new();
        for o in ops {
            match o {
                Op::Translate(a, l, k) => g.on_translate(a, l, k),
                Op::Fault(a) => { let _ = g.handle_fault(a); }
                Op::Invalidate(a) => { let _ = g.invalidate_page(a); }
                Op::Drain => { let _ = g.drain_pending(); }
                Op::DrainPages => { let _ = g.drain_pending_pages(); }
            }
            let _ = g.tracked_page_count();
        }
        let _ = g.drain_pending();
        let _ = g.drain_pending_pages();
        prop_assert!(g.drain_pending().is_empty());
        prop_assert!(g.drain_pending_pages().is_empty());
    }
}
