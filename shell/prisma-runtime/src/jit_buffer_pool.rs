//! JIT buffer pool shim + AArch64 branch-patch encoder.

use crate::jit_memory::JitBuffer;

/// Why an AArch64 branch patch could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchError {
    /// The patch site or target is not 4-byte aligned (AArch64 instructions
    /// are word-aligned).
    Unaligned,
    /// The PC-relative displacement does not fit a `B` imm26 field
    /// (±128 MiB, i.e. word delta in `[-2^25, 2^25 - 1]`).
    OutOfRange,
}

/// Inclusive bounds on the signed word displacement of an AArch64 `B`.
const MIN_BRANCH_WORDS: i64 = -(1 << 25);
const MAX_BRANCH_WORDS: i64 = (1 << 25) - 1;

/// Encode an unconditional AArch64 `B` branching from `site` to `target`
/// (both absolute addresses). Mirrors the encoding path of C++
/// `JitSlabPool::patch_aarch64_branch`: `0x1400_0000 | (imm26 & 0x03ff_ffff)`
/// where `imm26` is the signed word displacement `(target - site) / 4`.
///
/// # Errors
/// [`PatchError::Unaligned`] if either address is not 4-byte aligned;
/// [`PatchError::OutOfRange`] if the displacement exceeds the `B` range.
pub const fn encode_aarch64_b(site: u64, target: u64) -> Result<u32, PatchError> {
    if site & 0x3 != 0 || target & 0x3 != 0 {
        return Err(PatchError::Unaligned);
    }
    // Signed byte delta. Wrapping is fine: both are word-aligned and the
    // magnitude is range-checked below.
    let byte_delta = target.wrapping_sub(site) as i64;
    let word_delta = byte_delta / 4;
    if word_delta < MIN_BRANCH_WORDS || word_delta > MAX_BRANCH_WORDS {
        return Err(PatchError::OutOfRange);
    }
    Ok(0x1400_0000 | (word_delta as u32 & 0x03ff_ffff))
}

/// Lightweight metrics for a JIT buffer pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct JitBufferPool {
    /// Number of buffers currently checked out.
    live_buffers: usize,
    /// Total allocated bytes across all checked-out buffers.
    allocated_bytes: usize,
}

impl JitBufferPool {
    /// Creates a new empty pool.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            live_buffers: 0,
            allocated_bytes: 0,
        }
        // NOTE: counters are updated when `acquire`/`release` are called.
    }

    /// Acquires a new buffer from the pool and tracks the allocation.
    #[must_use]
    pub fn acquire(&mut self, size: usize) -> JitBuffer {
        self.live_buffers += 1;
        self.allocated_bytes += size;
        let mut buffer = JitBuffer::with_capacity(size);
        buffer.append(&vec![0u8; size]);
        buffer
    }

    /// Releases a buffer and updates pool accounting.
    pub fn release(&mut self, buffer: JitBuffer) {
        let buffer_len = buffer.len();
        if self.live_buffers > 0 {
            self.live_buffers -= 1;
        }
        if self.allocated_bytes >= buffer_len {
            self.allocated_bytes -= buffer_len;
        } else {
            self.allocated_bytes = 0;
        }
        drop(buffer);
    }

    /// Number of currently checked-out buffers.
    #[must_use]
    pub const fn live_buffers(&self) -> usize {
        self.live_buffers
    }

    /// Total bytes currently accounted by checked-out buffers.
    #[must_use]
    pub const fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }

    /// Whether the pool has no active buffers.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.live_buffers == 0
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_aarch64_b, JitBufferPool, PatchError};

    #[test]
    fn acquire_and_release_tracks_budget() {
        let mut pool = JitBufferPool::new();
        assert!(pool.is_idle());
        let buf = pool.acquire(8);
        assert_eq!(pool.live_buffers(), 1);
        assert_eq!(pool.allocated_bytes(), 8);
        pool.release(buf);
        assert!(pool.is_idle());
        assert_eq!(pool.allocated_bytes(), 0);
    }

    #[test]
    fn branch_forward_in_range() {
        // +16 bytes -> +4 words -> 0x14000000 | 4
        assert_eq!(encode_aarch64_b(0x1000, 0x1010), Ok(0x1400_0004));
    }

    #[test]
    fn branch_backward_in_range() {
        // -16 bytes -> -4 words -> imm26 two's complement 0x3FFFFFC
        assert_eq!(encode_aarch64_b(0x1010, 0x1000), Ok(0x17FF_FFFC));
    }

    #[test]
    fn branch_to_self_is_zero_offset() {
        assert_eq!(encode_aarch64_b(0x2000, 0x2000), Ok(0x1400_0000));
    }

    #[test]
    fn unaligned_site_or_target_rejected() {
        assert_eq!(encode_aarch64_b(0x1001, 0x1010), Err(PatchError::Unaligned));
        assert_eq!(encode_aarch64_b(0x1000, 0x1012), Err(PatchError::Unaligned));
    }

    #[test]
    fn max_positive_displacement_in_range() {
        // word_delta = 2^25 - 1 (max). byte_delta = (2^25 - 1) * 4.
        let site = 0u64;
        let target = ((1u64 << 25) - 1) * 4;
        assert_eq!(encode_aarch64_b(site, target), Ok(0x15FF_FFFF));
    }

    #[test]
    fn one_past_max_is_out_of_range() {
        // word_delta = 2^25 -> out of range.
        let site = 0u64;
        let target = (1u64 << 25) * 4;
        assert_eq!(encode_aarch64_b(site, target), Err(PatchError::OutOfRange));
    }

    #[test]
    fn min_negative_displacement_in_range() {
        // word_delta = -2^25 (min). byte_delta = -2^27.
        let site = 1u64 << 27;
        let target = 0u64;
        assert!(encode_aarch64_b(site, target).is_ok());
        // One word further back is out of range.
        assert_eq!(
            encode_aarch64_b(site + 4, 0),
            Err(PatchError::OutOfRange)
        );
    }
}
