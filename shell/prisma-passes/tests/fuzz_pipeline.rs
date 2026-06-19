//! Property-based robustness fuzzing for the optimization pipeline.
//!
//! Generates well-formed single-block SSA functions (every operand ref is
//! defined by an earlier statement) and asserts the default pipeline is
//! robust on them: it never panics — including on adversarial constant
//! folds like shift-by->=64 or divide-by-zero — and is a pure function of
//! its input. Complements the hand-written unit tests in `pipeline.rs`.

use proptest::prelude::*;

use prisma_ir::{
    BasicBlock, BinOp, BinOpKind, Constant, Function, Gpr, Op, OpSize, Return, Stmt, StoreReg,
};
use prisma_passes::pipeline::default_pipeline;

const GPRS: [Gpr; 16] = [
    Gpr::Rax,
    Gpr::Rcx,
    Gpr::Rdx,
    Gpr::Rbx,
    Gpr::Rsp,
    Gpr::Rbp,
    Gpr::Rsi,
    Gpr::Rdi,
    Gpr::R8,
    Gpr::R9,
    Gpr::R10,
    Gpr::R11,
    Gpr::R12,
    Gpr::R13,
    Gpr::R14,
    Gpr::R15,
];

const BINOPS: [BinOpKind; 11] = [
    BinOpKind::Add,
    BinOpKind::Sub,
    BinOpKind::Mul,
    BinOpKind::And,
    BinOpKind::Or,
    BinOpKind::Xor,
    BinOpKind::Shl,
    BinOpKind::Shr,
    BinOpKind::Sar,
    BinOpKind::UDiv,
    BinOpKind::UMod,
];

const SIZES: [OpSize; 4] = [OpSize::I8, OpSize::I16, OpSize::I32, OpSize::I64];

/// A statement to materialise. Operand selectors are reduced modulo the live
/// ref count at build time, so the resulting function is always valid SSA.
#[derive(Debug, Clone)]
enum Spec {
    Const(u64, u8),
    Bin(u8, u8, u8, u8),
    Store(u8, u8),
}

fn spec() -> impl Strategy<Value = Spec> {
    prop_oneof![
        (any::<u64>(), any::<u8>()).prop_map(|(v, s)| Spec::Const(v, s)),
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(o, l, r, s)| Spec::Bin(o, l, r, s)),
        (any::<u8>(), any::<u8>()).prop_map(|(r, v)| Spec::Store(r, v)),
    ]
}

fn pick(defined: &[u32], sel: u8) -> u32 {
    defined[sel as usize % defined.len()]
}

fn build(specs: &[Spec]) -> Function {
    let mut stmts = Vec::new();
    let mut defined: Vec<u32> = Vec::new();
    let mut next: u32 = 0;

    for s in specs {
        match s {
            Spec::Const(v, sz) => {
                stmts.push(Stmt::new(
                    Some(next),
                    Op::Constant(Constant {
                        value: *v,
                        size: SIZES[*sz as usize % SIZES.len()],
                    }),
                ));
                defined.push(next);
                next += 1;
            }
            Spec::Bin(op, l, r, sz) if !defined.is_empty() => {
                let lhs = pick(&defined, *l);
                let rhs = pick(&defined, *r);
                stmts.push(Stmt::new(
                    Some(next),
                    Op::BinOp(BinOp {
                        op: BINOPS[*op as usize % BINOPS.len()],
                        lhs,
                        rhs,
                        size: SIZES[*sz as usize % SIZES.len()],
                    }),
                ));
                defined.push(next);
                next += 1;
            }
            Spec::Store(reg, v) if !defined.is_empty() => {
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: GPRS[*reg as usize % GPRS.len()],
                        value: pick(&defined, *v),
                        size: OpSize::I64,
                    }),
                ));
            }
            // Operand-bearing specs before any value is defined are skipped.
            Spec::Bin(..) | Spec::Store(..) => {}
        }
    }

    stmts.push(Stmt::new(None, Op::Return(Return)));
    Function {
        entry: 0,
        blocks: vec![BasicBlock { id: 0, stmts }],
    }
}

proptest! {
    /// The pipeline never panics on a valid SSA function — including
    /// constant folds of shift-by->=64, divide-by-zero, and mod-by-zero.
    #[test]
    fn pipeline_never_panics(specs in prop::collection::vec(spec(), 0..48)) {
        let _ = default_pipeline().run(build(&specs));
    }

    /// The pipeline is a pure function: identical input yields identical output.
    #[test]
    fn pipeline_is_deterministic(specs in prop::collection::vec(spec(), 0..48)) {
        let func = build(&specs);
        let a = default_pipeline().run(func.clone());
        let b = default_pipeline().run(func);
        prop_assert_eq!(a, b);
    }
}
