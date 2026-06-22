//! ARM64 execution e2e for the branch block-exit ABI (`CondJumpRel`/`JumpRel`
//! routed through the frame's `next_pc`).
//!
//! A host-wrapped block has no sibling block to branch to, so a relative branch
//! must instead compute its taken guest PC, store it in `CpuStateFrame::next_pc`,
//! mark `exit_reason = EXIT_BRANCH`, and return — the run loop then resumes
//! there. This builds a single block that compares `rcx` to 0 and conditionally
//! jumps, and checks both the taken (full 64-bit) target and the fall-through are
//! recorded. aarch64-gated; off-target the install path runs and reports
//! `WrongArch`.

use prisma_backend::Lowerer;
use prisma_ir::{
    BasicBlock, CmpFlags, CondCode, CondJumpRel, Constant, Function, Gpr, LoadReg, Op, OpSize, Stmt,
};
#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, EXIT_BRANCH};

// Guest PCs above u32 — the next_pc path carries the full 64-bit target, unlike
// the sibling-block label path which truncates to a u32 block id.
const TAKEN_PC: u64 = 0x1_4000_2000;
const FALL_PC: u64 = 0x1_4000_3000;

/// `if rcx <cc> 0 { goto TAKEN } else { goto FALL }`, comparing at `size`,
/// lowered as a host-wrapped block so the conditional branch exits via `next_pc`.
/// A non-`I64` size exercises the shift-based flag alignment in the lowerer.
fn cond_branch_block(size: OpSize, cc: CondCode) -> Vec<u8> {
    let func = Function {
        entry: 0,
        blocks: vec![BasicBlock {
            id: 0,
            stmts: vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size,
                    }),
                ),
                Stmt::new(Some(1), Op::Constant(Constant { value: 0, size })),
                Stmt::new(
                    Some(2),
                    Op::CmpFlags(CmpFlags {
                        lhs: 0,
                        rhs: 1,
                        size,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::CondJumpRel(CondJumpRel {
                        cc,
                        target_guest_pc: TAKEN_PC,
                        fallthrough_guest_pc: FALL_PC,
                    }),
                ),
            ],
        }],
    };
    let words = Lowerer::new()
        .with_branch_exits()
        .lower_function(&func)
        .expect("lower the conditional-branch block");
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

// Run `cond_branch_block(size, cc)` with `rcx = rcx_val` and return the recorded
// next_pc on ARM64 (None off-target, where WrongArch is asserted instead).
fn run_cond(size: OpSize, cc: CondCode, rcx_val: u64) -> Option<u64> {
    let code = cond_branch_block(size, cc);
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = rcx_val;
    let r = execute_block(&code, &mut state);
    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.exit_reason, EXIT_BRANCH, "branch exit reason");
        Some(state.next_pc)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (EXIT_BRANCH, cc);
        assert!(matches!(r, Err(ExecError::WrongArch)));
        None
    }
}

#[test]
fn i64_taken_branch_records_target_pc() {
    let next = run_cond(OpSize::I64, CondCode::Eq, 0); // rcx == 0 -> taken
    if cfg!(target_arch = "aarch64") {
        assert_eq!(next, Some(TAKEN_PC));
    }
}

#[test]
fn i64_untaken_branch_records_fallthrough_pc() {
    let next = run_cond(OpSize::I64, CondCode::Eq, 7); // rcx != 0 -> fall
    if cfg!(target_arch = "aarch64") {
        assert_eq!(next, Some(FALL_PC));
    }
}

// The I32 cases exercise the shift-based flag alignment: a 32-bit compare aligns
// operands into the high half before SUBS so NZCV reflects the 32-bit result.
#[test]
fn i32_ne_branch_taken_when_nonzero() {
    let next = run_cond(OpSize::I32, CondCode::Ne, 4); // 4 != 0 -> taken (like a loop)
    if cfg!(target_arch = "aarch64") {
        assert_eq!(
            next,
            Some(TAKEN_PC),
            "I32 Ne with rcx=4 must take the branch"
        );
    }
}

#[test]
fn i32_ne_branch_untaken_when_zero() {
    let next = run_cond(OpSize::I32, CondCode::Ne, 0); // 0 == 0 -> fall (loop exit)
    if cfg!(target_arch = "aarch64") {
        assert_eq!(next, Some(FALL_PC), "I32 Ne with rcx=0 must fall through");
    }
}
