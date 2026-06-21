//! Property-based robustness fuzzing for the full `load_pe` driver.
//!
//! `load_pe` chains the whole untrusted-input pipeline — parse, map, relocate,
//! resolve imports, patch the IAT — over bytes that come straight off disk. It
//! must reject malformed input with a typed error, never panic, on any bytes a
//! crafted or corrupt `.exe` could present. These properties assert exactly
//! that over arbitrary and mutated inputs.

use proptest::prelude::*;

use prisma_orchestrator::load_pe::load_pe;
use prisma_orchestrator::module_table::ModuleTable;

/// Minimal valid PE32+ (no imports) the mutation fuzz perturbs so parses
/// frequently succeed and exercise the deeper map/reloc/IAT logic.
fn base_pe() -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
    buf[64..68].copy_from_slice(b"PE\0\0");
    let coff = 68;
    buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let opt = coff + 20;
    buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
    buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes());
    let sec = opt + 240;
    buf[sec..sec + 5].copy_from_slice(b".text");
    buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    buf
}

proptest! {
    /// `load_pe` never panics on arbitrary bytes — it returns Ok or a typed
    /// LoadError, never crashes the loader.
    #[test]
    fn load_pe_on_arbitrary_bytes_never_panics(raw in prop::collection::vec(any::<u8>(), 0..4096)) {
        let modules = ModuleTable::new();
        let _ = load_pe(&raw, &modules);
    }

    /// A bit-mutated valid PE never panics: whatever parses then drives the
    /// map/reloc/IAT path, and a malformed field errors rather than crashing.
    #[test]
    fn mutated_pe_never_panics(
        mutations in prop::collection::vec((any::<usize>(), any::<u8>()), 0..64),
    ) {
        let mut buf = base_pe();
        let n = buf.len();
        for (idx, val) in &mutations {
            buf[*idx % n] = *val;
        }
        let modules = ModuleTable::new();
        let _ = load_pe(&buf, &modules);
    }

    /// Appending arbitrary trailing bytes to a valid PE never destabilises the
    /// driver (sections / directories may now point into the tail).
    #[test]
    fn valid_pe_with_random_tail_is_robust(tail in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut buf = base_pe();
        buf.extend_from_slice(&tail);
        let modules = ModuleTable::new();
        // The unmodified base PE has no imports, so this should still load.
        let _ = load_pe(&buf, &modules);
    }

    /// Truncating a valid PE at any length never panics — every short read is
    /// bounds-checked into a typed error.
    #[test]
    fn truncated_pe_never_panics(cut in 0usize..400) {
        let buf = base_pe();
        let end = cut.min(buf.len());
        let modules = ModuleTable::new();
        let _ = load_pe(&buf[..end], &modules);
    }
}
