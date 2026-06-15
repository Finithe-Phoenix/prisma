//! Tail-call optimization pass.
//!
//! Mirrors C++ `tail_call_optimise`. A `CallRel{T,R}` immediately followed by
//! `RetAdjusted{pop_bytes: 0}` is observably equivalent to `JumpRel{T}` — the
//! push-then-pop of the return address `R` round-trips through the dispatcher
//! for nothing. Rewriting to a single `JumpRel` keeps the same final
//! destination and stack with one fewer dispatcher round trip.

use prisma_ir::{BasicBlock, Function, JumpRel, Op, Stmt};

use crate::Pass;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TailCall;

impl TailCall {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for TailCall {
    fn name(&self) -> &'static str {
        "tail_call_optimise"
    }

    fn run(&self, func: Function) -> Function {
        tail_call_optimise(func)
    }
}

/// Per-block tail-call folding.
pub fn tail_call_optimise(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let stmts = block.stmts;
            let mut out = Vec::with_capacity(stmts.len());
            let mut i = 0;
            while i < stmts.len() {
                if i + 1 < stmts.len() {
                    if let (Op::CallRel(call), Op::RetAdjusted(ret)) =
                        (&stmts[i].op, &stmts[i + 1].op)
                    {
                        if ret.pop_bytes == 0 {
                            out.push(Stmt::new(
                                None,
                                Op::JumpRel(JumpRel {
                                    target_guest_pc: call.target_guest_pc,
                                }),
                            ));
                            i += 2; // consume CallRel + RetAdjusted
                            continue;
                        }
                    }
                }
                out.push(stmts[i].clone());
                i += 1;
            }
            BasicBlock {
                id: block.id,
                stmts: out,
            }
        })
        .collect();

    Function {
        blocks,
        entry: func.entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{CallRel, RetAdjusted};

    fn block(stmts: Vec<Stmt>) -> Function {
        Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        }
    }

    #[test]
    fn call_then_ret0_becomes_jump() {
        let out = tail_call_optimise(block(vec![
            Stmt::new(
                None,
                Op::CallRel(CallRel {
                    target_guest_pc: 0x1234,
                    return_guest_pc: 0x5,
                }),
            ),
            Stmt::new(None, Op::RetAdjusted(RetAdjusted { pop_bytes: 0 })),
        ]));
        assert_eq!(out.blocks[0].stmts.len(), 1);
        match &out.blocks[0].stmts[0].op {
            Op::JumpRel(j) => assert_eq!(j.target_guest_pc, 0x1234),
            other => panic!("expected jumprel, got {other:?}"),
        }
    }

    #[test]
    fn nonzero_pop_is_not_folded() {
        let func = block(vec![
            Stmt::new(
                None,
                Op::CallRel(CallRel {
                    target_guest_pc: 0x1234,
                    return_guest_pc: 0x5,
                }),
            ),
            Stmt::new(None, Op::RetAdjusted(RetAdjusted { pop_bytes: 8 })),
        ]);
        let out = tail_call_optimise(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn call_without_following_ret_is_kept() {
        let func = block(vec![Stmt::new(
            None,
            Op::CallRel(CallRel {
                target_guest_pc: 0x1234,
                return_guest_pc: 0x5,
            }),
        )]);
        let out = tail_call_optimise(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn idempotent() {
        let func = block(vec![
            Stmt::new(
                None,
                Op::CallRel(CallRel {
                    target_guest_pc: 0x1234,
                    return_guest_pc: 0x5,
                }),
            ),
            Stmt::new(None, Op::RetAdjusted(RetAdjusted { pop_bytes: 0 })),
        ]);
        let once = tail_call_optimise(func);
        let twice = tail_call_optimise(once.clone());
        assert_eq!(once, twice);
    }
}
