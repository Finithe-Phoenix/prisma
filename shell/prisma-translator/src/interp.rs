//! A small reference interpreter for the integer + control-flow IR subset.
//!
//! This executes a block's optimized IR directly against a guest register file,
//! independent of the ARM64 backend. Its purpose is differential diagnosis: if a
//! program produces the wrong result on real ARM64 but the interpreter (running
//! the SAME decoded + optimized IR) produces the right one, the defect is in
//! lowering/codegen; if the interpreter is also wrong, the defect is in the
//! decode/optimize pipeline. It is deliberately limited to the ops a
//! straight-line integer block uses; an unsupported op stops the block.
//!
//! Flags are modelled the ARM64 way (NZCV from a `SUBS`-style compare), since
//! that is what `CmpFlags`/`AluFlags`/`CondJumpRel` lower to.

use std::collections::HashMap;

use prisma_ir::{BinOpKind, CondCode, Gpr, Op, OpSize, Ref, Stmt};

/// Guest integer register file (x86 GPR order, matching `CpuStateFrame::gpr`).
#[derive(Debug, Clone, Default)]
pub struct GuestRegs {
    pub gpr: [u64; 16],
}

/// NZCV condition flags, as an `SUBS` would set them. The four flags are the
/// architectural NZCV bits — modelling them as one bool each mirrors the ISA.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    n: bool,
    z: bool,
    c: bool,
    v: bool,
}

/// How interpreting a block ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockOutcome {
    /// A relative branch (`JumpRel`/`CondJumpRel`) resolved to this guest PC.
    Branch(u64),
    /// The block ended on a `SYSCALL`.
    Syscall,
    /// The block ended on a `ret` / indirect transfer (a dynamic target).
    DynamicTransfer,
    /// The block had no terminator (cut at the instruction budget).
    Fallthrough,
    /// An op outside the interpreted subset was reached.
    Unsupported(&'static str),
}

fn mask(value: u64, size: OpSize) -> u64 {
    match size {
        OpSize::I64 => value,
        _ => value & ((1u64 << size.bit_width()) - 1),
    }
}

fn msb(value: u64, size: OpSize) -> bool {
    (value >> (size.bit_width() - 1)) & 1 == 1
}

/// NZCV for `a - b` at `size`, exactly as ARM64 `SUBS` would compute them.
fn sub_flags(a: u64, b: u64, size: OpSize) -> Flags {
    let a = mask(a, size);
    let b = mask(b, size);
    let result = mask(a.wrapping_sub(b), size);
    Flags {
        n: msb(result, size),
        z: result == 0,
        c: a >= b, // no borrow
        v: msb(a, size) != msb(b, size) && msb(result, size) != msb(a, size),
    }
}

/// NZCV for `a + b` at `size`, as ARM64 `ADDS` would compute them.
fn add_flags(a: u64, b: u64, size: OpSize) -> Flags {
    let a = mask(a, size);
    let b = mask(b, size);
    let result = mask(a.wrapping_add(b), size);
    Flags {
        n: msb(result, size),
        z: result == 0,
        c: result < a, // unsigned carry-out
        v: msb(a, size) == msb(b, size) && msb(result, size) != msb(a, size),
    }
}

/// Logical-op flags (AND/OR/XOR): N/Z from the result, C and V cleared — x86
/// logical-op flag semantics, which the lowerer mirrors.
fn logic_flags(result: u64, size: OpSize) -> Flags {
    Flags {
        n: msb(result, size),
        z: mask(result, size) == 0,
        c: false,
        v: false,
    }
}

fn eval_cc(cc: CondCode, f: Flags) -> bool {
    match cc {
        CondCode::Eq => f.z,
        CondCode::Ne => !f.z,
        CondCode::Uge | CondCode::Nc => f.c,
        CondCode::Ult | CondCode::Cc => !f.c,
        CondCode::Ugt => f.c && !f.z,
        CondCode::Ule => !f.c || f.z,
        CondCode::Sge => f.n == f.v,
        CondCode::Slt => f.n != f.v,
        CondCode::Sgt => !f.z && (f.n == f.v),
        CondCode::Sle => f.z || (f.n != f.v),
        CondCode::Mi => f.n,
        CondCode::Pl => !f.n,
        CondCode::Ov => f.v,
        CondCode::NoOv => !f.v,
    }
}

fn store_reg(regs: &mut GuestRegs, reg: Gpr, value: u64, size: OpSize) {
    let idx = reg as usize;
    let v = mask(value, size);
    regs.gpr[idx] = match size {
        // 32-bit writes zero-extend to 64; 8/16-bit writes preserve the upper.
        OpSize::I32 | OpSize::I64 => v,
        OpSize::I16 => (regs.gpr[idx] & !0xFFFF) | v,
        OpSize::I8 => (regs.gpr[idx] & !0xFF) | v,
    };
}

