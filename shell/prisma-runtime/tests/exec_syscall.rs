//! End-to-end: a guest block ending in `SYSCALL` (0F 05) translates to ARM64
//! that exits to the host with `exit_reason == EXIT_SYSCALL`, leaving the guest
//! registers (the syscall number + args) intact for the run loop to service.
//!
//! The execution half is `aarch64`-gated (the bytes are ARM64 machine code), so
//! on the x86 dev host the test still translates + drives the W^X install path
//! and asserts `ExecError::WrongArch`. Behavioural validation runs on the
//! `ffi-link-arm64` CI runner. This mirrors the C++ core's `is_arm64` gate.

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError, EXIT_SYSCALL};
use prisma_translator::Translator;

#[test]
fn syscall_block_exits_with_the_syscall_reason_and_preserves_registers() {
    // mov eax, 60      (B8 3C 00 00 00)  -> rax = 60 (exit_group's number)
    // mov edi, 7       (BF 07 00 00 00)  -> rdi = 7  (the status arg)
    // syscall          (0F 05)           -> block terminator: exit to host
    let mut program = Vec::new();
    program.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    program.extend_from_slice(&[0xBF, 0x07, 0x00, 0x00, 0x00]);
    program.extend_from_slice(&[0x0F, 0x05]);

    let mut t = Translator::new();
    let block = t
        .translate_fused_block(0x1000, &program, 64)
        .expect("translate the syscall block");
    // SYSCALL ends the block (it is a terminator), so all three insns are in it.
    assert_eq!(block.instruction_count, 3);
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");

    let mut state = CpuStateFrame::default();
    let result = execute_block(&block.code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        result.expect("execute on the ARM64 host");
        // The block exited *because of* the syscall — the run loop reads this.
        assert_eq!(state.exit_reason, EXIT_SYSCALL);
        // The syscall number and arg survive the trap, ready to dispatch.
        assert_eq!(state.gpr[gpr::RAX], 60, "rax holds the syscall number");
        assert_eq!(state.gpr[gpr::RDI], 7, "rdi holds the status arg");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        // Off-target: translate + W^X install ran; execution skipped.
        assert!(matches!(result, Err(ExecError::WrongArch)));
        let _ = (EXIT_SYSCALL, gpr::RDI);
    }
}
