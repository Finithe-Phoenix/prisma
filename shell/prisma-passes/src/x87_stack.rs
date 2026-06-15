//! x87 stack forwarding pass.
//!
//! Mirrors C++ `x87_stack_eliminate`. Within a single block, avoids
//! re-loading `ST(i)` when the slot's value is already a known SSA ref:
//! `X87Load` of a known slot becomes a copy idiom (`Or %src,%src`) and the
//! result is aliased to the source. `X87Store`/`Push`/`Pop` maintain the
//! known-stack model; control-flow / call / trap ops clear knowledge.

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOp, BinOpKind, Function, Op, OpSize, Ref, Stmt};

use crate::Pass;

const X87_DEPTH: usize = 8;

/// Follow the alias chain to a fixpoint.
fn resolve(mut r: Ref, alias: &HashMap<Ref, Ref>) -> Ref {
    loop {
        match alias.get(&r) {
            Some(&next) if next != r => r = next,
            _ => return r,
        }
    }
}

/// Rewrite the value operand of `X87Store`/`X87Push` through the alias map.
fn rewrite_value_operands(op: Op, alias: &HashMap<Ref, Ref>) -> Op {
    match op {
        Op::X87Store(mut s) => {
            s.value = resolve(s.value, alias);
            Op::X87Store(s)
        }
        Op::X87Push(mut p) => {
            p.value = resolve(p.value, alias);
            Op::X87Push(p)
        }
        other => other,
    }
}

fn push_known(stack: &mut [Option<Ref>; X87_DEPTH], value: Ref) {
    for i in (1..X87_DEPTH).rev() {
        stack[i] = stack[i - 1];
    }
    stack[0] = Some(value);
}

fn pop_known(stack: &mut [Option<Ref>; X87_DEPTH]) {
    for i in 0..X87_DEPTH - 1 {
        stack[i] = stack[i + 1];
    }
    stack[X87_DEPTH - 1] = None;
}

