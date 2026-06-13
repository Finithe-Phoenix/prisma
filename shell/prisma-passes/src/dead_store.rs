//! Dead store elimination pass.
//!
//! Mirrors C++ `dead_store_eliminate`. A `StoreMem` to `(addr_ref, size)`
//! that is overwritten by a later `StoreMem` to the same key, with no
//! intervening memory op that could observe it, is dropped. `StoreMemTSO`
//! (release-ordered), `LoadMem`, `LoadMemTSO`, and `Fence` clear the pending
//! table — the earlier store may have been observed.

use std::collections::{HashMap, HashSet};

use prisma_ir::{BasicBlock, Function, Op, OpSize, Ref, Stmt};

use crate::Pass;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeadStore;

impl DeadStore {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for DeadStore {
    fn name(&self) -> &'static str {
        "dead_store_eliminate"
    }

    fn run(&self, func: Function) -> Function {
        dead_store_eliminate(func)
    }
}

/// Per-block dead store elimination.
pub fn dead_store_eliminate(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut pending: HashMap<(Ref, OpSize), usize> = HashMap::new();
            let mut dead: HashSet<usize> = HashSet::new();

            for (i, stmt) in block.stmts.iter().enumerate() {
                match &stmt.op {
                    Op::StoreMem(st) => {
                        let key = (st.addr, st.size);
                        if let Some(prev) = pending.insert(key, i) {
                            dead.insert(prev);
                        }
                    }
                    Op::LoadMem(_) | Op::LoadMemTSO(_) | Op::StoreMemTSO(_) | Op::Fence(_) => {
                        pending.clear();
                    }
                    _ => {}
                }
            }

            let stmts = block
                .stmts
                .into_iter()
                .enumerate()
                .filter_map(|(i, s)| if dead.contains(&i) { None } else { Some(s) })
                .collect();

            BasicBlock { id: block.id, stmts }
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
    use prisma_ir::{Fence, FenceKind, LoadMem, StoreMem, StoreMemTSO};

    fn store(addr: u32, value: u32, size: OpSize) -> Stmt {
        Stmt::new(None, Op::StoreMem(StoreMem { addr, value, size }))
    }

    #[test]
    fn overwritten_store_is_dropped() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![store(5, 0, OpSize::I64), store(5, 1, OpSize::I64)],
            }],
        };
        let out = dead_store_eliminate(func);
        assert_eq!(out.blocks[0].stmts.len(), 1);
        match &out.blocks[0].stmts[0].op {
            Op::StoreMem(s) => assert_eq!(s.value, 1),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn intervening_load_keeps_both() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    store(5, 0, OpSize::I64),
                    Stmt::new(Some(9), Op::LoadMem(LoadMem { addr: 6, size: OpSize::I64 })),
                    store(5, 1, OpSize::I64),
                ],
            }],
        };
        let out = dead_store_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn tso_store_never_killed() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(None, Op::StoreMemTSO(StoreMemTSO { addr: 5, value: 0, size: OpSize::I64 })),
                    store(5, 1, OpSize::I64),
                ],
            }],
        };
        let out = dead_store_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn different_addr_keeps_both() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![store(5, 0, OpSize::I64), store(6, 1, OpSize::I64)],
            }],
        };
        let out = dead_store_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn fence_blocks_elimination() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    store(5, 0, OpSize::I64),
                    Stmt::new(None, Op::Fence(Fence { kind: FenceKind::Sfence })),
                    store(5, 1, OpSize::I64),
                ],
            }],
        };
        let out = dead_store_eliminate(func.clone());
        assert_eq!(out, func);
    }
}
