//! Hybrid e2e: a synthetic x86-64 PE goes through the Rust loader
//! (`prisma_orchestrator::pe_loader`) and is executed by the C++ DBT
//! through the FFI bridge. This is the RFC 0014 milestone test — the
//! first time both halves of the project run one program together.
//!
//! Guest code layout: a PE32+ with one .text section at RVA 0x1000
//! whose raw data is the guest program. The dispatcher fetches from
//! the mapped image exactly the way it will fetch from Wine-loaded
//! binaries in Fase 4.

use prisma_core::{BlockExitKind, DispatchExit, Dispatcher, Gpr, GuestImage, Translator};
use prisma_orchestrator::pe_loader::{map_image, parse, PeMachine};

/// movabs rax, 42 ; ret
const GUEST_PROGRAM: [u8; 11] = [
    0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];

/// Minimal PE32+ (DOS stub → NT headers → one .text section) with
/// `code` as the section's raw data at RVA 0x1000, entry RVA 0x1000,
/// image base `0x1_4000_0000`. Mirrors the orchestrator's unit-test
/// builder; duplicated here because that one is `#[cfg(test)]`.
fn synth_pe(code: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
    buf[64..68].copy_from_slice(b"PE\0\0");
    let coff = 64 + 4;
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
    let raw_size = u32::try_from(code.len()).unwrap();
    buf[sec + 16..sec + 20].copy_from_slice(&raw_size.to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

#[test]
fn pe_text_section_translates_through_the_bridge() {
    let file = synth_pe(&GUEST_PROGRAM);

    let pe = parse(&file).expect("PE parse");
    assert_eq!(pe.machine, PeMachine::X86_64);
    let mapped = map_image(&pe, &file).expect("PE map");
    assert_eq!(mapped.entry_pc, 0x1_4000_1000);

    let mut translator = Translator::new().expect("translator");
    let entry_off = usize::try_from(mapped.entry_pc - mapped.base).unwrap();
    let info = translator
        .translate(mapped.entry_pc, &mapped.bytes[entry_off..])
        .expect("translate PE entry block");
    assert_eq!(info.guest_size, GUEST_PROGRAM.len() as u64);
    assert_eq!(info.exit_kind, BlockExitKind::RetAdjusted);
}

#[test]
fn pe_executes_end_to_end_on_arm64() {
    if !cfg!(target_arch = "aarch64") {
        // Translation coverage runs in the test above; execution
        // requires an ARM64 host, same gate as the C++ e2e corpus.
        return;
    }

    let file = synth_pe(&GUEST_PROGRAM);
    let pe = parse(&file).expect("PE parse");
    let mapped = map_image(&pe, &file).expect("PE map");

    let mut translator = Translator::new().expect("translator");
    let image = GuestImage::new(mapped.base, mapped.bytes.clone());
    let mut dispatcher = Dispatcher::new(&mut translator, &image).expect("dispatcher");
    dispatcher.install_halt_return_stack().expect("halt stack");

    let outcome = dispatcher.run(mapped.entry_pc, 64).expect("run");
    assert_eq!(outcome.exit, DispatchExit::Halted);
    assert_eq!(outcome.stats.blocks_executed, 1);
    assert_eq!(dispatcher.gpr(Gpr::Rax).expect("rax"), 42);
}
