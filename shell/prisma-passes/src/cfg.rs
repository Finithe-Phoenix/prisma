//! Control-flow graph analysis for `prisma_ir::Function`.
//!
//! Lives in `prisma-passes` (not `prisma-ir`) to keep the shared IR type file
//! untouched. Provides the successor/postorder/dominator/natural-loop
//! primitives that `global_cse` and `licm` need, mirroring the C++
//! `prisma/dominators.hpp` surface.
//!
//! Only intra-function, block-targeting terminators define edges:
//! `Jump{target_block}`, `CondJump{if_true,if_false}`, and
//! `CondJumpFlags{if_true,if_false}`. Guest-PC terminators (`JumpRel`,
//! `CondJumpRel`, `Return`, `JumpReg`, ...) leave the function and contribute
//! no intra-function successors.

use std::collections::{HashMap, HashSet};

use prisma_ir::{Function, Op};

/// Successor block ids of `block_id` (in first-seen order, deduplicated).
#[must_use]
pub fn successors(func: &Function, block_id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let push = |id: u32, out: &mut Vec<u32>| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    let Some(block) = func.blocks.iter().find(|b| b.id == block_id) else {
        return out;
    };
    for stmt in &block.stmts {
        match &stmt.op {
            Op::Jump(j) => push(j.target_block, &mut out),
            Op::CondJump(c) => {
                push(c.if_true, &mut out);
                push(c.if_false, &mut out);
            }
            Op::CondJumpFlags(c) => {
                push(c.if_true, &mut out);
                push(c.if_false, &mut out);
            }
            _ => {}
        }
    }
    out
}

/// Depth-first postorder of the blocks reachable from `func.entry`.
///
/// Dangling jump targets (ids with no matching block — possible when the IR is
/// malformed or only partially built) are ignored, so every id returned is a
/// real block. Downstream passes can index `func.blocks` by these ids safely.
#[must_use]
pub fn postorder(func: &Function) -> Vec<u32> {
    let valid: HashSet<u32> = func.blocks.iter().map(|b| b.id).collect();
    let succs_of = |id: u32| -> Vec<u32> {
        successors(func, id)
            .into_iter()
            .filter(|s| valid.contains(s))
            .collect()
    };

    let mut order = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    // Iterative DFS with an explicit child-cursor stack.
    let mut stack: Vec<(u32, Vec<u32>, usize)> = Vec::new();
    if valid.contains(&func.entry) {
        visited.insert(func.entry);
        stack.push((func.entry, succs_of(func.entry), 0));
    }
    while let Some(frame) = stack.last_mut() {
        let (id, succs, cursor) = frame;
        if *cursor < succs.len() {
            let next = succs[*cursor];
            *cursor += 1;
            if visited.insert(next) {
                let s = succs_of(next);
                stack.push((next, s, 0));
            }
        } else {
            order.push(*id);
            stack.pop();
        }
    }
    order
}

/// Immediate dominators keyed by block id (Cooper-Harvey-Kennedy). The entry
/// block maps to itself. Only reachable blocks appear.
#[must_use]
pub fn dominators(func: &Function) -> HashMap<u32, u32> {
    let po = postorder(func);
    let mut idom: HashMap<u32, u32> = HashMap::new();
    if po.is_empty() {
        return idom;
    }
    // Postorder number per block (entry has the highest number).
    let po_num: HashMap<u32, usize> = po.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    // Reverse postorder for the iteration order.
    let rpo: Vec<u32> = po.iter().rev().copied().collect();

    // Predecessor map restricted to reachable blocks.
    let reachable: HashSet<u32> = po.iter().copied().collect();
    let mut preds: HashMap<u32, Vec<u32>> = HashMap::new();
    for &b in &po {
        for s in successors(func, b) {
            if reachable.contains(&s) {
                preds.entry(s).or_default().push(b);
            }
        }
    }

    let entry = func.entry;
    idom.insert(entry, entry);

    // intersect: walk both fingers up the dominator tree by postorder number.
    let intersect = |mut a: u32, mut b: u32, idom: &HashMap<u32, u32>| -> u32 {
        while a != b {
            while po_num[&a] < po_num[&b] {
                a = idom[&a];
            }
            while po_num[&b] < po_num[&a] {
                b = idom[&b];
            }
        }
        a
    };

    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo {
            if b == entry {
                continue;
            }
            let Some(bpreds) = preds.get(&b) else {
                continue;
            };
            // new_idom = first processed predecessor.
            let mut new_idom: Option<u32> = None;
            for &p in bpreds {
                if idom.contains_key(&p) {
                    new_idom = Some(match new_idom {
                        None => p,
                        Some(cur) => intersect(p, cur, &idom),
                    });
                }
            }
            if let Some(ni) = new_idom {
                if idom.get(&b) != Some(&ni) {
                    idom.insert(b, ni);
                    changed = true;
                }
            }
        }
    }
    idom
}

