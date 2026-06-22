// Prisma translation facade — the integrated Rust pipeline.
//
// Combines the decoder, optimization passes, ARM64 lowerer, and translation
// cache into one entry point: bytes in, optimized ARM64 machine code out,
// memoized by (guest_addr, content hash). Mirrors the C++ `prisma_translator`
// facade. `translate` handles one guest instruction; `translate_block` chains a
// straight-line run up to the next control transfer, caching each instruction
// independently. Fusing a block into a single optimized region (which needs
// function-global SSA renumbering across instructions) is the documented
// follow-up.

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::missing_const_for_fn, clippy::must_use_candidate)]

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

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate the guest instruction at `bytes[0..]` (decoded at `guest_addr`)
    /// into ARM64 machine code, running the full optimization pipeline and
    /// memoizing the result in the translation cache.
    ///
    /// # Errors
    /// [`TranslateError::Decode`] if the bytes are not a decodable instruction;
    /// [`TranslateError::Lower`] if the resulting IR is not lowerable by the
    /// current backend slice.
    pub fn translate(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
    ) -> Result<Translation, TranslateError> {
        let decoded = decode_one_at(bytes, 0, guest_addr).map_err(TranslateError::Decode)?;
        self.translate_decoded(guest_addr, bytes, &decoded)
    }

    /// Translate a straight-line run of instructions starting at `guest_addr`
    /// into one concatenated ARM64 block, stopping at the first control-transfer
    /// instruction, when `bytes` is exhausted, or after `max_insns` (a guard
    /// against pathological runs). Each instruction is translated and cached
    /// independently. An undecodable byte mid-run ends the block; an
    /// undecodable first instruction is an error.
    ///
    /// # Errors
    /// [`TranslateError`] from the first instruction if it cannot be decoded or
    /// lowered.
    pub fn translate_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        let mut code = Vec::new();
        let mut offset = 0usize;
        let mut pc = guest_addr;
        let mut instruction_count = 0usize;
        let mut ended_at_terminator = false;
        let mut successors: Vec<u64> = Vec::new();

        while offset < bytes.len() && instruction_count < max_insns {
            let decoded = match decode_one_at(bytes, offset, pc) {
                Ok(d) => d,
                Err(e) => {
                    if instruction_count == 0 {
                        return Err(TranslateError::Decode(e));
                    }
                    break;
                }
            };
            // The decoder can report consuming more bytes than remain (a
            // truncated trailing instruction whose operand runs off the
            // buffer). Bound the slice instead of panicking: stop the block if
            // we already have an instruction, else surface a typed error.
            let Some(insn) = offset
                .checked_add(decoded.bytes_consumed)
                .and_then(|end| bytes.get(offset..end))
            else {
                if instruction_count == 0 {
                    return Err(TranslateError::Truncated {
                        offset,
                        consumed: decoded.bytes_consumed,
                        remaining: bytes.len() - offset,
                    });
                }
                break;
            };
            let translation = self.translate_decoded(pc, insn, &decoded)?;
            code.extend_from_slice(&translation.code);
            instruction_count += 1;
            offset += decoded.bytes_consumed;
            pc = pc.wrapping_add(decoded.bytes_consumed as u64);
            if let Some(term) = decoded.stmts.iter().find(|s| is_terminator(&s.op)) {
                successors = static_successors(&term.op);
                ended_at_terminator = true;
                break;
            }
        }

        if !ended_at_terminator && instruction_count > 0 {
            // Block ended on the cap / exhausted bytes: its only successor is
            // the fall-through PC after the last instruction.
            successors = vec![pc];
        }

        Ok(BlockTranslation {
            code,
            instruction_count,
            guest_bytes: offset,
            ended_at_terminator,
            successors,
        })
    }

    /// Like [`Translator::translate_block`], but fuses the whole straight-line
    /// run into a SINGLE optimized SSA region instead of translating each
    /// instruction in isolation. The decoder numbers refs per instruction, so
    /// each instruction's refs are renumbered (via [`prisma_ir::Op::map_refs`])
    /// into a disjoint range before being concatenated; the default pipeline
    /// then optimizes ACROSS instruction boundaries (e.g. forwarding a
    /// register write into a later read) and the result is lowered once.
    ///
    /// Not cached (the unit is a block, not a single instruction). Returns an
    /// empty block for empty input.
    ///
    /// # Errors
    /// [`TranslateError::Decode`] if the first instruction cannot be decoded;
    /// [`TranslateError::Lower`] if the fused region is not lowerable.
    pub fn translate_fused_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        let mut stmts = Vec::new();
        let mut offset = 0usize;
        let mut pc = guest_addr;
        let mut instruction_count = 0usize;
        let mut ended_at_terminator = false;
        let mut successors: Vec<u64> = Vec::new();
        // Next free SSA ref: every instruction's refs are shifted above all
        // refs already placed in the block so names never collide.
        let mut base: u32 = 0;

        while offset < bytes.len() && instruction_count < max_insns {
            let decoded = match decode_one_at(bytes, offset, pc) {
                Ok(d) => d,
                Err(e) => {
                    if instruction_count == 0 {
                        return Err(TranslateError::Decode(e));
                    }
                    break;
                }
            };

            let mut renumbered = decoded.stmts.clone();
            let mut local_max = base;
            let mut overflow = false;
            for stmt in &mut renumbered {
                stmt.map_refs(|r| {
                    r.checked_add(base).map_or_else(
                        || {
                            overflow = true;
                            r
                        },
                        |v| {
                            local_max = local_max.max(v);
                            v
                        },
                    )
                });
            }
            if overflow {
                // Ref space exhausted (pathological run): stop cleanly.
                break;
            }

            stmts.extend(renumbered);
            instruction_count += 1;
            offset += decoded.bytes_consumed;
            pc = pc.wrapping_add(decoded.bytes_consumed as u64);
            base = local_max.saturating_add(1);

            if let Some(term) = decoded.stmts.iter().find(|s| is_terminator(&s.op)) {
                successors = static_successors(&term.op);
                ended_at_terminator = true;
                break;
            }
        }

        if !ended_at_terminator && instruction_count > 0 {
            successors = vec![pc];
        }

        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        };
        let optimized = self.pipeline.run(func);
        // The runtime executes every translated block via execute_block, which
        // wraps it in the AAPCS64 block prologue/epilogue. A terminator (guest
        // ret, SYSCALL) must therefore route through the full epilogue — a bare
        // ret would skip the prologue's stack/callee-saved restore and corrupt
        // the host on return.
        let words = Lowerer::new()
            .with_returns_via_epilogue()
            .lower_function(&optimized)
            .map_err(TranslateError::Lower)?;
        let mut code = Vec::with_capacity(words.len() * 4);
        for word in &words {
            code.extend_from_slice(&word.to_le_bytes());
        }

        Ok(BlockTranslation {
            code,
            instruction_count,
            guest_bytes: offset,
            ended_at_terminator,
            successors,
        })
    }

    fn translate_decoded(
        &mut self,
        guest_addr: u64,
        insn: &[u8],
        decoded: &Decoded,
    ) -> Result<Translation, TranslateError> {
        if let LookupResult::Hit(entry) = self.cache.lookup(guest_addr, insn) {
            self.stats.cache_hits += 1;
            return Ok(Translation {
                code: entry.code_bytes.into_vec(),
                guest_bytes: decoded.bytes_consumed,
                from_cache: true,
            });
        }
        self.stats.cache_misses += 1;

        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: decoded.stmts.clone(),
            }],
        };
        let optimized = self.pipeline.run(func);

        // The runtime executes every translated block via execute_block, which
        // wraps it in the AAPCS64 block prologue/epilogue. A terminator (guest
        // ret, SYSCALL) must therefore route through the full epilogue — a bare
        // ret would skip the prologue's stack/callee-saved restore and corrupt
        // the host on return.
        let words = Lowerer::new()
            .with_returns_via_epilogue()
            .lower_function(&optimized)
            .map_err(TranslateError::Lower)?;
        let mut code = Vec::with_capacity(words.len() * 4);
        for word in &words {
            code.extend_from_slice(&word.to_le_bytes());
        }

        let entry = CacheEntry {
            guest_addr,
            guest_size: u32::try_from(decoded.bytes_consumed).unwrap_or(u32::MAX),
            code_size: u32::try_from(code.len()).unwrap_or(u32::MAX),
            code_bytes: code.clone().into_boxed_slice(),
            hit_count: 0,
            last_used: 0,
        };
        self.cache.insert((guest_addr, fnv1a_64(insn)), entry);

        Ok(Translation {
            code,
            guest_bytes: decoded.bytes_consumed,
            from_cache: false,
        })
    }

    /// Number of distinct translations currently held in the cache.
    pub fn cached_count(&self) -> usize {
        self.cache.entry_count()
    }

    /// Cumulative cache hit/miss counters since construction (or last reset).
    pub const fn stats(&self) -> TranslatorStats {
        self.stats
    }

    /// Reset the hit/miss counters to zero (the cache contents are untouched).
    pub fn reset_stats(&mut self) {
        self.stats = TranslatorStats::default();
    }

    /// Bound the translation cache: at most `max_entries` entries and
    /// `max_bytes` of code (0 means unbounded). LRU eviction enforces both.
    pub fn set_cache_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.cache.set_limits(max_entries, max_bytes);
    }

    /// Drop the cached translation(s) at `guest_addr`. Call this when the guest
    /// rewrites code at that address (self-modifying code) so the next
    /// translation re-decodes the new bytes instead of serving stale code.
    pub fn invalidate(&mut self, guest_addr: u64) {
        // The cache keys on (addr, content hash) but tracks addr -> hash, so a
        // zero-hash key evicts whatever translation currently lives at the addr.
        self.cache.invalidate(&(guest_addr, 0));
    }

    /// Drop every cached translation (e.g. on a full guest address-space flush).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // mov rax, rcx  (REX.W 89 /r)
    const MOV_RAX_RCX: &[u8] = &[0x48, 0x89, 0xC8];
    // add rax, 0x10 (REX.W 83 /0 ib)
    const ADD_RAX_IMM8: &[u8] = &[0x48, 0x83, 0xC0, 0x10];

    #[test]
    fn decode_block_successors_follows_a_jmp_without_lowering() {
        // EB 0E = JMP +0x0E -> targets guest_addr + 2 + 0x0E. Decoding alone
        // finds the target even though the lowerer cannot yet lower JumpRel.
        let succ = decode_block_successors(0x4_0000, &[0xEB, 0x0E], 64);
        assert_eq!(succ, vec![0x4_0000 + 2 + 0x0E]);
        // A straight-line run with no terminator falls through to the next PC.
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);
        let fall = decode_block_successors(0x4_0000, &prog, 64);
        assert_eq!(fall, vec![0x4_0000 + prog.len() as u64]);
    }

    #[test]
    fn static_successors_extracts_relative_targets() {
        use prisma_ir::{CallRel, JumpRel, Op, Return};
        assert_eq!(
            static_successors(&Op::JumpRel(JumpRel {
                target_guest_pc: 0x1234
            })),
            vec![0x1234]
        );
        // A call's successors are the callee and the return site.
        assert_eq!(
            static_successors(&Op::CallRel(CallRel {
                target_guest_pc: 0x2000,
                return_guest_pc: 0x1005,
            })),
            vec![0x2000, 0x1005]
        );
        // A return is a dynamic transfer: no static successor.
        assert!(static_successors(&Op::Return(Return)).is_empty());
    }

    #[test]
    fn straight_line_block_successor_is_the_fall_through_pc() {
        // Two non-terminator instructions, no control transfer: the single
        // successor is the PC just past the block.
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);
        let mut t = Translator::new();
        let block = t.translate_block(0x4_0000, &prog, 64).expect("translate");
        assert!(!block.ended_at_terminator);
        assert_eq!(block.successors, vec![0x4_0000 + prog.len() as u64]);
    }

    #[test]
    fn translate_emits_code_and_reports_guest_size() {
        let mut t = Translator::new();
        let out = t.translate(0x1000, MOV_RAX_RCX).unwrap();
        assert!(!out.from_cache);
        assert_eq!(out.guest_bytes, 3);
        // mov rax, rcx lowers to a load + store; the optimizer must not delete
        // the architectural StoreReg, so the code is non-empty.
        assert!(!out.code.is_empty());
        assert_eq!(out.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
    }

    #[test]
    fn second_translation_is_a_cache_hit_with_identical_code() {
        let mut t = Translator::new();
        let first = t.translate(0x2000, ADD_RAX_IMM8).unwrap();
        assert!(!first.from_cache);
        assert_eq!(t.cached_count(), 1);

        let second = t.translate(0x2000, ADD_RAX_IMM8).unwrap();
        assert!(second.from_cache);
        assert_eq!(second.code, first.code);
        assert_eq!(t.cached_count(), 1);
    }

    #[test]
    fn distinct_addresses_cache_separately() {
        let mut t = Translator::new();
        let _ = t.translate(0x3000, MOV_RAX_RCX).unwrap();
        let _ = t.translate(0x4000, MOV_RAX_RCX).unwrap();
        assert_eq!(t.cached_count(), 2);
    }

    #[test]
    fn running_the_pipeline_is_deterministic() {
        let mut a = Translator::new();
        let mut b = Translator::new();
        assert_eq!(
            a.translate(0x5000, ADD_RAX_IMM8).unwrap().code,
            b.translate(0x5000, ADD_RAX_IMM8).unwrap().code
        );
    }

    #[test]
    fn undecodable_bytes_report_a_decode_error() {
        let mut t = Translator::new();
        // Empty input cannot be decoded.
        assert!(matches!(
            t.translate(0x6000, &[]),
            Err(TranslateError::Decode(_))
        ));
    }

    #[test]
    fn translate_block_stops_at_terminator() {
        // mov rax, rcx ; add rax, 0x10 ; ret
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);
        prog.push(0xC3); // ret

        let mut t = Translator::new();
        let block = t.translate_block(0x7000, &prog, 64).unwrap();
        assert!(block.ended_at_terminator);
        assert_eq!(block.instruction_count, 3);
        assert_eq!(block.guest_bytes, prog.len());
        // Each guest instruction cached independently.
        assert_eq!(t.cached_count(), 3);
        // Concatenated code is the sum of the per-instruction translations.
        assert!(!block.code.is_empty());
        assert_eq!(block.code.len() % 4, 0);
    }

    #[test]
    fn translate_block_honours_instruction_cap() {
        // Three movs, no terminator; cap at 2 instructions.
        let mut prog = Vec::new();
        for _ in 0..3 {
            prog.extend_from_slice(MOV_RAX_RCX);
        }
        let mut t = Translator::new();
        let block = t.translate_block(0x8000, &prog, 2).unwrap();
        assert!(!block.ended_at_terminator);
        assert_eq!(block.instruction_count, 2);
        assert_eq!(block.guest_bytes, MOV_RAX_RCX.len() * 2);
    }

    #[test]
    fn translate_block_runs_to_end_without_terminator() {
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);

        let mut t = Translator::new();
        let block = t.translate_block(0x9000, &prog, 64).unwrap();
        assert!(!block.ended_at_terminator);
        assert_eq!(block.instruction_count, 2);
        assert_eq!(block.guest_bytes, prog.len());
    }

    #[test]
    fn translate_block_first_instruction_error_propagates() {
        let mut t = Translator::new();
        // A lone REX.W prefix has no opcode byte: a truncated first instruction.
        assert!(matches!(
            t.translate_block(0xA000, &[0x48], 64),
            Err(TranslateError::Decode(_))
        ));
    }

    #[test]
    fn translate_block_on_empty_input_is_an_empty_block() {
        let mut t = Translator::new();
        let block = t.translate_block(0xB000, &[], 64).unwrap();
        assert_eq!(block.instruction_count, 0);
        assert_eq!(block.guest_bytes, 0);
        assert!(block.code.is_empty());
        assert!(!block.ended_at_terminator);
    }

    #[test]
    fn stats_track_hits_and_misses() {
        let mut t = Translator::new();
        assert_eq!(t.stats(), TranslatorStats::default());

        t.translate(0xC000, MOV_RAX_RCX).unwrap(); // miss
        t.translate(0xC000, MOV_RAX_RCX).unwrap(); // hit
        t.translate(0xC008, ADD_RAX_IMM8).unwrap(); // miss

        let s = t.stats();
        assert_eq!(s.cache_hits, 1);
        assert_eq!(s.cache_misses, 2);
        assert_eq!(s.total(), 3);

        t.reset_stats();
        assert_eq!(t.stats(), TranslatorStats::default());
    }

    #[test]
    fn set_cache_limits_evicts_to_the_entry_budget() {
        let mut t = Translator::new();
        t.set_cache_limits(1, 0); // at most one entry
        t.translate(0xD000, MOV_RAX_RCX).unwrap();
        t.translate(0xD008, MOV_RAX_RCX).unwrap();
        t.translate(0xD010, MOV_RAX_RCX).unwrap();
        assert_eq!(t.cached_count(), 1, "LRU should hold the budget at 1 entry");
    }

    #[test]
    fn invalidate_evicts_one_address_and_forces_a_re_translation() {
        let mut t = Translator::new();
        t.translate(0xE000, MOV_RAX_RCX).unwrap();
        t.translate(0xE008, ADD_RAX_IMM8).unwrap();
        assert_eq!(t.cached_count(), 2);

        t.invalidate(0xE000);
        assert_eq!(t.cached_count(), 1, "only the rewritten address is dropped");

        // Re-translating the invalidated address is a fresh miss, not a hit.
        let again = t.translate(0xE000, MOV_RAX_RCX).unwrap();
        assert!(!again.from_cache);
    }

    #[test]
    fn clear_cache_drops_every_translation() {
        let mut t = Translator::new();
        t.translate(0xF000, MOV_RAX_RCX).unwrap();
        t.translate(0xF008, ADD_RAX_IMM8).unwrap();
        assert_eq!(t.cached_count(), 2);
        t.clear_cache();
        assert_eq!(t.cached_count(), 0);
    }

    #[test]
    fn fused_block_stops_at_terminator() {
        // mov rax, rcx ; add rax, 0x10 ; ret
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);
        prog.push(0xC3);

        let mut t = Translator::new();
        let block = t.translate_fused_block(0x1_0000, &prog, 64).unwrap();
        assert!(block.ended_at_terminator);
        assert_eq!(block.instruction_count, 3);
        assert_eq!(block.guest_bytes, prog.len());
        assert!(!block.code.is_empty());
        assert_eq!(block.code.len() % 4, 0);
    }

    #[test]
    fn fused_single_instruction_matches_the_simple_path() {
        // With one instruction the renumber base is 0 (identity shift), so the
        // fused lowering must byte-match the plain single-instruction path.
        let mut a = Translator::new();
        let mut b = Translator::new();
        let fused = a.translate_fused_block(0x2_0000, ADD_RAX_IMM8, 64).unwrap();
        let simple = b.translate(0x2_0000, ADD_RAX_IMM8).unwrap();
        assert_eq!(fused.code, simple.code);
        assert_eq!(fused.instruction_count, 1);
    }

    #[test]
    fn fused_block_is_never_larger_than_separate_translation() {
        // mov rax, rcx ; mov rax, rcx — fusing exposes the redundant first
        // store / repeated load to the optimizer, so the single fused region
        // can only be <= the two independently lowered instructions.
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(MOV_RAX_RCX);

        let mut t1 = Translator::new();
        let fused = t1.translate_fused_block(0x3_0000, &prog, 64).unwrap();
        let mut t2 = Translator::new();
        let separate = t2.translate_block(0x3_0000, &prog, 64).unwrap();

        assert_eq!(fused.instruction_count, 2);
        assert!(
            fused.code.len() <= separate.code.len(),
            "fused {} bytes should not exceed separate {} bytes",
            fused.code.len(),
            separate.code.len()
        );
    }

    #[test]
    fn fused_block_is_deterministic() {
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(ADD_RAX_IMM8);
        let mut a = Translator::new();
        let mut b = Translator::new();
        assert_eq!(
            a.translate_fused_block(0x4_0000, &prog, 64).unwrap(),
            b.translate_fused_block(0x4_0000, &prog, 64).unwrap()
        );
    }

    #[test]
    fn truncated_trailing_instruction_does_not_slice_out_of_bounds() {
        // Regression: the proptest minimal failing input — a decode that
        // reports consuming past the buffer. Before the bounds check this
        // panicked with "range end index out of range"; now it must stop
        // cleanly (terminate the block) or return a typed error, never panic.
        let bytes = [106u8, 0, 144, 15, 255, 45];
        let mut t = Translator::new();
        // PUSH/NOP decode fine first, so the block ends gracefully rather than
        // erroring; the contract under test is simply "does not panic".
        let _ = t.translate_block(0, &bytes, 3);
    }

    #[test]
    fn a_lone_truncated_instruction_errors_instead_of_panicking() {
        // A single instruction whose operand runs off the end: with no prior
        // instruction to fall back on, it surfaces as a typed Truncated error.
        // 0x0F 0xFF ... is enough to make the decoder want more than is present.
        let bytes = [0x0Fu8, 0xFF];
        let mut t = Translator::new();
        let result = t.translate_block(0, &bytes, 4);
        // Either a typed error or a clean (possibly empty) block — never a panic.
        if let Err(e) = result {
            assert!(matches!(
                e,
                TranslateError::Truncated { .. } | TranslateError::Decode(_)
            ));
        }
    }
}
