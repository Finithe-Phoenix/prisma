//! Function-level optimization pipeline.

use crate::{
    algebraic::Algebraic, branch_fold::BranchFold, const_prop::ConstantProp, copy_prop::CopyProp,
    cse::Cse, dce::Dce, dead_store::DeadStore, flag_write_elim::FlagWriteElim,
    global_cse::GlobalCse, licm::Licm, peephole::Peephole, redundant_load::RedundantLoad,
    strength_reduce::StrengthReduce, x87_stack::X87Stack, Pass,
};
use prisma_ir::Function;

/// Ordered list of optimization passes.
#[derive(Default)]
pub struct PassPipeline {
    passes: Vec<Box<dyn Pass>>,
}

impl PassPipeline {
    /// Run all registered passes in order.
    pub fn run(&self, mut func: Function) -> Function {
        for pass in &self.passes {
            func = pass.run(func);
        }
        func
    }

    /// Number of passes in this pipeline.
    #[must_use]
    pub fn size(&self) -> usize {
        self.passes.len()
    }

    /// Human-readable list of pass names.
    #[must_use]
    pub fn pass_names(&self) -> Vec<&'static str> {
        self.passes.iter().map(|p| p.name()).collect()
    }
}

/// Number of passes in the default pipeline (mirrors the C++ pass manager).
pub const DEFAULT_PIPELINE_LEN: usize = 13;

/// Return the default Prisma optimization pipeline.
#[must_use]
pub fn default_pipeline() -> PassPipeline {
    PassPipeline {
        // Order matches C++ default_pipeline() in core/src/passes/pass_manager.cpp.
        passes: vec![
            Box::new(ConstantProp::new()),
            Box::new(Algebraic::new()),
            Box::new(StrengthReduce::new()),
            Box::new(Peephole::new()),
            Box::new(ConstantProp::new()),
            Box::new(RedundantLoad::new()),
            Box::new(Cse::new()),
            Box::new(X87Stack::new()),
            Box::new(CopyProp::new()),
            Box::new(DeadStore::new()),
            Box::new(BranchFold::new()),
            Box::new(FlagWriteElim::new()),
            Box::new(Dce::new()),
        ],
    }
}

/// Return the function-level (CFG-aware) pipeline.
///
/// Mirrors C++ `default_function_pipeline()`: `global_cse` collapses
/// duplicate computations along dominator edges first, then
/// `loop_invariant_motion` hoists invariants to loop preheaders.
#[must_use]
pub fn default_function_pipeline() -> PassPipeline {
    PassPipeline {
        passes: vec![Box::new(GlobalCse::new()), Box::new(Licm::new())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{
        BasicBlock, BinOp, BinOpKind, Constant, Function, Gpr, Op, OpSize, Return, Stmt, StoreReg,
    };

    #[test]
    fn default_pipeline_has_expected_passes() {
        let p = default_pipeline();
        assert_eq!(p.size(), DEFAULT_PIPELINE_LEN);
        assert_eq!(
            p.pass_names(),
            vec![
                "constant_propagate",
                "algebraic_simplify",
                "strength_reduce",
                "peephole",
                "constant_propagate",
                "redundant_load_eliminate",
                "common_subexpression_eliminate",
                "x87_stack_eliminate",
                "copy_prop",
                "dead_store_eliminate",
                "branch_fold",
                "flag_write_elimination",
                "dead_code_eliminate",
            ]
        );
    }

    #[test]
    fn pipeline_folds_constant_add_and_dce_keeps_store() {
        // r0=5; r1=3; r2=r0+r1; StoreReg rax, r2
        // -> const-prop folds r2 to 8, then DCE drops the now-dead r0/r1.
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(Some(0), Op::Constant(Constant { value: 5, size: OpSize::I64 })),
                    Stmt::new(Some(1), Op::Constant(Constant { value: 3, size: OpSize::I64 })),
                    Stmt::new(
                        Some(2),
                        Op::BinOp(BinOp { op: BinOpKind::Add, lhs: 0, rhs: 1, size: OpSize::I64 }),
                    ),
                    Stmt::new(None, Op::StoreReg(StoreReg { reg: Gpr::Rax, value: 2, size: OpSize::I64 })),
                    Stmt::new(None, Op::Return(Return)),
                ],
            }],
        };
        let out = default_pipeline().run(func);
        let stmts = &out.blocks[0].stmts;
        // r2 is now a folded constant 8, dead operands removed.
        let folded_8 = stmts.iter().any(|s| {
            s.result == Some(2)
                && matches!(&s.op, Op::Constant(c) if c.value == 8)
        });
        assert!(folded_8, "expected r2 folded to 8: {stmts:?}");
        assert!(stmts.iter().any(|s| matches!(s.op, Op::StoreReg(_))));
        assert!(stmts.iter().any(|s| matches!(s.op, Op::Return(_))));
    }

    #[test]
    fn pipeline_is_idempotent_on_fixed_point() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(Some(0), Op::Constant(Constant { value: 5, size: OpSize::I64 })),
                    Stmt::new(None, Op::StoreReg(StoreReg { reg: Gpr::Rax, value: 0, size: OpSize::I64 })),
                    Stmt::new(None, Op::Return(Return)),
                ],
            }],
        };
        let once = default_pipeline().run(func);
        let twice = default_pipeline().run(once.clone());
        assert_eq!(once, twice);
    }
}
