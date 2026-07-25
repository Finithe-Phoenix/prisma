// Public translator facade with fused-block input bounds enforcement.
//
// The implementation remains in `lib.rs`; this wrapper keeps the public API
// stable while ensuring fused translation never reports bytes outside the
// caller-provided guest buffer.

#[path = "lib.rs"]
mod implementation;

pub use implementation::interp;
pub use implementation::{
    decode_block_successors, BlockTranslation, OptimizedBlock, TranslateError, Translation,
    TranslatorStats,
};

use prisma_decoder::decode::decode_one_at;

/// Integrated decode -> optimize -> lower -> cache pipeline.
pub struct Translator {
    inner: implementation::Translator,
}

impl Default for Translator {
    fn default() -> Self {
        Self {
            inner: implementation::Translator::default(),
        }
    }
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn translate(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
    ) -> Result<Translation, TranslateError> {
        self.inner.translate(guest_addr, bytes)
    }

    pub fn translate_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        self.inner
            .translate_block(guest_addr, bytes, max_insns)
    }

    pub fn translate_fused_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        let end = bounded_fused_prefix(guest_addr, bytes, max_insns)?;
        self.inner
            .translate_fused_block(guest_addr, &bytes[..end], max_insns)
    }

    pub fn optimize_fused_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<OptimizedBlock, TranslateError> {
        let end = bounded_fused_prefix(guest_addr, bytes, max_insns)?;
        self.inner
            .optimize_fused_block(guest_addr, &bytes[..end], max_insns)
    }

    pub fn cached_count(&self) -> usize {
        self.inner.cached_count()
    }

    pub const fn stats(&self) -> TranslatorStats {
        self.inner.stats()
    }

    pub fn reset_stats(&mut self) {
        self.inner.reset_stats();
    }

    pub fn set_cache_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.inner.set_cache_limits(max_entries, max_bytes);
    }

    pub fn invalidate(&mut self, guest_addr: u64) {
        self.inner.invalidate(guest_addr);
    }

    pub fn clear_cache(&mut self) {
        self.inner.clear_cache();
    }
}

fn bounded_fused_prefix(
    guest_addr: u64,
    bytes: &[u8],
    max_insns: usize,
) -> Result<usize, TranslateError> {
    let mut offset = 0usize;
    let mut pc = guest_addr;
    let mut instruction_count = 0usize;

    while offset < bytes.len() && instruction_count < max_insns {
        let decoded = match decode_one_at(bytes, offset, pc) {
            Ok(decoded) => decoded,
            Err(error) => {
                if instruction_count == 0 {
                    return Err(TranslateError::Decode(error));
                }
                break;
            }
        };

        let Some(end) = offset
            .checked_add(decoded.bytes_consumed)
            .filter(|&end| end <= bytes.len())
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

        offset = end;
        pc = pc.wrapping_add(decoded.bytes_consumed as u64);
        instruction_count += 1;
    }

    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fused_block_stays_within_the_caller_buffer() {
        // Proptest regression ccba251a: the final decoded instruction reports
        // consuming beyond this eight-byte buffer.
        let bytes = [132u8, 6, 24, 101, 0, 15, 120, 53];
        let mut translator = Translator::new();
        let block = translator
            .optimize_fused_block(0, &bytes, 3)
            .expect("optimize the bounded prefix");

        assert!(block.guest_bytes <= bytes.len());
        assert!(block.instruction_count <= 3);
    }
}
