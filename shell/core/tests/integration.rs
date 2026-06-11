//! Cross-language integration tests: Rust driving the C++ DBT core
//! through the C ABI (RFC 0014). Mirrors `core/tests/test_capi.cpp` so
//! a drift between capi.h and the bindings fails here first.
//!
//! JIT execution is gated on aarch64 exactly like the C++ e2e corpus;
//! translation and dispatch error paths run on every host.

use prisma_core::{
    capi_version, BlockExitKind, CoreError, DispatchExit, Dispatcher, Gpr, GuestImage, Translator,
};

/// movabs rax, 42 ; ret
const MOV_RAX_42_RET: [u8; 11] = [
    0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];

const BASE: u64 = 0x1000;

#[test]
fn abi_version_matches_bindings() {
    assert_eq!(capi_version(), prisma_core_sys::PRISMA_CAPI_VERSION);
}

#[test]
fn translate_reports_block_info_and_cache_hits() {
    let mut t = Translator::new().expect("translator");

    let info = t.translate(BASE, &MOV_RAX_42_RET).expect("translate");
    assert_eq!(info.guest_size, MOV_RAX_42_RET.len() as u64);
    assert!(info.code_size > 0);
    assert_eq!(info.exit_kind, BlockExitKind::RetAdjusted);
    assert!(!info.from_cache);

    let again = t.translate(BASE, &MOV_RAX_42_RET).expect("re-translate");
    assert!(again.from_cache);

    let stats = t.stats().expect("stats");
    assert_eq!(stats.translations_attempted, 2);
    assert_eq!(stats.cache_misses, 1);
    assert_eq!(stats.cache_hits, 1);
}

#[test]
fn empty_input_maps_to_typed_error() {
    let mut t = Translator::new().expect("translator");
    assert_eq!(t.translate(BASE, &[]), Err(CoreError::EmptyInput));
}

#[test]
fn undecodable_input_maps_to_typed_error() {
    let mut t = Translator::new().expect("translator");
    // 0xC7 (MOV r/m, imm32) is outside the decoder's supported set.
    let bytes = [0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, 0xC3];
    assert_eq!(t.translate(BASE, &bytes), Err(CoreError::DecodeFailed));
}

#[test]
fn legacy_ret_exit_shape() {
    let mut t = Translator::new().expect("translator");
    t.set_real_call_ret(false).expect("toggle");
    let info = t.translate(BASE, &MOV_RAX_42_RET).expect("translate");
    assert_eq!(info.exit_kind, BlockExitKind::Return);
}

#[test]
fn dispatcher_halts_at_entry_without_executing() {
    let mut t = Translator::new().expect("translator");
    let image = GuestImage::new(BASE, MOV_RAX_42_RET.to_vec());
    let mut d = Dispatcher::new(&mut t, &image).expect("dispatcher");
    d.add_halt_pc(BASE).expect("halt pc");

    let outcome = d.run(BASE, 16).expect("run");
    assert_eq!(outcome.exit, DispatchExit::Halted);
    assert_eq!(outcome.final_pc, BASE);
    assert_eq!(outcome.stats.blocks_executed, 0);
    assert_eq!(outcome.stats.direct_jit_patch_attempts, 0);
    assert_eq!(outcome.stats.direct_jit_patch_applied, 0);
    assert_eq!(outcome.stats.direct_jit_patch_rejected, 0);
    assert_eq!(outcome.stats.direct_jit_patch_unpatches, 0);
    assert_eq!(outcome.stats.direct_jit_patch_executes, 0);
    assert!(outcome.message.is_empty());
}

#[test]
fn out_of_image_fetch_surfaces_fetch_fault() {
    let mut t = Translator::new().expect("translator");
    let image = GuestImage::new(BASE, MOV_RAX_42_RET.to_vec());
    let mut d = Dispatcher::new(&mut t, &image).expect("dispatcher");

    let outcome = d.run(0xDEAD_0000, 16).expect("run");
    assert_eq!(outcome.exit, DispatchExit::FetchFailed);
    assert_eq!(outcome.final_pc, 0xDEAD_0000);
    assert!(!outcome.message.is_empty());
}

#[test]
fn gpr_accessors_round_trip() {
    let mut t = Translator::new().expect("translator");
    let image = GuestImage::new(BASE, MOV_RAX_42_RET.to_vec());
    let mut d = Dispatcher::new(&mut t, &image).expect("dispatcher");

    d.set_gpr(Gpr::Rdi, 0xDEAD_BEEF).expect("set");
    assert_eq!(d.gpr(Gpr::Rdi).expect("get"), 0xDEAD_BEEF);
    assert_eq!(d.guest_pc().expect("pc"), 0);
}

#[test]
fn direct_jit_patch_stats_are_exposed_on_arm64() {
    if !cfg!(target_arch = "aarch64") {
        return;
    }

    let mut t = Translator::new().expect("translator");
    let image = GuestImage::new(
        BASE,
        vec![
            0xEB, 0x0E, // jmp +14 -> BASE + 0x10
            0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
            0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
            0xEB, 0xEE, // jmp -18 -> BASE
        ],
    );
    let mut d = Dispatcher::new(&mut t, &image).expect("dispatcher");

    let outcome = d.run(BASE, 6).expect("run");
    assert_eq!(outcome.exit, DispatchExit::StepLimit);
    assert_eq!(outcome.stats.direct_jit_patch_attempts, 3);
    assert_eq!(outcome.stats.direct_jit_patch_applied, 1);
    assert_eq!(outcome.stats.direct_jit_patch_rejected, 2);
    assert_eq!(outcome.stats.direct_jit_patch_executes, 2);
}

#[test]
fn guest_program_executes_on_arm64() {
    if !cfg!(target_arch = "aarch64") {
        // Translation-only coverage runs in the other tests; JIT
        // execution requires an ARM64 host.
        return;
    }

    let mut t = Translator::new().expect("translator");
    let image = GuestImage::new(BASE, MOV_RAX_42_RET.to_vec());
    let mut d = Dispatcher::new(&mut t, &image).expect("dispatcher");
    d.install_halt_return_stack().expect("halt stack");

    let outcome = d.run(BASE, 64).expect("run");
    assert_eq!(outcome.exit, DispatchExit::Halted);
    assert_eq!(outcome.stats.blocks_executed, 1);
    assert_eq!(d.gpr(Gpr::Rax).expect("rax"), 42);
}
