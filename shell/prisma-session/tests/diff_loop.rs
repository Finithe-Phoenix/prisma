//! ARM64 differential: pinpoint the arithmetic-loop codegen bug.
//!
//! For each block of the loop program, run BOTH the ARM64 `execute_block` and the
//! reference `interpret_block` from the SAME starting register state (over the
//! exact same optimized IR), then compare the resulting GPRs and the control
//! decision (taken branch target / syscall). The first block that disagrees is
//! the one whose codegen is wrong — the interpreter already proved the IR itself
//! is correct (`prisma-translator`'s `interp_loop_diag`).
//!
//! aarch64-gated: on the x86 dev host `execute_block` is `WrongArch`, so the
//! comparison cannot run; the test just asserts that.

use prisma_runtime::executor::{
    execute_block, CpuStateFrame, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL,
};
#[cfg(target_arch = "aarch64")]
use prisma_translator::interp::{interpret_block, BlockOutcome, GuestRegs};
use prisma_translator::Translator;

const ENTRY_VA: u64 = 0x1_4000_1000;

#[rustfmt::skip]
const LOOP_PROGRAM: [u8; 28] = [
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0
    0xB9, 0x05, 0x00, 0x00, 0x00, // mov ecx, 5
    0x83, 0xC7, 0x01,             // add edi, 1   (entry + 10 = top)
    0x83, 0xE9, 0x01,             // sub ecx, 1
    0x83, 0xF9, 0x00,             // cmp ecx, 0
    0x75, 0xF5,                   // jnz top
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60
    0x0F, 0x05,                   // syscall
];

#[test]
fn arm64_matches_interpreter_block_by_block() {
    let mut t = Translator::new();

    #[cfg(not(target_arch = "aarch64"))]
    {
        // Off-target: execute_block can't run; confirm the gate and stop.
        let blk = t
            .translate_fused_block(ENTRY_VA, &LOOP_PROGRAM, 16)
            .expect("translate");
        let mut frame = CpuStateFrame::default();
        let _ = (&EXIT_BRANCH, &EXIT_SYSCALL, &EXIT_NORMAL);
        assert!(execute_block(&blk.code, &mut frame).is_err());
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut frame = CpuStateFrame::default();
        let mut pc = ENTRY_VA;

        for step in 0..64 {
            let off = usize::try_from(pc - ENTRY_VA).expect("pc in program");
            let opt = t
                .optimize_fused_block(pc, &LOOP_PROGRAM[off..], 16)
                .expect("optimize");
            let blk = t
                .translate_fused_block(pc, &LOOP_PROGRAM[off..], 16)
                .expect("translate");

            // Both sides start from the same architectural state.
            let mut iregs = GuestRegs { gpr: frame.gpr };
            let iout = interpret_block(&opt.func.blocks[0].stmts, &mut iregs);

            frame.exit_reason = EXIT_NORMAL;
            execute_block(&blk.code, &mut frame).expect("execute on ARM64");

            assert_eq!(
                frame.gpr, iregs.gpr,
                "GPR divergence at pc={pc:#x} step={step}: ARM64={:x?} interp={:x?}",
                frame.gpr, iregs.gpr
            );

            // Compare the control decision and advance by the ARM64 result.
            match iout {
                BlockOutcome::Branch(target) => {
                    assert_eq!(
                        frame.exit_reason, EXIT_BRANCH,
                        "pc={pc:#x} step={step}: interp branched but ARM64 exit_reason={}",
                        frame.exit_reason
                    );
                    assert_eq!(
                        frame.next_pc, target,
                        "BRANCH TARGET divergence at pc={pc:#x} step={step}: ARM64 next_pc={:#x} interp={target:#x}",
                        frame.next_pc
                    );
                    pc = frame.next_pc;
                }
                BlockOutcome::Syscall => {
                    assert_eq!(
                        frame.exit_reason, EXIT_SYSCALL,
                        "pc={pc:#x}: syscall mismatch"
                    );
                    return; // reached the exit syscall with matching state
                }
                other => panic!("unexpected interp outcome {other:?} at pc={pc:#x}"),
            }
        }
        panic!("loop did not reach the exit syscall within the step budget");
    }
}
