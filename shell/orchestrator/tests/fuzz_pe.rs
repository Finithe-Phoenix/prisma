//! Property-based robustness fuzzing for the PE loader.
//!
//! A loaded `.exe` is fully untrusted input crossing the host<->guest
//! boundary. These properties assert the parser never panics on arbitrary or
//! mutated bytes, and that the parse -> map pipeline stays robust. The
//! `size_of_image` field is attacker-controlled and drives `map_image`'s
//! up-front allocation, so the mutated-PE property clamps it to a safe range
//! here (the unbounded-allocation hazard is fixed and tested separately in the
//! loader itself); this harness exercises the section-bounds logic without
//! reproducing that allocation.

use proptest::prelude::*;

use prisma_orchestrator::pe_loader::{map_image, parse};

/// Minimal valid PE32+ (DOS stub -> NT -> COFF -> optional header -> 1
/// section) that `parse` accepts. The fuzz mutates copies of this so parses
/// frequently succeed and exercise the deeper section/map logic.
fn base_pe() -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
    buf[64..68].copy_from_slice(b"PE\0\0");
    let coff = 68;
    buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes()); // machine
    buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
    let opt = coff + 20;
    buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes()); // PE32+
    buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry RVA
    buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes()); // image_base
    buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes()); // SizeOfImage
    let sec = opt + 240;
    buf[sec..sec + 5].copy_from_slice(b".text");
    buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual_size
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual_address
    buf
}

/// 16 MiB — well above the synthetic image, well below a memory blow-up. Keeps
/// the fuzz's own `map_image` calls bounded while the real cap is tested apart.
const FUZZ_MAP_CLAMP: u32 = 16 << 20;

proptest! {
    /// `parse` never panics on arbitrary bytes.
    #[test]
    fn parse_arbitrary_bytes_never_panics(raw in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse(&raw);
    }

    /// Parsing a bit-mutated valid PE never panics, and mapping whatever
    /// parsed stays robust (with the allocation clamped for the harness).
    #[test]
    fn mutated_pe_parse_and_map_never_panic(
        mutations in prop::collection::vec((any::<usize>(), any::<u8>()), 0..96),
    ) {
        let mut buf = base_pe();
        let n = buf.len();
        for (idx, val) in &mutations {
            buf[*idx % n] = *val;
        }
        if let Ok(img) = parse(&buf) {
            if img.size_of_image <= FUZZ_MAP_CLAMP {
                let _ = map_image(&img, &buf);
            }
        }
    }

    /// Appending random trailing bytes to a valid PE never destabilises parse
    /// or map (sections may now point into the tail).
    #[test]
    fn valid_pe_with_random_tail_is_robust(tail in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut buf = base_pe();
        buf.extend_from_slice(&tail);
        if let Ok(img) = parse(&buf) {
            if img.size_of_image <= FUZZ_MAP_CLAMP {
                let _ = map_image(&img, &buf);
            }
        }
    }
}
