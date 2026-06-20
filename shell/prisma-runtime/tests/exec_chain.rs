//! ARM64 execution e2e for chaining blocks via the guest PC — the core of a
//! dispatcher run loop.
//!
//! A 3-instruction straight-line program is translated and executed ONE
//! instruction at a time, advancing the guest PC by each block's consumed guest
//! bytes and carrying the `CpuStateFrame` across blocks. This proves the runtime
//! runs a multi-block program by chaining, not just a single fused block. The
//! GPR assertion is aarch64-gated; off-target each block still translates and
//! the install path runs (execution skipped with `WrongArch`).

#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};
use prisma_translator::Translator;

#[test]
fn chains_single_instruction_blocks_via_pc() {
    // mov rax, rcx ; add rax, 5 ; sub rax, 2  -> rax = rcx + 3
    let program: &[u8] = &[
        0x48, 0x89, 0xC8, // mov rax, rcx
        0x48, 0x83, 0xC0, 0x05, // add rax, 5
        0x48, 0x83, 0xE8, 0x02, // sub rax, 2
    ];
    let base_pc = 0x1000u64;

    let mut t = Translator::new();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 40;

    let mut offset = 0usize;
    let mut blocks = 0usize;
    while offset < program.len() && blocks < 8 {
        // max_insns = 1 -> one block per instruction, so we exercise chaining.
        let block = t
            .translate_block(base_pc + offset as u64, &program[offset..], 1)
            .expect("translate one-instruction block");
        assert!(block.guest_bytes > 0, "block must consume guest bytes");
        assert_eq!(block.instruction_count, 1, "one instruction per block");

        let r = execute_block(&block.code, &mut state);
        #[cfg(target_arch = "aarch64")]
        r.expect("execute the block on the ARM64 host");
        #[cfg(not(target_arch = "aarch64"))]
        assert!(matches!(r, Err(ExecError::WrongArch)));

        offset += block.guest_bytes;
        blocks += 1;
    }

    assert_eq!(offset, program.len(), "consumed the whole program");
    assert_eq!(blocks, 3, "three chained single-instruction blocks");

    #[cfg(target_arch = "aarch64")]
    assert_eq!(
        state.gpr[gpr::RAX],
        43,
        "rcx(40) + 5 - 2, chained across blocks"
    );
}
