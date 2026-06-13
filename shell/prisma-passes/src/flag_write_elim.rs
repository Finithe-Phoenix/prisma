//! Flag write elimination pass.
//!
//! Mirrors C++ `flag_write_elimination`. Drops implicit flag writes whose
//! NZCV is never consumed. Flag writers: `CmpFlags`, `AluFlags`, `Compare`,
//! `WriteFlagsCountZero`. Flag readers: `CondJumpRel`, `Select` (lowers to
//! csel). `Compare` is never dropped (its result ref may be consumed
//! elsewhere); only the result-less writers (`CmpFlags`, `AluFlags`,
//! `WriteFlagsCountZero`) are droppable.

use prisma_ir::{BasicBlock, Function, Op};

use crate::Pass;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlagWriteElim;

impl FlagWriteElim {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for FlagWriteElim {
    fn name(&self) -> &'static str {
        "flag_write_elimination"
    }

    fn run(&self, func: Function) -> Function {
        flag_write_elimination(func)
    }
}

/// Per-block flag write elimination.
pub fn flag_write_elimination(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let n = block.stmts.len();
            let mut drop = vec![false; n];
            // Index of the most-recent flag writer; None if none pending.
            let mut pending_writer: Option<usize> = None;
            let mut pending_droppable = false;

            for (i, stmt) in block.stmts.iter().enumerate() {
                match &stmt.op {
                    Op::CmpFlags(_) | Op::AluFlags(_) | Op::WriteFlagsCountZero(_) => {
                        if let Some(prev) = pending_writer {
                            if pending_droppable {
                                drop[prev] = true;
                            }
                        }
                        pending_writer = Some(i);
                        pending_droppable = true;
                    }
                    Op::Compare(_) => {
                        if let Some(prev) = pending_writer {
                            if pending_droppable {
                                drop[prev] = true;
                            }
                        }
                        pending_writer = Some(i);
                        pending_droppable = false;
                    }
                    Op::CondJumpRel(_) | Op::Select(_) => {
                        // Reader pins the most recent writer.
                        pending_writer = None;
                        pending_droppable = false;
                    }
                    _ => {}
                }
            }
            // End-of-block: a still-pending droppable write has no consumer.
            if let Some(prev) = pending_writer {
                if pending_droppable {
                    drop[prev] = true;
                }
            }

            let stmts = block
                .stmts
                .into_iter()
                .enumerate()
                .filter_map(|(i, s)| if drop[i] { None } else { Some(s) })
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
    use prisma_ir::{CmpFlags, CondCode, CondJumpRel, OpSize, Select, Stmt};

    fn cmp(lhs: u32, rhs: u32) -> Stmt {
        Stmt::new(None, Op::CmpFlags(CmpFlags { lhs, rhs, size: OpSize::I64 }))
    }

    fn cond_jump() -> Stmt {
        Stmt::new(
            None,
            Op::CondJumpRel(CondJumpRel {
                cc: CondCode::Eq,
                target_guest_pc: 1,
                fallthrough_guest_pc: 2,
            }),
        )
    }

    fn block(stmts: Vec<Stmt>) -> Function {
        Function { entry: 0, blocks: vec![BasicBlock { id: 0, stmts }] }
    }

    #[test]
    fn unconsumed_cmp_is_dropped() {
        let out = flag_write_elimination(block(vec![cmp(0, 1)]));
        assert!(out.blocks[0].stmts.is_empty());
    }

    #[test]
    fn consumed_cmp_is_kept() {
        let out = flag_write_elimination(block(vec![cmp(0, 1), cond_jump()]));
        assert_eq!(out.blocks[0].stmts.len(), 2);
    }

    #[test]
    fn superseded_cmp_is_dropped_but_last_kept_if_read() {
        // cmp A ; cmp B ; cond_jump  -> first cmp dropped, second kept.
        let out = flag_write_elimination(block(vec![cmp(0, 1), cmp(2, 3), cond_jump()]));
        assert_eq!(out.blocks[0].stmts.len(), 2);
        match &out.blocks[0].stmts[0].op {
            Op::CmpFlags(c) => assert_eq!((c.lhs, c.rhs), (2, 3)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn select_reads_flags_and_pins_writer() {
        let out = flag_write_elimination(block(vec![
            cmp(0, 1),
            Stmt::new(
                Some(9),
                Op::Select(Select { cc: CondCode::Eq, true_value: 0, false_value: 1, size: OpSize::I64 }),
            ),
        ]));
        assert_eq!(out.blocks[0].stmts.len(), 2);
    }

    #[test]
    fn compare_is_never_dropped() {
        use prisma_ir::Compare;
        let out = flag_write_elimination(block(vec![Stmt::new(
            Some(2),
            Op::Compare(Compare { cc: CondCode::Eq, lhs: 0, rhs: 1, size: OpSize::I64 }),
        )]));
        assert_eq!(out.blocks[0].stmts.len(), 1);
    }
}
