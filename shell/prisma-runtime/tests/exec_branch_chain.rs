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

/// `if rcx == 0 { goto TAKEN } else { goto FALL }`, lowered as a host-wrapped
/// block so the conditional branch exits via `next_pc`.
fn cond_branch_block() -> Vec<u8> {
    let func = Function {
        entry: 0,
        blocks: vec![BasicBlock {
            id: 0,
            stmts: vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::CmpFlags(CmpFlags {
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::CondJumpRel(CondJumpRel {
                        cc: CondCode::Eq,
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

#[test]
fn taken_branch_records_target_pc() {
    let code = cond_branch_block();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0; // rcx == 0 -> condition true -> TAKEN
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.exit_reason, EXIT_BRANCH, "branch exit reason");
        assert_eq!(state.next_pc, TAKEN_PC, "taken target recorded");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = EXIT_BRANCH;
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn untaken_branch_records_fallthrough_pc() {
    let code = cond_branch_block();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 7; // rcx != 0 -> condition false -> FALL
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.exit_reason, EXIT_BRANCH, "branch exit reason");
        assert_eq!(state.next_pc, FALL_PC, "fall-through target recorded");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
