//! Redundant load elimination pass.
//!
//! Mirrors C++ `redundant_load_eliminate`. A second `LoadMem` through the
//! same `(addr_ref, size)` with no intervening store/fence is rewritten to a
//! copy of the prior load's result (`Or %v, %v`), letting copy-prop + DCE do
//! downstream cleanup. `LoadMemTSO` is never forwarded (acquire semantics).

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOp, BinOpKind, Function, Op, OpSize, Ref, Stmt};

use crate::Pass;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RedundantLoad;

impl RedundantLoad {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for RedundantLoad {
    fn name(&self) -> &'static str {
        "redundant_load_eliminate"
    }

    fn run(&self, func: Function) -> Function {
        redundant_load_eliminate(func)
    }
}

/// Per-block redundant-load elimination.
pub fn redundant_load_eliminate(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            // key: (addr_ref, size) -> prior load result ref
            let mut last_load: HashMap<(Ref, OpSize), Ref> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                match &stmt.op {
                    Op::LoadMem(l) => {
                        if let Some(result) = stmt.result {
                            let key = (l.addr, l.size);
                            if let Some(&prev) = last_load.get(&key) {
                                out.push(Stmt::new(
                                    Some(result),
                                    Op::BinOp(BinOp {
                                        op: BinOpKind::Or,
                                        lhs: prev,
                                        rhs: prev,
                                        size: l.size,
                                    }),
                                ));
                                continue;
                            }
                            last_load.insert(key, result);
                        }
                    }
                    Op::StoreMem(_) | Op::StoreMemTSO(_) | Op::Fence(_) => {
                        // Any store could alias; a fence orders memory.
                        last_load.clear();
                    }
                    _ => {}
                }
                out.push(stmt);
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
    use prisma_ir::{Fence, FenceKind, LoadMem, StoreMem};

    fn load(result: u32, addr: u32, size: OpSize) -> Stmt {
        Stmt::new(Some(result), Op::LoadMem(LoadMem { addr, size }))
    }

    #[test]
    fn second_identical_load_becomes_copy() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![load(0, 5, OpSize::I64), load(1, 5, OpSize::I64)],
            }],
        };
        let out = redundant_load_eliminate(func);
        match &out.blocks[0].stmts[1].op {
            Op::BinOp(b) => {
                assert_eq!(b.op, BinOpKind::Or);
                assert_eq!(b.lhs, 0);
                assert_eq!(b.rhs, 0);
            }
            other => panic!("expected copy, got {other:?}"),
        }
    }

    #[test]
    fn store_between_loads_blocks_forwarding() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    load(0, 5, OpSize::I64),
                    Stmt::new(
                        None,
                        Op::StoreMem(StoreMem {
                            addr: 6,
                            value: 0,
                            size: OpSize::I64,
                        }),
                    ),
                    load(1, 5, OpSize::I64),
                ],
            }],
        };
        let out = redundant_load_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn fence_blocks_forwarding() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    load(0, 5, OpSize::I32),
                    Stmt::new(
                        None,
                        Op::Fence(Fence {
                            kind: FenceKind::Mfence,
                        }),
                    ),
                    load(1, 5, OpSize::I32),
                ],
            }],
        };
        let out = redundant_load_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn different_size_is_not_redundant() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![load(0, 5, OpSize::I32), load(1, 5, OpSize::I64)],
            }],
        };
        let out = redundant_load_eliminate(func.clone());
        assert_eq!(out, func);
    }
}
