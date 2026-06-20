//! ARM64 execution e2e for the block-exit ABI: `Op::Return` routed through the
//! epilogue.
//!
//! A bare `ret` inside a wrapped region would skip the prologue's stack-pointer
//! and callee-saved restore and corrupt the caller. With the
//! `with_returns_via_epilogue` lowerer, `Op::Return` emits the full epilogue
//! (restore + `ret`), so a region that returns early leaves the host stack and
//! callee-saved registers correct. This region has TWO exits — an early
//! `Op::Return` and a fall-through — and both must return cleanly with the right
//! result. aarch64-gated; off-target the install path runs and reports
//! `WrongArch`.

use prisma_backend::Lowerer;
use prisma_ir::{
    BasicBlock, CmpFlags, CondCode, CondJumpRel, Constant, Function, Gpr, LoadReg, Op, OpSize,
    Return, Stmt, StoreReg,
};
#[cfg(not(target_arch = "aarch64"))]
use prisma_runtime::executor::ExecError;
use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame};

/// `if rcx == 0 { rax = 222; fall through } else { rax = 111; return }`.
fn early_return_region() -> Vec<u8> {
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
                            target_guest_pc: 2,
                            fallthrough_guest_pc: 1,
                        }),
                    ),
                ],
            },
            // rcx != 0 -> rax = 111, early Op::Return (must go through epilogue).
            BasicBlock {
                id: 1,
                stmts: vec![
                    Stmt::new(
                        Some(3),
                        Op::Constant(Constant {
                            value: 111,
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
                    Stmt::new(None, Op::Return(Return)),
                ],
            },
            // rcx == 0 -> rax = 222, fall through to the end (wrap epilogue).
            BasicBlock {
                id: 2,
                stmts: vec![
                    Stmt::new(
                        Some(4),
                        Op::Constant(Constant {
                            value: 222,
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
            BasicBlock {
                id: 3,
                stmts: vec![],
            },
        ],
    };
    let words = Lowerer::new()
        .with_returns_via_epilogue()
        .lower_function(&func)
        .expect("lower the early-return region");
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn early_return_path_returns_cleanly() {
    let code = early_return_region();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 5; // != 0 -> early Op::Return
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host (clean early return)");
        assert_eq!(state.gpr[gpr::RAX], 111, "early-return block result");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}

#[test]
fn fallthrough_path_returns_cleanly() {
    let code = early_return_region();
    let mut state = CpuStateFrame::default();
    state.gpr[gpr::RCX] = 0; // == 0 -> fall through
    let r = execute_block(&code, &mut state);

    #[cfg(target_arch = "aarch64")]
    {
        r.expect("execute on the ARM64 host");
        assert_eq!(state.gpr[gpr::RAX], 222, "fall-through block result");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        assert!(matches!(r, Err(ExecError::WrongArch)));
    }
}