/// A natural loop: its header block and the set of block ids in its body
/// (header included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalLoop {
    pub header: u32,
    pub body: Vec<u32>,
}

/// True iff `a` dominates `b` (walking idom chain from `b` up to entry).
fn dominates(idom: &HashMap<u32, u32>, a: u32, b: u32) -> bool {
    let mut cur = b;
    loop {
        if cur == a {
            return true;
        }
        match idom.get(&cur) {
            Some(&next) if next != cur => cur = next,
            _ => return false,
        }
    }
}

/// Natural loops discovered from back edges (`u -> v` where `v` dominates
/// `u`). Bodies are computed by the classic reverse-reachability walk from the
/// back-edge tail, stopping at the header.
#[must_use]
pub fn natural_loops(func: &Function) -> Vec<NaturalLoop> {
    let idom = dominators(func);
    let reachable: HashSet<u32> = idom.keys().copied().collect();
    // Predecessor map.
    let mut preds: HashMap<u32, Vec<u32>> = HashMap::new();
    for &b in &reachable {
        for s in successors(func, b) {
            if reachable.contains(&s) {
                preds.entry(s).or_default().push(b);
            }
        }
    }

    // Back-edges that share a header describe the same natural loop; merge
    // their bodies so each header yields exactly one NaturalLoop.
    let mut bodies: HashMap<u32, HashSet<u32>> = HashMap::new();
    let mut header_order: Vec<u32> = Vec::new();
    // Deterministic iteration over blocks in function order.
    for block in &func.blocks {
        let u = block.id;
        if !reachable.contains(&u) {
            continue;
        }
        for v in successors(func, u) {
            // Back edge: header v dominates tail u.
            if reachable.contains(&v) && dominates(&idom, v, u) {
                let body = bodies.entry(v).or_insert_with(|| {
                    header_order.push(v);
                    HashSet::new()
                });
                body.insert(v);
                let mut stack = vec![u];
                while let Some(n) = stack.pop() {
                    if body.insert(n) {
                        if let Some(ps) = preds.get(&n) {
                            for &p in ps {
                                stack.push(p);
                            }
                        }
                    }
                }
            }
        }
    }

    header_order
        .into_iter()
        .map(|header| {
            let body = &bodies[&header];
            // Stable body order: function block order.
            let body_vec: Vec<u32> = func
                .blocks
                .iter()
                .map(|b| b.id)
                .filter(|id| body.contains(id))
                .collect();
            NaturalLoop {
                header,
                body: body_vec,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{BasicBlock, CondJump, Jump, Return, Stmt};

    fn jump(target: u32) -> Stmt {
        Stmt::new(
            None,
            Op::Jump(Jump {
                target_block: target,
            }),
        )
    }
    fn cond(if_true: u32, if_false: u32) -> Stmt {
        Stmt::new(
            None,
            Op::CondJump(CondJump {
                cond: 0,
                if_true,
                if_false,
            }),
        )
    }
    fn ret() -> Stmt {
        Stmt::new(None, Op::Return(Return))
    }
    fn blk(id: u32, stmts: Vec<Stmt>) -> BasicBlock {
        BasicBlock { id, stmts }
    }

    #[test]
    fn successors_reads_terminators() {
        let f = Function {
            entry: 0,
            blocks: vec![blk(0, vec![cond(1, 2)])],
        };
        assert_eq!(successors(&f, 0), vec![1, 2]);
    }

    #[test]
    fn diamond_dominators() {
        // 0 -> {1,2} -> 3
        let f = Function {
            entry: 0,
            blocks: vec![
                blk(0, vec![cond(1, 2)]),
                blk(1, vec![jump(3)]),
                blk(2, vec![jump(3)]),
                blk(3, vec![ret()]),
            ],
        };
        let idom = dominators(&f);
        assert_eq!(idom[&0], 0);
        assert_eq!(idom[&1], 0);
        assert_eq!(idom[&2], 0);
        // 3 has two preds (1,2); its idom is 0 (the join point).
        assert_eq!(idom[&3], 0);
    }

    #[test]
    fn postorder_visits_children_first() {
        let f = Function {
            entry: 0,
            blocks: vec![blk(0, vec![jump(1)]), blk(1, vec![ret()])],
        };
        let po = postorder(&f);
        // child 1 before parent 0
        assert_eq!(po, vec![1, 0]);
    }

    #[test]
    fn simple_loop_detected() {
        // 0 -> 1 ; 1 -> {1 (back), 2} ; 2 -> ret
        let f = Function {
            entry: 0,
            blocks: vec![
                blk(0, vec![jump(1)]),
                blk(1, vec![cond(1, 2)]),
                blk(2, vec![ret()]),
            ],
        };
        let loops = natural_loops(&f);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header, 1);
        assert_eq!(loops[0].body, vec![1]);
    }

    #[test]
    fn two_block_loop_body() {
        // 0 -> 1 ; 1 -> 2 ; 2 -> {1 (back), 3} ; 3 -> ret
        let f = Function {
            entry: 0,
            blocks: vec![
                blk(0, vec![jump(1)]),
                blk(1, vec![jump(2)]),
                blk(2, vec![cond(1, 3)]),
                blk(3, vec![ret()]),
            ],
        };
        let loops = natural_loops(&f);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header, 1);
        assert_eq!(loops[0].body, vec![1, 2]);
    }

    #[test]
    fn two_back_edges_same_header_merge_to_one_loop() {
        // 0 -> 1 ; 1 -> {2,3} ; 2 -> 1 (back) ; 3 -> {1 (back), 4} ; 4 -> ret
        // Two back-edges into header 1 must yield ONE loop with merged body.
        let f = Function {
            entry: 0,
            blocks: vec![
                blk(0, vec![jump(1)]),
                blk(1, vec![cond(2, 3)]),
                blk(2, vec![jump(1)]),
                blk(3, vec![cond(1, 4)]),
                blk(4, vec![ret()]),
            ],
        };
        let loops = natural_loops(&f);
        assert_eq!(loops.len(), 1, "back-edges sharing a header merge");
        assert_eq!(loops[0].header, 1);
        assert_eq!(loops[0].body, vec![1, 2, 3]);
    }

    #[test]
    fn dangling_target_does_not_panic() {
        // Jump to a non-existent block id 99 — must be ignored, not panic.
        let f = Function {
            entry: 0,
            blocks: vec![blk(0, vec![jump(99)])],
        };
        assert_eq!(postorder(&f), vec![0]);
        assert_eq!(dominators(&f).get(&0), Some(&0));
        assert!(natural_loops(&f).is_empty());
    }

    #[test]
    fn no_loop_in_acyclic() {
        let f = Function {
            entry: 0,
            blocks: vec![blk(0, vec![jump(1)]), blk(1, vec![ret()])],
        };
        assert!(natural_loops(&f).is_empty());
    }
}
