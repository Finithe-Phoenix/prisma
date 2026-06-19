//! ARM64 end-to-end execution of the immediate-ALU broadening family.
//!
//! These tests run a real fused block (decode -> optimize -> lower to ARM64)
//! through the W^X JIT buffer and observe the guest `CpuStateFrame` mutate, the
//! same gate as the C++ e2e corpus: GPR assertions only run on aarch64 hosts;
//! everywhere else the translation is exercised but the call is skipped because
//! the emitted bytes are ARM64 machine code that the host cannot run.
//!
//! Covered opcodes:
//!   - `81 /0 id`  ADD r/m64, imm32         (the 0x81 group, mirror of 0x83)
//!   - `81 /4 id`  AND r/m32, imm32
//!   - `05 id`     ADD eax/rax, imm32       (accumulator-immediate)
//!   - `2D id`     SUB eax/rax, imm32       (accumulator-immediate)

use prisma_translator::Translator;

/// The lowered block carries the `CpuStateFrame*` in x27 (AAPCS64 callee-saved,
/// see `prisma_backend::abi::K_STATE_PTR_REG`). `LoadReg`/`StoreReg` index into
/// `gpr[]` which begins at offset 0 of the frame, so a plain `[u64; 16]` keyed
/// by `Gpr as usize` is a faithful stand-in for the C++ frame's GPR file.
#[cfg(target_arch = "aarch64")]
const GPR_COUNT: usize = 16;

/// Translate `guest` at `guest_addr` into ARM64 machine code, asserting it
/// lowered to a non-empty block. Panics on a translation error so the test
/// reports the decode/lower failure directly. Returned so aarch64 hosts can
/// execute it; on other hosts the non-empty assertion is the coverage.
fn translate(guest_addr: u64, guest: &[u8]) -> Vec<u8> {
    let mut translator = Translator::new();
    let block = translator
        .translate_fused_block(guest_addr, guest, 64)
        .expect("fused block translation");
    assert!(!block.code.is_empty(), "lowered block must emit ARM64 code");
    block.code
}

/// Execute a lowered block over a fresh GPR frame seeded from `gprs` and return
/// the resulting frame. Only callable on aarch64.
#[cfg(target_arch = "aarch64")]
fn run_block(code: &[u8], gprs: [u64; GPR_COUNT]) -> [u64; GPR_COUNT] {
    use prisma_runtime::jit_memory::ExecBuffer;

    let mut buffer = ExecBuffer::alloc(code.len()).expect("exec buffer");
    assert!(buffer.write(code), "write code into W^X buffer");
    buffer.make_executable().expect("flip to RX");

    let entry = buffer.as_ptr();
    let mut frame = gprs;
    // SAFETY: `entry` points at freshly emitted, I-cache-flushed ARM64 code that
    // ends in `ret`. We load the frame pointer into x27 (the state-pointer reg
    // the block reads), save/restore x27 ourselves because it is callee-saved,
    // and clobber the block's scratch range (x9..x23). `buffer` outlives the
    // call.
    unsafe {
        core::arch::asm!(
            "mov x27, {frame}",
            "blr {entry}",
            frame = in(reg) frame.as_mut_ptr(),
            entry = in(reg) entry,
            out("x27") _,
            // Scratch + value registers the lowered block may clobber.
            out("x9") _, out("x10") _, out("x11") _, out("x12") _,
            out("x13") _, out("x14") _, out("x15") _, out("x16") _,
            out("x17") _, out("x18") _, out("x19") _, out("x20") _,
            out("x21") _, out("x22") _, out("x23") _,
            out("lr") _,
            clobber_abi("C"),
        );
    }
    frame
}

/// Assert the translation is well-formed on every host. `code` is consumed here
/// so the non-aarch64 build has no unused binding; aarch64 hosts additionally
/// execute it in each test.
fn assert_translated(code: &[u8]) {
    assert!(!code.is_empty());
}

#[test]
fn add_rax_imm32_executes() {
    // add rax, 0x1234  (REX.W 81 /0 id)
    let code = translate(0x1000, b"\x48\x81\xC0\x34\x12\x00\x00");
    assert_translated(&code);

    #[cfg(target_arch = "aarch64")]
    {
        use prisma_ir::Gpr;
        let mut gprs = [0u64; GPR_COUNT];
        gprs[Gpr::Rax as usize] = 0x1111;
        let out = run_block(&code, gprs);
        assert_eq!(out[Gpr::Rax as usize], 0x1111 + 0x1234);
    }
}

#[test]
fn and_eax_imm32_executes() {
    // and eax, 0x0F0F  (81 /4 id, 32-bit operand zero-extends to rax)
    let code = translate(0x2000, b"\x81\xE0\x0F\x0F\x00\x00");
    assert_translated(&code);

    #[cfg(target_arch = "aarch64")]
    {
        use prisma_ir::Gpr;
        let mut gprs = [0u64; GPR_COUNT];
        gprs[Gpr::Rax as usize] = 0xFFFF_FFFF_FFFF_FFFF;
        let out = run_block(&code, gprs);
        // 32-bit ALU result is written zero-extended into the 64-bit register.
        assert_eq!(out[Gpr::Rax as usize], 0x0F0F);
    }
}

#[test]
fn add_eax_imm32_accumulator_executes() {
    // add eax, 0x1234  (05 id, accumulator-immediate)
    let code = translate(0x3000, b"\x05\x34\x12\x00\x00");
    assert_translated(&code);

    #[cfg(target_arch = "aarch64")]
    {
        use prisma_ir::Gpr;
        let mut gprs = [0u64; GPR_COUNT];
        gprs[Gpr::Rax as usize] = 0x1000;
        let out = run_block(&code, gprs);
        assert_eq!(out[Gpr::Rax as usize], 0x1000 + 0x1234);
    }
}

#[test]
fn sub_rax_imm32_accumulator_executes() {
    // sub rax, 0x10  (REX.W 2D id, accumulator-immediate)
    let code = translate(0x4000, b"\x48\x2D\x10\x00\x00\x00");
    assert_translated(&code);

    #[cfg(target_arch = "aarch64")]
    {
        use prisma_ir::Gpr;
        let mut gprs = [0u64; GPR_COUNT];
        gprs[Gpr::Rax as usize] = 0x100;
        let out = run_block(&code, gprs);
        assert_eq!(out[Gpr::Rax as usize], 0x100 - 0x10);
    }
}
