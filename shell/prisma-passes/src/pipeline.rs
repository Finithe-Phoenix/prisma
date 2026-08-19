//! Function-level optimization pipeline.

use crate::{
    algebraic::Algebraic, branch_fold::BranchFold, const_prop::ConstantProp, copy_prop::CopyProp,
    cse::Cse, dce::Dce, dead_store::DeadStore, flag_write_elim::FlagWriteElim,
    global_cse::GlobalCse, licm::Licm, peephole::Peephole, redundant_load::RedundantLoad,
    strength_reduce::StrengthReduce, x87_stack::X87Stack, Pass,
};
use prisma_ir::Function;
use std::time::{Duration, Instant};

#[cfg(all(windows, target_arch = "arm64ec"))]
fn arm64ec_phase_marker(message: &'static [u8]) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
        fn WriteFile(
            file: *mut std::ffi::c_void,
            buffer: *const std::ffi::c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }

    const STD_ERROR_HANDLE: u32 = (-12_i32) as u32;
    let Ok(bytes_to_write) = u32::try_from(message.len()) else {
        return;
    };
    // SAFETY: diagnostics borrow a static message and never own the handle.
    let file = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if file.is_null() || file.addr() == usize::MAX {
        return;
    }
    let mut bytes_written = 0_u32;
    let _ = unsafe {
        WriteFile(
            file,
            message.as_ptr().cast(),
            bytes_to_write,
            &raw mut bytes_written,
            std::ptr::null_mut(),
        )
    };
}

#[cfg(all(windows, target_arch = "arm64ec"))]
const PASS_ENTER_MARKERS: [&[u8]; DEFAULT_PIPELINE_LEN] = [
    b"prisma-phase: pass-01-enter\n",
    b"prisma-phase: pass-02-enter\n",
    b"prisma-phase: pass-03-enter\n",
    b"prisma-phase: pass-04-enter\n",
    b"prisma-phase: pass-05-enter\n",
    b"prisma-phase: pass-06-enter\n",
    b"prisma-phase: pass-07-enter\n",
    b"prisma-phase: pass-08-enter\n",
    b"prisma-phase: pass-09-enter\n",
    b"prisma-phase: pass-10-enter\n",
    b"prisma-phase: pass-11-enter\n",
    b"prisma-phase: pass-12-enter\n",
    b"prisma-phase: pass-13-enter\n",
];

#[cfg(all(windows, target_arch = "arm64ec"))]
const PASS_READY_MARKERS: [&[u8]; DEFAULT_PIPELINE_LEN] = [
    b"prisma-phase: pass-01-ready\n",
    b"prisma-phase: pass-02-ready\n",
    b"prisma-phase: pass-03-ready\n",
    b"prisma-phase: pass-04-ready\n",
    b"prisma-phase: pass-05-ready\n",
    b"prisma-phase: pass-06-ready\n",
    b"prisma-phase: pass-07-ready\n",
    b"prisma-phase: pass-08-ready\n",
    b"prisma-phase: pass-09-ready\n",
    b"prisma-phase: pass-10-ready\n",
    b"prisma-phase: pass-11-ready\n",
    b"prisma-phase: pass-12-ready\n",
    b"prisma-phase: pass-13-ready\n",
];

/// Per-pass timing and the total, produced by [`PassPipeline::run_with_stats`].
/// Mirrors the C++ `PassRunStats` shape (name + elapsed per pass).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PipelineStats {
    pub pass_times: Vec<(&'static str, Duration)>,
    pub total_time: Duration,
}

/// Ordered list of optimization passes.
#[derive(Default)]
pub struct PassPipeline {
    passes: Vec<Box<dyn Pass>>,
}

impl PassPipeline {
    /// Run all registered passes in order.
    pub fn run(&self, mut func: Function) -> Function {
        for (index, pass) in self.passes.iter().enumerate() {
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Some(marker) = PASS_ENTER_MARKERS.get(index) {
                arm64ec_phase_marker(marker);
            }
            func = pass.run(func);
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Some(marker) = PASS_READY_MARKERS.get(index) {
                arm64ec_phase_marker(marker);
            }
        }
        func
    }

    /// Run all passes, recording per-pass and total elapsed time.
    #[must_use]
    pub fn run_with_stats(&self, mut func: Function) -> (Function, PipelineStats) {
        let mut stats = PipelineStats::default();
        let start = Instant::now();
        for pass in &self.passes {
            let t0 = Instant::now();
            func = pass.run(func);
            stats.pass_times.push((pass.name(), t0.elapsed()));
        }
        stats.total_time = start.elapsed();
        (func, stats)
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
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 2,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(None, Op::Return(Return)),
                ],
            }],
        };
        let out = default_pipeline().run(func);
        let stmts = &out.blocks[0].stmts;
        // r2 is now a folded constant 8, dead operands removed.
        let folded_8 = stmts
            .iter()
            .any(|s| s.result == Some(2) && matches!(&s.op, Op::Constant(c) if c.value == 8));
        assert!(folded_8, "expected r2 folded to 8: {stmts:?}");
        assert!(stmts.iter().any(|s| matches!(s.op, Op::StoreReg(_))));
        assert!(stmts.iter().any(|s| matches!(s.op, Op::Return(_))));
    }

    #[test]
    fn run_with_stats_records_every_pass() {
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
                    Stmt::new(None, Op::Return(Return)),
                ],
            }],
        };
        let pipeline = default_pipeline();
        let (out_stats, stats) = pipeline.run_with_stats(func.clone());
        let out_plain = pipeline.run(func);
        // Same result whether or not stats are recorded.
        assert_eq!(out_stats, out_plain);
        // One timing entry per pass, names in pipeline order.
        assert_eq!(stats.pass_times.len(), DEFAULT_PIPELINE_LEN);
        let names: Vec<&str> = stats.pass_times.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, pipeline.pass_names());
    }

    #[test]
    fn pipeline_is_idempotent_on_fixed_point() {
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
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 0,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(None, Op::Return(Return)),
                ],
            }],
        };
        let once = default_pipeline().run(func);
        let twice = default_pipeline().run(once.clone());
        assert_eq!(once, twice);
    }
}
