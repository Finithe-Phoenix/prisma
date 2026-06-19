// Prisma translation facade — the integrated Rust pipeline.
//
// Combines the decoder, optimization passes, ARM64 lowerer, and translation
// cache into one entry point: bytes in, optimized ARM64 machine code out,
// memoized by (guest_addr, content hash). Mirrors the C++ `prisma_translator`
// facade. Currently translates a single guest instruction per call; chaining a
// straight-line block (which needs function-global SSA renumbering) is the
// documented follow-up.

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::missing_const_for_fn, clippy::must_use_candidate)]

use prisma_backend::lowerer::{LowerError, Lowerer};
use prisma_cache::cache::{fnv1a_64, LookupResult};
use prisma_cache::{CacheEntry, TranslationCache};
use prisma_decoder::decode::decode_one_at;
use prisma_decoder::DecodeError;
use prisma_ir::{BasicBlock, Function};
use prisma_passes::pipeline::{default_pipeline, PassPipeline};

/// A translated guest instruction: the ARM64 machine code plus how many guest
/// bytes it covered and whether it came from the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub code: Vec<u8>,
    pub guest_bytes: usize,
    pub from_cache: bool,
}

/// Errors from the translation pipeline.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslateError {
    #[error("decode failed: {0:?}")]
    Decode(DecodeError),
    #[error("lowering failed: {0:?}")]
    Lower(LowerError),
}

/// The integrated decode -> optimize -> lower -> cache pipeline.
pub struct Translator {
    cache: TranslationCache,
    pipeline: PassPipeline,
}

impl Default for Translator {
    fn default() -> Self {
        Self {
            cache: TranslationCache::new(),
            pipeline: default_pipeline(),
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
        let insn = &bytes[..decoded.bytes_consumed];

        if let LookupResult::Hit(entry) = self.cache.lookup(guest_addr, insn) {
            return Ok(Translation {
                code: entry.code_bytes.into_vec(),
                guest_bytes: decoded.bytes_consumed,
                from_cache: true,
            });
        }

        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: decoded.stmts,
            }],
        };
        let optimized = self.pipeline.run(func);

        let words = Lowerer::new()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // mov rax, rcx  (REX.W 89 /r)
    const MOV_RAX_RCX: &[u8] = &[0x48, 0x89, 0xC8];
    // add rax, 0x10 (REX.W 83 /0 ib)
    const ADD_RAX_IMM8: &[u8] = &[0x48, 0x83, 0xC0, 0x10];

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
}
