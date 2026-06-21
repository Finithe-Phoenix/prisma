//! Property-fuzz the JIT executable-memory pool's accounting and release.
//!
//! The resource-discipline clause requires that W^X executable mappings are
//! freed deterministically — a leak must never survive a restart. The unit
//! tests check one install/clear sequence; this drives *arbitrary* interleavings
//! of install and clear and asserts the two invariants that rule out a leak:
//!
//! * **Exact accounting** — after every operation `mapped_bytes()` equals the
//!   sum of the page-rounded capacities the pool actually handed out (no
//!   under- or over-count), and the budget is always page-aligned;
//! * **Deterministic release** — `clear()` drops the byte budget and the buffer
//!   count to zero regardless of the preceding history.

use prisma_runtime::jit_memory::ExecPool;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum PoolOp {
    /// Install `n` bytes of (dummy) code as a fresh executable buffer.
    Add(usize),
    /// Release every installed buffer.
    Clear,
}

fn op_strategy() -> impl Strategy<Value = PoolOp> {
    prop_oneof![
        4 => (1usize..256).prop_map(PoolOp::Add),
        1 => Just(PoolOp::Clear),
    ]
}

proptest! {
    #[test]
    fn exec_pool_accounting_is_exact_and_clear_always_releases(
        ops in prop::collection::vec(op_strategy(), 0..24)
    ) {
        let mut pool = ExecPool::new(64);
        // Shadow model: the byte budget and count the pool *should* report.
        let mut expected_bytes = 0usize;
        let mut expected_len = 0usize;

        for op in &ops {
            match op {
                PoolOp::Add(n) => {
                    let code = vec![0x90u8; *n]; // `n` nops — never empty (n >= 1)
                    if let Some(alloc) = pool.add(&code) {
                        // The pool's own page-rounded capacity is the truth.
                        expected_bytes += alloc.capacity;
                        expected_len += 1;
                    }
                }
                PoolOp::Clear => {
                    pool.clear();
                    expected_bytes = 0;
                    expected_len = 0;
                }
            }

            // Exact accounting after every operation — no leak, no double-count.
            prop_assert_eq!(pool.mapped_bytes(), expected_bytes);
            prop_assert_eq!(pool.len(), expected_len);
            prop_assert_eq!(pool.is_empty(), expected_len == 0);
            // Each buffer is page-rounded, so the budget is always page-aligned.
            prop_assert_eq!(pool.mapped_bytes() % 4096, 0);
        }

        // Deterministic release: a final clear frees everything, whatever the
        // history was — the property a restart relies on.
        pool.clear();
        prop_assert_eq!(pool.mapped_bytes(), 0);
        prop_assert!(pool.is_empty());
        // The pool is reusable after an explicit release.
        prop_assert!(pool.add(&[0x90, 0xC3]).is_some());
        prop_assert_eq!(pool.len(), 1);
    }
}
