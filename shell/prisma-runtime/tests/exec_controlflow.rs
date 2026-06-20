//! ARM64 execution e2e for intra-region control flow.
//!
//! Proves the executor runs a multi-block region with a real conditional branch
//! on hardware: a hand-built IR function with `CmpFlags` + `CondJumpRel` +
//! `JumpRel` lowers to ARM64 `B.cond`/`B` between blocks, ending by falling
//! through to the wrap epilogue (no `Op::Return` in the body, so the executor's
//! prologue/epilogue still bracket it correctly). The branch is taken or not
//! depending on the seeded registers. aarch64-gated; off-target the W^X install
//! path runs and the call reports `WrongArch`.

use prisma_backend::Lowerer;
use prisma_ir::{
    BasicBlock, CmpFlags, CondCode, CondJumpRel, Constant, Function, Gpr, JumpRel, LoadReg, Op,
    OpSize, Stmt, StoreReg,
};
#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};

/// `if rcx == rdx { rax = 100 } else { rax = 200 }`, as four IR blocks.
fn branch_region() -> Vec<u8> {
    let func = Function {
        entry: 0,
        blocks: vec![
            BasicBlock {
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
                        Op::LoadReg(LoadReg {
                            reg: Gpr::Rdx,
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
                            target_guest_pc: 2,
                            fallthrough_guest_pc: 1,
                        }),
                    ),
                ],
            },
            // rcx != rdx -> rax = 200, then jump over the equal block.
            BasicBlock {
                id: 1,
                stmts: vec![
                    Stmt::new(
                        Some(3),
                        Op::Constant(Constant {
                            value: 200,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 3,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(None, Op::JumpRel(JumpRel { target_guest_pc: 3 })),
                ],
            },
            // rcx == rdx -> rax = 100, fall through to the end block.
            BasicBlock {
                id: 2,
                stmts: vec![
                    Stmt::new(
                        Some(4),
                        Op::Constant(Constant {
                            value: 100,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 4,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            },
            // End block: falls through to the wrap epilogue.
            BasicBlock {
                id: 3,
                stmts: vec![],
            },
        ],
    };
    let words = Lowerer::new()
        .lower_function(&func)
        .expect("lower the branch region");
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn conditional_branch_taken_path() {
    let code = branch_region();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = 7; // equal -> taken
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 100, "rcx == rdx -> equal block");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn conditional_branch_fallthrough_path() {
    let code = branch_region();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 7;
    state.gpr[gpr::RDX] = 9; // not equal -> fallthrough
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 200, "rcx != rdx -> fallthrough block");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
