//! Common subexpression elimination (local, within a basic block).
//!
//! Mirrors C++ `common_subexpression_eliminate`. For each `BinOp`, a
//! canonical key `(op, lhs, rhs, size)` is looked up; a repeat is rewritten
//! to a copy of the prior result (`Or %prev, %prev`). Any flag/side-effecting
//! op conservatively flushes the table.

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOp, BinOpKind, Function, Op, OpSize, Ref, Stmt};

use crate::Pass;

fn is_flushing_op(op: &Op) -> bool {
    matches!(
        op,
        Op::StoreReg(_)
            | Op::StoreMem(_)
            | Op::StoreMemTSO(_)
            | Op::CmpFlags(_)
            | Op::AluFlags(_)
            | Op::WriteFlagsPopcnt(_)
            | Op::WriteFlagsCountZero(_)
            | Op::Syscall(_)
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cse;

impl Cse {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for Cse {
    fn name(&self) -> &'static str {
        "common_subexpression_eliminate"
    }

    fn run(&self, func: Function) -> Function {
        common_subexpression_eliminate(func)
    }
}

/// Per-block CSE.
pub fn common_subexpression_eliminate(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            // key: (op, lhs, rhs, size) -> defining result ref
            let mut seen: HashMap<(BinOpKind, Ref, Ref, OpSize), Ref> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                let mut new_stmt = stmt;
                if is_flushing_op(&new_stmt.op) {
                    seen.clear();
                } else if let (Some(result), Op::BinOp(b)) = (new_stmt.result, &new_stmt.op) {
                    let key = (b.op, b.lhs, b.rhs, b.size);
                    if let Some(&prev) = seen.get(&key) {
                        let size = b.size;
                        new_stmt.op = Op::BinOp(BinOp {
                            op: BinOpKind::Or,
                            lhs: prev,
                            rhs: prev,
                            size,
                        });
                    } else {
                        seen.insert(key, result);
                    }
                }
                out.push(new_stmt);
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
    use prisma_ir::{CmpFlags, Constant};

    fn binop(result: u32, op: BinOpKind, lhs: u32, rhs: u32) -> Stmt {
        Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op,
                lhs,
                rhs,
                size: OpSize::I64,
            }),
        )
    }

    #[test]
    fn repeated_binop_becomes_copy() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    binop(2, BinOpKind::Add, 0, 1),
                    binop(3, BinOpKind::Add, 0, 1), // identical -> copy of r2
                ],
            }],
        };
        let out = common_subexpression_eliminate(func);
        match &out.blocks[0].stmts[1].op {
            Op::BinOp(b) => {
                assert_eq!(b.op, BinOpKind::Or);
                assert_eq!(b.lhs, 2);
                assert_eq!(b.rhs, 2);
            }
            other => panic!("expected copy, got {other:?}"),
        }
    }

    #[test]
    fn flushing_op_resets_table() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    binop(2, BinOpKind::Add, 0, 1),
                    Stmt::new(
                        None,
                        Op::CmpFlags(CmpFlags {
                            lhs: 0,
                            rhs: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    binop(3, BinOpKind::Add, 0, 1), // NOT deduped after flush
                ],
            }],
        };
        let out = common_subexpression_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn different_operands_not_deduped() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    binop(2, BinOpKind::Add, 0, 1),
                    binop(3, BinOpKind::Add, 0, 4),
                ],
            }],
        };
        let out = common_subexpression_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn idempotent() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    binop(2, BinOpKind::Add, 0, 0),
                    binop(3, BinOpKind::Add, 0, 0),
                ],
            }],
        };
        let once = common_subexpression_eliminate(func);
        let twice = common_subexpression_eliminate(once.clone());
        assert_eq!(once, twice);
    }
}
