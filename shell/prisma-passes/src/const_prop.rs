//! Constant propagation pass.
//!
//! This is a first-pass forward fold pass mirroring C++'s `constant_propagate`:
//! it folds `Constant + Constant` and trivial `Extend`/`Truncate` chains when
//! both source operands are known constants.

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOpKind, Constant, Extend, Function, Op, OpSize, Stmt, Truncate};

use crate::Pass;

fn eval_popcnt(v: u64, size: OpSize) -> u64 {
    (v & size.mask()).count_ones() as u64
}

fn eval_lzcnt(v: u64, size: OpSize) -> u64 {
    let masked = v & size.mask();
    if masked == 0 {
        u64::from(size.bit_width())
    } else {
        u64::from(masked.leading_zeros() - (64 - size.bit_width()))
    }
}

fn eval_tzcnt(v: u64, size: OpSize) -> u64 {
    let masked = v & size.mask();
    if masked == 0 {
        u64::from(size.bit_width())
    } else {
        u64::from(masked.trailing_zeros())
    }
}

fn eval_binop(op: BinOpKind, a: u64, b: u64) -> u64 {
    match op {
        BinOpKind::Add => a.wrapping_add(b),
        BinOpKind::Sub => a.wrapping_sub(b),
        BinOpKind::Mul => a.wrapping_mul(b),
        BinOpKind::And => a & b,
        BinOpKind::Or => a | b,
        BinOpKind::Xor => a ^ b,
        BinOpKind::Shl => a.wrapping_shl((b & 0x3f) as u32),
        BinOpKind::Shr => a.wrapping_shr((b & 0x3f) as u32),
        BinOpKind::Sar => (a as i64).wrapping_shr((b & 0x3f) as u32) as u64,
        BinOpKind::Rol => {
            let n = (b & 0x3f) as u32;
            if n == 0 {
                a
            } else {
                a.rotate_left(n)
            }
        }
        BinOpKind::Ror => {
            let n = (b & 0x3f) as u32;
            if n == 0 {
                a
            } else {
                a.rotate_right(n)
            }
        }
        BinOpKind::Rcl => {
            let n = (b & 0x3f) as u32;
            if n == 0 {
                a
            } else {
                (a << n) | (a >> (64 - n))
            }
        }
        BinOpKind::Rcr => {
            let n = (b & 0x3f) as u32;
            if n == 0 {
                a
            } else {
                (a >> n) | (a << (64 - n))
            }
        }
        BinOpKind::UMulHi => {
            let product = (a as u128) * (b as u128);
            (product >> 64) as u64
        }
        BinOpKind::SMulHi => {
            let product = (a as i128) * (b as i128);
            (product >> 64) as u64
        }
        BinOpKind::UDiv => {
            if b == 0 {
                0
            } else {
                a / b
            }
        }
        BinOpKind::SDiv => {
            let lhs = a as i64;
            let rhs = b as i64;
            if rhs == 0 {
                0
            } else if lhs == i64::MIN && rhs == -1 {
                lhs as u64
            } else {
                (lhs / rhs) as u64
            }
        }
        BinOpKind::UMod => {
            if b == 0 {
                a
            } else {
                a % b
            }
        }
        BinOpKind::SMod => {
            let lhs = a as i64;
            let rhs = b as i64;
            if rhs == 0 {
                lhs as u64
            } else if lhs == i64::MIN && rhs == -1 {
                0
            } else {
                (lhs % rhs) as u64
            }
        }
        BinOpKind::Pdep => bit_deposit(a, b),
        BinOpKind::Pext => bit_extract(a, b),
    }
}

fn bit_deposit(src: u64, mask: u64) -> u64 {
    let mut out = 0;
    let mut remaining = mask;
    let mut bit = 1u64;
    let mut value = src;
    while remaining != 0 {
        let low = remaining & remaining.wrapping_neg();
        if (value & 1) != 0 {
            out |= low;
        }
        value >>= 1;
        remaining &= remaining - 1;
        bit <<= 1;
    }
    out
}

fn bit_extract(src: u64, mask: u64) -> u64 {
    let mut out = 0u64;
    let mut i = 0u32;
    let mut remaining = mask;
    while remaining != 0 {
        let low = remaining & remaining.wrapping_neg();
        if (src & low) != 0 {
            out |= 1u64 << i;
        }
        remaining &= remaining - 1;
        i += 1;
    }
    out
}

