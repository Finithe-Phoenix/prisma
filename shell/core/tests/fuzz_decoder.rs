//! Property-based fuzzing of the decoder/translator through the FFI
//! bridge (RFC 0014). Complements the AFL++ harness in `fuzz/`: AFL
//! explores coverage-guided over hours, proptest gives every PR a
//! fast randomized sweep with shrinking on failure.
//!
//! The properties are about robustness, not correctness: any byte
//! stream must come back as Ok or a typed error — never a crash,
//! never UB (the C++ side runs under ASan/UBSan in its own CI jobs;
//! here we assert the boundary contract holds for hostile input).
//!
//! Translation only — nothing here executes guest code, so the suite
//! runs identically on every host.

use prisma_core::Translator;
use proptest::prelude::*;

const BASE: u64 = 0x40_0000;

/// One x86-64-shaped instruction: optional legacy/REX prefixes, an
/// opcode (sometimes 0F-escaped), a `ModRM` byte, and a few trailing
/// bytes that read as SIB/displacement/immediate. Biased toward the
/// decoder's dispatch paths without being a real assembler.
fn instruction_shaped() -> impl Strategy<Value = Vec<u8>> {
    let legacy_prefix = proptest::sample::select(vec![0x66u8, 0xF2, 0xF3, 0x67, 0x2E, 0x65]);
    let rex = 0x40u8..=0x4F;
    (
        proptest::collection::vec(legacy_prefix, 0..3),
        proptest::option::of(rex),
        proptest::bool::ANY,
        any::<u8>(),
        any::<u8>(),
        proptest::collection::vec(any::<u8>(), 0..8),
    )
        .prop_map(|(prefixes, rex, escape, opcode, modrm, tail)| {
            let mut bytes = prefixes;
            if let Some(r) = rex {
                bytes.push(r);
            }
            if escape {
                bytes.push(0x0F);
            }
            bytes.push(opcode);
            bytes.push(modrm);
            bytes.extend_from_slice(&tail);
            bytes
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Fully arbitrary byte streams: the decoder sees pure noise.
    #[test]
    fn arbitrary_bytes_return_ok_or_typed_error(
        bytes in proptest::collection::vec(any::<u8>(), 1..256),
    ) {
        let mut t = Translator::new().expect("translator");
        match t.translate(BASE, &bytes) {
            Ok(info) => {
                // A successful translation consumed at least one guest
                // byte, no more than we supplied, and produced code.
                prop_assert!(info.guest_size >= 1);
                prop_assert!(info.guest_size <= bytes.len() as u64);
                prop_assert!(info.code_size > 0);
            }
            Err(e) => {
                // Typed error is the contract; reaching here at all
                // means no crash and no boundary violation.
                let _ = e;
            }
        }
    }

    /// Instruction-shaped streams reach much deeper dispatch paths
    /// (prefix handling, 0F escapes, ModRM/SIB decoding).
    #[test]
    fn instruction_shaped_streams_never_crash(
        instrs in proptest::collection::vec(instruction_shaped(), 1..6),
    ) {
        let bytes: Vec<u8> = instrs.concat();
        let mut t = Translator::new().expect("translator");
        let _ = t.translate(BASE, &bytes);
    }

    /// Translation is deterministic and memoised: translating the
    /// same bytes at the same address twice yields the same block
    /// shape, with the second hit served from the cache.
    #[test]
    fn translation_is_deterministic_and_cached(
        instrs in proptest::collection::vec(instruction_shaped(), 1..4),
    ) {
        let bytes: Vec<u8> = instrs.concat();
        let mut t = Translator::new().expect("translator");
        let first = t.translate(BASE, &bytes);
        let second = t.translate(BASE, &bytes);
        match (first, second) {
            (Ok(a), Ok(b)) => {
                prop_assert!(!a.from_cache);
                prop_assert!(b.from_cache);
                prop_assert_eq!(a.guest_size, b.guest_size);
                prop_assert_eq!(a.exit_kind, b.exit_kind);
                prop_assert_eq!(a.code_size, b.code_size);
            }
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            (a, b) => {
                return Err(TestCaseError::fail(format!(
                    "non-deterministic translate: {a:?} then {b:?}"
                )));
            }
        }
    }
}