fn clears_x87_knowledge(op: &Op) -> bool {
    matches!(
        op,
        Op::CallRel(_)
            | Op::CallReg(_)
            | Op::RetAdjusted(_)
            | Op::Jump(_)
            | Op::JumpRel(_)
            | Op::JumpReg(_)
            | Op::CondJump(_)
            | Op::CondJumpRel(_)
            | Op::CondJumpFlags(_)
            | Op::Return(_)
            | Op::Syscall(_)
            | Op::Trap(_)
            | Op::InlineAsm(_)
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct X87Stack;

impl X87Stack {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for X87Stack {
    fn name(&self) -> &'static str {
        "x87_stack_eliminate"
    }

    fn run(&self, func: Function) -> Function {
        x87_stack_eliminate(func)
    }
}

/// Per-block x87 stack forwarding.
pub fn x87_stack_eliminate(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut known: [Option<Ref>; X87_DEPTH] = [None; X87_DEPTH];
            let mut alias: HashMap<Ref, Ref> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                let result = stmt.result;
                let op = rewrite_value_operands(stmt.op, &alias);

                match &op {
                    Op::X87Load(load) => {
                        let idx = load.st_index as usize;
                        if idx < X87_DEPTH {
                            if let (Some(result), Some(slot)) = (result, known[idx]) {
                                let src = resolve(slot, &alias);
                                alias.insert(result, src);
                                out.push(Stmt::new(
                                    Some(result),
                                    Op::BinOp(BinOp {
                                        op: BinOpKind::Or,
                                        lhs: src,
                                        rhs: src,
                                        size: OpSize::I64,
                                    }),
                                ));
                                continue;
                            }
                        }
                        // Not forwarded: keep the load, update knowledge.
                        out.push(Stmt::new(result, op));
                        if idx < X87_DEPTH {
                            if let Some(result) = result {
                                known[idx] = Some(result);
                            } else {
                                known = [None; X87_DEPTH];
                            }
                        } else {
                            known = [None; X87_DEPTH];
                        }
                    }
                    Op::X87Store(store) => {
                        let idx = store.st_index as usize;
                        if idx < X87_DEPTH {
                            known[idx] = Some(resolve(store.value, &alias));
                        } else {
                            known = [None; X87_DEPTH];
                        }
                        out.push(Stmt::new(result, op));
                    }
                    Op::X87Push(push) => {
                        let v = resolve(push.value, &alias);
                        push_known(&mut known, v);
                        out.push(Stmt::new(result, op));
                    }
                    Op::X87Pop(_) => {
                        if let (Some(result), Some(top)) = (result, known[0]) {
                            let src = resolve(top, &alias);
                            alias.insert(result, src);
                        }
                        pop_known(&mut known);
                        out.push(Stmt::new(result, op));
                    }
                    other => {
                        if clears_x87_knowledge(other) {
                            known = [None; X87_DEPTH];
                        }
                        out.push(Stmt::new(result, op));
                    }
                }
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
    use prisma_ir::{X87Load, X87Pop, X87Push, X87Store};

    fn block(stmts: Vec<Stmt>) -> Function {
        Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        }
    }

    #[test]
    fn store_then_load_forwards_to_copy() {
        // X87Store ST0 = %9 ; %1 = X87Load ST0  -> %1 = Or %9,%9
        let out = x87_stack_eliminate(block(vec![
            Stmt::new(
                None,
                Op::X87Store(X87Store {
                    st_index: 0,
                    value: 9,
                }),
            ),
            Stmt::new(Some(1), Op::X87Load(X87Load { st_index: 0 })),
        ]));
        match &out.blocks[0].stmts[1].op {
            Op::BinOp(b) => {
                assert_eq!(b.op, BinOpKind::Or);
                assert_eq!(b.lhs, 9);
                assert_eq!(b.rhs, 9);
            }
            other => panic!("expected copy, got {other:?}"),
        }
    }

    #[test]
    fn unknown_slot_load_is_kept() {
        let out = x87_stack_eliminate(block(vec![Stmt::new(
            Some(1),
            Op::X87Load(X87Load { st_index: 0 }),
        )]));
        assert!(matches!(out.blocks[0].stmts[0].op, Op::X87Load(_)));
    }

    #[test]
    fn push_shifts_stack_and_load_st1_forwards_old_top() {
        // store ST0=%9 ; push %8 (ST0=%8, ST1=%9) ; load ST1 -> copy of %9
        let out = x87_stack_eliminate(block(vec![
            Stmt::new(
                None,
                Op::X87Store(X87Store {
                    st_index: 0,
                    value: 9,
                }),
            ),
            Stmt::new(None, Op::X87Push(X87Push { value: 8 })),
            Stmt::new(Some(1), Op::X87Load(X87Load { st_index: 1 })),
        ]));
        match &out.blocks[0].stmts[2].op {
            Op::BinOp(b) => assert_eq!((b.lhs, b.rhs), (9, 9)),
            other => panic!("expected copy of %9, got {other:?}"),
        }
    }

    #[test]
    fn call_clears_knowledge() {
        use prisma_ir::CallRel;
        let out = x87_stack_eliminate(block(vec![
            Stmt::new(
                None,
                Op::X87Store(X87Store {
                    st_index: 0,
                    value: 9,
                }),
            ),
            Stmt::new(
                None,
                Op::CallRel(CallRel {
                    target_guest_pc: 1,
                    return_guest_pc: 2,
                }),
            ),
            Stmt::new(Some(1), Op::X87Load(X87Load { st_index: 0 })),
        ]));
        // Load after the call is not forwarded.
        assert!(matches!(out.blocks[0].stmts[2].op, Op::X87Load(_)));
    }

    #[test]
    fn pop_then_load_uses_new_top() {
        // store ST0=%9 ; push %8 ; pop ; load ST0 -> copy of %9
        let out = x87_stack_eliminate(block(vec![
            Stmt::new(
                None,
                Op::X87Store(X87Store {
                    st_index: 0,
                    value: 9,
                }),
            ),
            Stmt::new(None, Op::X87Push(X87Push { value: 8 })),
            Stmt::new(Some(5), Op::X87Pop(X87Pop)),
            Stmt::new(Some(1), Op::X87Load(X87Load { st_index: 0 })),
        ]));
        match &out.blocks[0].stmts.last().unwrap().op {
            Op::BinOp(b) => assert_eq!((b.lhs, b.rhs), (9, 9)),
            other => panic!("expected copy of %9, got {other:?}"),
        }
    }
}