fn eval_binop(op: BinOpKind, a: u64, b: u64, size: OpSize) -> Option<u64> {
    let r = match op {
        BinOpKind::Add => a.wrapping_add(b),
        BinOpKind::Sub => a.wrapping_sub(b),
        BinOpKind::And => a & b,
        BinOpKind::Or => a | b,
        BinOpKind::Xor => a ^ b,
        BinOpKind::Mul => a.wrapping_mul(b),
        BinOpKind::Shl => a.wrapping_shl((b & 63) as u32),
        BinOpKind::Shr => a.wrapping_shr((b & 63) as u32),
        _ => return None,
    };
    Some(mask(r, size))
}

/// Interpret one block's statements against `regs`, returning how it ended.
///
/// Operates on the optimized IR (post decode + renumber + pipeline) so it sees
/// exactly what the backend would lower.
#[must_use]
pub fn interpret_block(stmts: &[Stmt], regs: &mut GuestRegs) -> BlockOutcome {
    let mut vals: HashMap<Ref, u64> = HashMap::new();
    let mut flags = Flags::default();
    let get = |vals: &HashMap<Ref, u64>, r: Ref| vals.get(&r).copied().unwrap_or(0);

    for stmt in stmts {
        match &stmt.op {
            Op::Constant(c) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, mask(c.value, c.size));
                }
            }
            Op::LoadReg(l) => {
                if let Some(d) = stmt.result {
                    vals.insert(d, mask(regs.gpr[l.reg as usize], l.size));
                }
            }
            Op::StoreReg(s) => store_reg(regs, s.reg, get(&vals, s.value), s.size),
            Op::BinOp(b) => {
                let Some(r) = eval_binop(b.op, get(&vals, b.lhs), get(&vals, b.rhs), b.size) else {
                    return BlockOutcome::Unsupported("binop kind");
                };
                if let Some(d) = stmt.result {
                    vals.insert(d, r);
                }
            }
            Op::CmpFlags(c) => {
                flags = sub_flags(get(&vals, c.lhs), get(&vals, c.rhs), c.size);
            }
            Op::AluFlags(a) => {
                let (l, r) = (get(&vals, a.lhs), get(&vals, a.rhs));
                flags = match a.op {
                    BinOpKind::Sub => sub_flags(l, r, a.size),
                    BinOpKind::Add => add_flags(l, r, a.size),
                    BinOpKind::And => logic_flags(l & r, a.size),
                    BinOpKind::Or => logic_flags(l | r, a.size),
                    BinOpKind::Xor => logic_flags(l ^ r, a.size),
                    _ => return BlockOutcome::Unsupported("aluflags kind"),
                };
            }
            Op::CondJumpRel(j) => {
                return if eval_cc(j.cc, flags) {
                    BlockOutcome::Branch(j.target_guest_pc)
                } else {
                    BlockOutcome::Branch(j.fallthrough_guest_pc)
                };
            }
            Op::JumpRel(j) => return BlockOutcome::Branch(j.target_guest_pc),
            Op::Syscall(_) => return BlockOutcome::Syscall,
            Op::Return(_) | Op::JumpReg(_) | Op::CallReg(_) => {
                return BlockOutcome::DynamicTransfer;
            }
            _ => return BlockOutcome::Unsupported("op"),
        }
    }
    BlockOutcome::Fallthrough
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{CmpFlags, Constant, LoadReg, Stmt, StoreReg};

    #[test]
    fn cmp_then_branch_takes_when_nonzero() {
        // ecx loaded, cmp ecx,0, jnz: with ecx=4 -> taken.
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rcx,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::CmpFlags(CmpFlags {
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::CondJumpRel(prisma_ir::CondJumpRel {
                    cc: CondCode::Ne,
                    target_guest_pc: 0x100,
                    fallthrough_guest_pc: 0x200,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rcx as usize] = 4;
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x100)
        );
        regs.gpr[Gpr::Rcx as usize] = 0;
        assert_eq!(
            interpret_block(&stmts, &mut regs),
            BlockOutcome::Branch(0x200)
        );
    }

    #[test]
    fn store_reg_i32_zero_extends() {
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rcx,
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
        ];
        let mut regs = GuestRegs::default();
        regs.gpr[Gpr::Rcx as usize] = 0xFFFF_FFFF_0000_0000;
        let _ = interpret_block(&stmts, &mut regs);
        assert_eq!(regs.gpr[Gpr::Rcx as usize], 5, "32-bit write zero-extends");
    }
}
