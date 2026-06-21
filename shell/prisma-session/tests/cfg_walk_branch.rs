//! M2 reachable-CFG walk across a real static branch.
//!
//! The unit tests cover a single block ending on RET; this exercises the walk's
//! reason for existing — following a control transfer. A `.text` whose entry
//! jumps forward to a second block (ending in RET) must make
//! `translate_reachable` visit BOTH blocks: it translates the entry, reads the
//! `JMP rel8`'s static target from the block's successors, and translates the
//! target too.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::Session;

/// A minimal PE32+ whose file-backed `.text` holds `code`, mapped at the entry.
fn pe_with_code(code: &[u8]) -> Vec<u8> {
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
    let raw_off = u32::try_from(buf.len()).unwrap();
    buf[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

#[test]
fn reachable_walk_follows_a_forward_jmp_to_a_second_block() {
    // .text layout (entry RVA 0x1000):
    //   0x00: EB 0E      JMP +0x0E  -> targets 0x02 + 0x0E = 0x10
    //   0x02..0x10: padding (skipped by the jump)
    //   0x10: C3         RET
    let mut code = vec![0u8; 0x11];
    code[0] = 0xEB; // JMP rel8
    code[1] = 0x0E; // -> entry + 0x10
    code[0x10] = 0xC3; // RET

    let entry = 0x1_4000_1000u64;
    let s = Session::load(&pe_with_code(&code), &ModuleTable::new()).expect("load");
    let visited = s.reachable_blocks(16);

    // Both the entry block and the JMP's target block are translated, in that
    // order — the walk followed the static branch.
    assert_eq!(visited, vec![entry, entry + 0x10]);
}

#[test]
fn a_self_jump_terminates_via_the_seen_set() {
    // EB FE = JMP -2 -> targets itself (a tight infinite loop at run time). The
    // static walk must visit it exactly once and stop (the seen-set dedups the
    // self-edge), never looping forever.
    let code = vec![0xEBu8, 0xFE];
    let entry = 0x1_4000_1000u64;
    let s = Session::load(&pe_with_code(&code), &ModuleTable::new()).expect("load");
    let visited = s.reachable_blocks(16);
    assert_eq!(visited, vec![entry]);
}
