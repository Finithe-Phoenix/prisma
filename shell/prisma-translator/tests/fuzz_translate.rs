//! Property-based robustness fuzzing for the translation facade.
//!
//! The translator is the public entry point that consumes untrusted guest
//! bytes end to end (decode -> optimize -> lower -> cache), so the hard
//! invariant is that `translate`/`translate_block` terminate with
//! `Ok`/`Err`/a bounded block on any input — they never panic — and never
//! claim to cover bytes they did not decode. Closes the robustness-fuzz set
//! at the integrated level, above the per-crate decoder/passes/backend
//! harnesses.

use proptest::prelude::*;

use prisma_translator::Translator;

proptest! {
    /// Single-instruction translation never panics on arbitrary bytes.
    #[test]
    fn translate_never_panics(
        addr in any::<u64>(),
        bytes in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let mut t = Translator::new();
        let _ = t.translate(addr, &bytes);
    }

    /// Block translation never panics and never reports covering more bytes
    /// than it was given.
    #[test]
    fn translate_block_never_panics_and_stays_in_bounds(
        addr in any::<u64>(),
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        cap in 0usize..32,
    ) {
        let mut t = Translator::new();
        if let Ok(block) = t.translate_block(addr, &bytes, cap) {
            prop_assert!(block.guest_bytes <= bytes.len());
            prop_assert!(block.instruction_count <= cap);
        }
    }

    /// Translation is deterministic: two fresh translators agree on the result
    /// for identical input.
    #[test]
    fn translate_is_deterministic(
        addr in any::<u64>(),
        bytes in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let mut a = Translator::new();
        let mut b = Translator::new();
        prop_assert_eq!(a.translate(addr, &bytes), b.translate(addr, &bytes));
    }

    /// Fused-block translation (with SSA renumbering across instructions) never
    /// panics on arbitrary bytes and stays within its byte / instruction budget.
    #[test]
    fn translate_fused_block_never_panics_and_stays_in_bounds(
        addr in any::<u64>(),
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        cap in 0usize..32,
    ) {
        let mut t = Translator::new();
        if let Ok(block) = t.translate_fused_block(addr, &bytes, cap) {
            prop_assert!(block.guest_bytes <= bytes.len());
            prop_assert!(block.instruction_count <= cap);
        }
    }
}