fn sign_extend(v: u64, from: OpSize, to: OpSize) -> u64 {
    let masked = v & from.mask();
    if from.bit_width() >= 64 {
        return masked & to.mask();
    }
    let sign_bit = 1u64 << (from.bit_width() - 1);
    let out = if (masked & sign_bit) != 0 {
        masked | !((1u64 << from.bit_width()) - 1)
    } else {
        masked
    };
    out & to.mask()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConstantProp;

impl ConstantProp {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for ConstantProp {
    fn name(&self) -> &'static str {
        "constant_propagate"
    }

    fn run(&self, func: Function) -> Function {
        constant_propagate(func)
    }
}

/// Fold constants per basic block.
#[allow(clippy::too_many_lines)]
pub fn constant_propagate(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut consts: HashMap<u32, u64> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                let next = match stmt.result {
                    Some(result) => match stmt.op {
                        Op::Constant(c) => {
                            let value = c.value & c.size.mask();
                            consts.insert(result, value);
                            Stmt {
                                result: Some(result),
                                op: Op::Constant(Constant {
                                    value,
                                    size: c.size,
                                }),
                            }
                        }
                        Op::BinOp(ref b) => {
                            let lhs = consts.get(&b.lhs).copied();
                            let rhs = consts.get(&b.rhs).copied();
                            if let (Some(a), Some(bv)) = (lhs, rhs) {
                                let value = eval_binop(b.op, a, bv) & b.size.mask();
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: b.size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        Op::Extend(ref e) => {
                            if let Some(v) = consts.get(&e.value).copied() {
                                let value = if e.is_signed {
                                    sign_extend(v, e.from_size, e.to_size)
                                } else {
                                    (v & e.from_size.mask()) & e.to_size.mask()
                                };
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: e.to_size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        Op::Truncate(ref t) => {
                            if let Some(v) = consts.get(&t.value).copied() {
                                let value = v & t.to_size.mask();
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: t.to_size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        Op::Popcnt(ref p) => {
                            if let Some(v) = consts.get(&p.value).copied() {
                                let value = eval_popcnt(v, p.size);
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: p.size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        Op::Lzcnt(ref l) => {
                            if let Some(v) = consts.get(&l.value).copied() {
                                let value = eval_lzcnt(v, l.size);
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: l.size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        Op::Tzcnt(ref t) => {
                            if let Some(v) = consts.get(&t.value).copied() {
                                let value = eval_tzcnt(v, t.size);
                                consts.insert(result, value);
                                Stmt {
                                    result: Some(result),
                                    op: Op::Constant(Constant {
                                        value,
                                        size: t.size,
                                    }),
                                }
                            } else {
                                stmt
                            }
                        }
                        _ => stmt,
                    },
                    None => stmt,
                };

                out.push(next);
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
    use prisma_ir::{BinOp, Lzcnt, Popcnt, Tzcnt};

    #[test]
    fn fold_add() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 5,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Constant(Constant {
                            value: 3,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(2),
                        Op::BinOp(BinOp {
                            op: BinOpKind::Add,
                            lhs: 0,
                            rhs: 1,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            }],
        };
        let out = constant_propagate(func);
        match &out.blocks[0].stmts[2].op {
            Op::Constant(c) => assert_eq!(c.value, 8),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn fold_extend_then_truncate_chain() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 0xff,
                            size: OpSize::I8,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Extend(Extend {
                            value: 0,
                            from_size: OpSize::I8,
                            to_size: OpSize::I64,
                            is_signed: true,
                        }),
                    ),
                    Stmt::new(
                        Some(2),
                        Op::Truncate(Truncate {
                            value: 1,
                            to_size: OpSize::I32,
                        }),
                    ),
                ],
            }],
        };

        let out = constant_propagate(func);
        match &out.blocks[0].stmts[2].op {
            Op::Constant(c) => assert_eq!(c.value, 0xffff_ffff),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn fold_bit_count_ops_with_size_masks() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 0x1ff,
                            size: OpSize::I16,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Popcnt(Popcnt {
                            value: 0,
                            size: OpSize::I8,
                        }),
                    ),
                    Stmt::new(
                        Some(2),
                        Op::Lzcnt(Lzcnt {
                            value: 0,
                            size: OpSize::I16,
                        }),
                    ),
                    Stmt::new(
                        Some(3),
                        Op::Tzcnt(Tzcnt {
                            value: 0,
                            size: OpSize::I16,
                        }),
                    ),
                    Stmt::new(
                        Some(4),
                        Op::Constant(Constant {
                            value: 0,
                            size: OpSize::I32,
                        }),
                    ),
                    Stmt::new(
                        Some(5),
                        Op::Lzcnt(Lzcnt {
                            value: 4,
                            size: OpSize::I32,
                        }),
                    ),
                    Stmt::new(
                        Some(6),
                        Op::Tzcnt(Tzcnt {
                            value: 4,
                            size: OpSize::I32,
                        }),
                    ),
                ],
            }],
        };

        let out = constant_propagate(func);
        let folded_constant = |idx: usize| match &out.blocks[0].stmts[idx].op {
            Op::Constant(c) => c.value,
            other => panic!("unexpected: {other:?}"),
        };

        assert_eq!(folded_constant(1), 8);
        assert_eq!(folded_constant(2), 7);
        assert_eq!(folded_constant(3), 0);
        assert_eq!(folded_constant(5), 32);
        assert_eq!(folded_constant(6), 32);
    }
}
