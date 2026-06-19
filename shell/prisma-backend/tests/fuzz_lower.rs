//! Property-based robustness fuzzing for the IR -> ARM64 lowerer.
//!
//! Generates well-formed single-block SSA functions and asserts the lowerer
//! is robust on them: lowering terminates with `Ok(code)`/`Err(LowerError)`
//! on any valid input — it never panics — and is a pure function of the
//! function it is given. Unsupported/unencodable shapes are surfaced as
//! typed `LowerError`s, never crashes. Closes the robustness-fuzz set
//! alongside decoder, cache, and passes.

use proptest::prelude::*;

use prisma_backend::lowerer::Lowerer;
use prisma_ir::{
    BasicBlock, BinOp, BinOpKind, Compare, CondCode, Constant, Function, Gpr, LoadReg, Op, OpSize,
    Return, Stmt, StoreReg,
};

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

const BINOPS: [BinOpKind; 9] = [
    BinOpKind::Add,
    BinOpKind::Sub,
    BinOpKind::Mul,
    BinOpKind::And,
    BinOpKind::Or,
    BinOpKind::Xor,
    BinOpKind::Shl,
    BinOpKind::Shr,
    BinOpKind::Sar,
];

const CCS: [CondCode; 6] = [
    CondCode::Eq,
    CondCode::Ne,
    CondCode::Ult,
    CondCode::Uge,
    CondCode::Slt,
    CondCode::Sge,
];

const SIZES: [OpSize; 4] = [OpSize::I8, OpSize::I16, OpSize::I32, OpSize::I64];

#[derive(Debug, Clone)]
enum Spec {
    Const(u64, u8),
    Load(u8, u8),
    Bin(u8, u8, u8, u8),
    Cmp(u8, u8, u8, u8),
    Store(u8, u8),
}

fn spec() -> impl Strategy<Value = Spec> {
    prop_oneof![
        (any::<u64>(), any::<u8>()).prop_map(|(v, s)| Spec::Const(v, s)),
        (any::<u8>(), any::<u8>()).prop_map(|(r, s)| Spec::Load(r, s)),
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(o, l, r, s)| Spec::Bin(o, l, r, s)),
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(c, l, r, s)| Spec::Cmp(c, l, r, s)),
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
            Spec::Load(reg, sz) => {
                stmts.push(Stmt::new(
                    Some(next),
                    Op::LoadReg(LoadReg {
                        reg: GPRS[*reg as usize % GPRS.len()],
                        size: SIZES[*sz as usize % SIZES.len()],
                    }),
                ));
                defined.push(next);
                next += 1;
            }
            Spec::Bin(op, l, r, sz) if !defined.is_empty() => {
                stmts.push(Stmt::new(
                    Some(next),
                    Op::BinOp(BinOp {
                        op: BINOPS[*op as usize % BINOPS.len()],
                        lhs: pick(&defined, *l),
                        rhs: pick(&defined, *r),
                        size: SIZES[*sz as usize % SIZES.len()],
                    }),
                ));
                defined.push(next);
                next += 1;
            }
            Spec::Cmp(cc, l, r, sz) if !defined.is_empty() => {
                stmts.push(Stmt::new(
                    Some(next),
                    Op::Compare(Compare {
                        cc: CCS[*cc as usize % CCS.len()],
                        lhs: pick(&defined, *l),
                        rhs: pick(&defined, *r),
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
            Spec::Bin(..) | Spec::Cmp(..) | Spec::Store(..) => {}
        }
    }

    stmts.push(Stmt::new(None, Op::Return(Return)));
    Function {
        entry: 0,
        blocks: vec![BasicBlock { id: 0, stmts }],
    }
}

proptest! {
    /// Lowering a valid SSA function never panics; unsupported or unencodable
    /// shapes come back as typed `LowerError`s.
    #[test]
    fn lower_never_panics(specs in prop::collection::vec(spec(), 0..48)) {
        let _ = Lowerer::new().lower_function(&build(&specs));
    }

    /// Lowering is a pure function of its input.
    #[test]
    fn lower_is_deterministic(specs in prop::collection::vec(spec(), 0..48)) {
        let func = build(&specs);
        prop_assert_eq!(
            Lowerer::new().lower_function(&func),
            Lowerer::new().lower_function(&func)
        );
    }
}
