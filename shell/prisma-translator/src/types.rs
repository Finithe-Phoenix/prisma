pub mod interp;

use prisma_backend::lowerer::{LowerError, Lowerer};
use prisma_cache::cache::{fnv1a_64, LookupResult};
use prisma_cache::{CacheEntry, TranslationCache};
use prisma_decoder::decode::{decode_one_at, Decoded};
use prisma_decoder::DecodeError;
use prisma_ir::{BasicBlock, Function, Op};
use prisma_passes::pipeline::{default_pipeline, PassPipeline};

/// A translated guest instruction: the ARM64 machine code plus how many guest
/// bytes it covered and whether it came from the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub code: Vec<u8>,
    pub guest_bytes: usize,
    pub from_cache: bool,
}

/// A translated straight-line block: the concatenated ARM64 code and how it
/// ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTranslation {
    pub code: Vec<u8>,
    pub instruction_count: usize,
    pub guest_bytes: usize,
    /// True if the block ended on a control-transfer instruction (rather than
    /// the byte budget, instruction cap, or a mid-run decode failure).
    pub ended_at_terminator: bool,
    /// Statically-known successor guest PCs: the relative-branch/call targets
    /// and fall-through of the terminator (or just the fall-through PC if the
    /// block ended without one). Empty for a dynamic transfer (indirect
    /// jump/call, return) whose target is only known at run time. The
    /// run loop walks these to translate the reachable CFG ahead of execution.
    pub successors: Vec<u64>,
}

/// A decoded + optimized fused block, before lowering — the IR the backend
/// lowers and the reference interpreter executes.
#[derive(Debug, Clone)]
pub struct OptimizedBlock {
    pub func: Function,
    pub instruction_count: usize,
    pub guest_bytes: usize,
    pub ended_at_terminator: bool,
    pub successors: Vec<u64>,
}

/// Statically-known successor guest PCs of a terminating instruction's `op`.
///
/// Relative branches and calls carry their targets in the IR; an indirect
/// jump/call, a return, or a block-indexed jump is a dynamic transfer with no
/// statically-known successor (empty), to be resolved at run time.
fn static_successors(op: &Op) -> Vec<u64> {
    match op {
        Op::JumpRel(j) => vec![j.target_guest_pc],
        Op::CondJumpRel(c) => vec![c.target_guest_pc, c.fallthrough_guest_pc],
        Op::CallRel(c) => vec![c.target_guest_pc, c.return_guest_pc],
        _ => Vec::new(),
    }
}

/// The static successor guest PCs of the straight-line block at `guest_addr`,
/// found by DECODING only — no optimization or lowering.
///
/// This walks the control-flow graph independently of whether the backend can
/// yet lower the block (e.g. a relative-branch terminator that
/// [`Translator::translate_block`] would reject): CFG discovery only needs the
/// decoded terminator's targets. Returns the terminator's static successors, or
/// the fall-through PC if the block ran to the cap/end without one, or empty if
/// nothing decoded.
#[must_use]
pub fn decode_block_successors(guest_addr: u64, bytes: &[u8], max_insns: usize) -> Vec<u64> {
    let mut offset = 0usize;
    let mut pc = guest_addr;
    let mut count = 0usize;
    while offset < bytes.len() && count < max_insns {
        let Ok(decoded) = decode_one_at(bytes, offset, pc) else {
            break;
        };
        let Some(end) = offset.checked_add(decoded.bytes_consumed) else {
            break;
        };
        if end > bytes.len() {
            break; // decoder over-ran the buffer (truncated instruction)
        }
        if let Some(term) = decoded.stmts.iter().find(|s| is_terminator(&s.op)) {
            return static_successors(&term.op);
        }
        offset = end;
        pc = pc.wrapping_add(decoded.bytes_consumed as u64);
        count += 1;
    }
    if count > 0 {
        vec![pc] // fall-through: the PC after the last decoded instruction
    } else {
        Vec::new()
    }
}

/// Whether `op` transfers control and therefore ends a basic block.
fn is_terminator(op: &Op) -> bool {
    matches!(
        op,
        Op::Return(_)
            | Op::Jump(_)
            | Op::JumpReg(_)
            | Op::JumpRel(_)
            | Op::CondJump(_)
            | Op::CondJumpRel(_)
            | Op::CondJumpFlags(_)
            | Op::CallRel(_)
            | Op::CallReg(_)
            | Op::RetAdjusted(_)
            | Op::Trap(_)
            | Op::Syscall(_)
    )
}

/// Errors from the translation pipeline.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslateError {
    #[error("decode failed: {0:?}")]
    Decode(DecodeError),
    #[error("lowering failed: {0:?}")]
    Lower(LowerError),
    #[error("instruction at offset {offset} reports {consumed} bytes but only {remaining} remain")]
    Truncated {
        offset: usize,
        consumed: usize,
        remaining: usize,
    },
}

/// Cumulative translator counters, useful for profiling the dispatch loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranslatorStats {
    /// Translations served from the cache without re-running the pipeline.
    pub cache_hits: u64,
    /// Translations that missed the cache and ran the full pipeline.
    pub cache_misses: u64,
}

impl TranslatorStats {
    /// Total translation requests served (hits + misses).
    pub const fn total(self) -> u64 {
        self.cache_hits + self.cache_misses
    }
}

/// The integrated decode -> optimize -> lower -> cache pipeline.
pub struct Translator {
    cache: TranslationCache,
    pipeline: PassPipeline,
    stats: TranslatorStats,
}

impl Default for Translator {
    fn default() -> Self {
        Self {
            cache: TranslationCache::new(),
            pipeline: default_pipeline(),
            stats: TranslatorStats::default(),
        }
    }
}
