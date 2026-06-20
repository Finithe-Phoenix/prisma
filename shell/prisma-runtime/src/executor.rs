//! Execute translated ARM64 block bodies against a guest CPU state frame.
//!
//! `prisma-backend`'s lowerer emits a block *body* with no calling-convention
//! prologue/epilogue: it reads and writes guest GPRs relative to the
//! `CpuStateFrame*` held in x27 ([`abi::K_STATE_PTR_REG`]), using scratch
//! registers x9..x23. To make a body callable we wrap it with the AAPCS64 block
//! prologue (save callee-saved pairs, `mov x27, x0`) and epilogue (restore +
//! `ret`); the result is an `extern "C" fn(*mut CpuStateFrame)`.
//!
//! Execution is gated to `aarch64` — the bytes are ARM64 machine code, so they
//! can only run on an ARM64 host. On other hosts only [`wrap_block`] (pure byte
//! assembly) is available and testable. This mirrors the C++ core's
//! `constexpr is_arm64` execution gate: behavioural validation runs on the
//! `ffi-link-arm64` CI runner, not on the x86 dev host.

use prisma_backend::abi;
use prisma_backend::Arm64Assembler;

/// Size of the guest GPR file in bytes (16 × u64).
const GPR_BYTES: usize = 16 * 8;
/// Byte offset of `fs_base` inside the frame (matches the lowerer's
/// `FS_BASE_OFFSET`); `gs_base` follows 8 bytes later.
const FS_BASE_OFFSET: usize = 792;

/// Guest CPU state, laid out exactly as the lowerer addresses it.
///
/// Relative to [`abi::K_STATE_PTR_REG`] (x27): `gpr[i]` at byte `i * 8` (x86 GPR
/// order Rax..R15), `fs_base` at 792, `gs_base` at 800. The reserved span stands
/// in for the vector/flag state the C++ `CpuStateFrame` holds in between — it is
/// untouched by GPR-only blocks but keeps segment-base accesses in bounds.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct CpuStateFrame {
    /// Guest general-purpose registers, indexed like [`prisma_ir::Gpr`].
    pub gpr: [u64; 16],
    _reserved: [u8; FS_BASE_OFFSET - GPR_BYTES],
    /// `FS` segment base.
    pub fs_base: u64,
    /// `GS` segment base.
    pub gs_base: u64,
    /// Persistent x86 carry flag (0 or 1) at byte offset 808, matching the
    /// lowerer's `CF_OFFSET`. Used by multi-precision ADC/SBB.
    pub cf: u64,
    _tail: [u8; 56],
}

impl Default for CpuStateFrame {
    fn default() -> Self {
        Self {
            gpr: [0; 16],
            _reserved: [0; FS_BASE_OFFSET - GPR_BYTES],
            fs_base: 0,
            gs_base: 0,
            cf: 0,
            _tail: [0; 56],
        }
    }
}

/// x86-64 GPR indices into [`CpuStateFrame::gpr`] (mirrors `prisma_ir::Gpr`).
pub mod gpr {
    pub const RAX: usize = 0;
    pub const RCX: usize = 1;
    pub const RDX: usize = 2;
    pub const RBX: usize = 3;
    pub const RSP: usize = 4;
    pub const RBP: usize = 5;
    pub const RSI: usize = 6;
    pub const RDI: usize = 7;
    pub const R8: usize = 8;
    pub const R9: usize = 9;
    pub const R10: usize = 10;
    pub const R11: usize = 11;
    pub const R12: usize = 12;
    pub const R13: usize = 13;
    pub const R14: usize = 14;
    pub const R15: usize = 15;
}

fn push_le_words(words: &[u32], out: &mut Vec<u8>) {
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
}

/// Wrap a translated block *body* into a callable function's bytes.
///
/// Brackets the little-endian ARM64 `body` (from `prisma-translator` / the
/// lowerer) with the AAPCS64 block prologue and epilogue, producing the bytes of
/// an `extern "C" fn(*mut CpuStateFrame)`. The body's internal branches are
/// PC-relative and its guest-state accesses are x27-relative, so prepending the
/// prologue (which sets `x27 = x0`) and appending the epilogue does not perturb
/// it.
#[must_use]
pub fn wrap_block(body: &[u8]) -> Vec<u8> {
    let mut prologue = Arm64Assembler::new();
    abi::emit_block_prologue(&mut prologue);
    let mut epilogue = Arm64Assembler::new();
    abi::emit_block_epilogue_and_ret(&mut epilogue);

    let mut out = Vec::with_capacity(body.len() + 64);
    push_le_words(&prologue.finish(), &mut out);
    out.extend_from_slice(body);
    push_le_words(&epilogue.finish(), &mut out);
    out
}

/// Errors from JIT execution of a translated block.
#[derive(Debug)]
pub enum ExecError {
    /// The executable buffer could not be allocated.
    Alloc(std::io::Error),
    /// The code did not fit the allocated buffer.
    Write,
    /// The buffer could not be flipped to read/execute.
    Protect(std::io::Error),
    /// The host is not ARM64, so the translated bytes cannot be executed. The
    /// wrap/install path still ran; this is the no-op return on x86 etc.
    WrongArch,
}

/// Execute a translated block body against `state`.
///
/// Wraps `body` with the block ABI ([`wrap_block`]) and installs it W^X-safely
/// via [`crate::jit_memory::ExecBuffer`]. On an ARM64 host it then calls the
/// block as `extern "C" fn(*mut CpuStateFrame)` and the frame's GPRs are updated
/// in place; on any other host the wrap/install path runs but execution is
/// skipped with [`ExecError::WrongArch`] (the bytes are ARM64 machine code).
/// Real behavioural validation therefore happens on the `ffi-link-arm64` CI
/// runner, mirroring the C++ core's `constexpr is_arm64` gate.
///
/// # Errors
/// [`ExecError`] if the executable buffer cannot be allocated/written/protected,
/// or [`ExecError::WrongArch`] on a non-ARM64 host.
pub fn execute_block(body: &[u8], state: &mut CpuStateFrame) -> Result<(), ExecError> {
    use crate::jit_memory::ExecBuffer;

    let code = wrap_block(body);
    let mut buf = ExecBuffer::alloc(code.len()).map_err(ExecError::Alloc)?;
    if !buf.write(&code) {
        return Err(ExecError::Write);
    }
    buf.make_executable().map_err(ExecError::Protect)?;

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `code` is a valid AAPCS64 `extern "C" fn(*mut CpuStateFrame)` —
        // the prologue moves the x0 argument into x27, the body addresses guest
        // state through x27 and clobbers only caller-saved / prologue-saved
        // registers, and the epilogue restores callee-saved registers and
        // returns. `buf` is read+execute and is kept alive across the call.
        let entry: extern "C" fn(*mut CpuStateFrame) =
            unsafe { core::mem::transmute(buf.as_ptr()) };
        entry(state);
        Ok(())
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = state;
        Err(ExecError::WrongArch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_state_frame_layout_matches_the_lowerer() {
        // The lowerer addresses gpr[i] at i*8 and fs/gs at 792/800.
        let frame = CpuStateFrame::default();
        let base = std::ptr::addr_of!(frame).cast::<u8>();
        // SAFETY: all offsets are within the frame; pointers are only compared.
        unsafe {
            assert_eq!(
                std::ptr::addr_of!(frame.gpr).cast::<u8>().offset_from(base),
                0
            );
            assert_eq!(
                std::ptr::addr_of!(frame.fs_base)
                    .cast::<u8>()
                    .offset_from(base),
                792
            );
            assert_eq!(
                std::ptr::addr_of!(frame.gs_base)
                    .cast::<u8>()
                    .offset_from(base),
                800
            );
            assert_eq!(
                std::ptr::addr_of!(frame.cf).cast::<u8>().offset_from(base),
                808
            );
        }
    }

    #[test]
    fn wrap_block_brackets_body_with_prologue_and_epilogue() {
        // Body = a single NOP-equivalent word so we can see the brackets.
        let body_word: u32 = 0xD503_201F; // nop
        let body = body_word.to_le_bytes().to_vec();
        let wrapped = wrap_block(&body);

        // 7 prologue words + 1 body word + 7 epilogue words = 15 words.
        assert_eq!(wrapped.len(), 15 * 4);
        // Prologue starts with the first callee-saved STP (x19,x20 pre-index).
        assert_eq!(&wrapped[0..4], &0xA9BF_53F3u32.to_le_bytes());
        // The body word sits right after the 7-word prologue.
        assert_eq!(&wrapped[28..32], &body_word.to_le_bytes());
        // The block ends with `ret`.
        assert_eq!(&wrapped[wrapped.len() - 4..], &0xD65F_03C0u32.to_le_bytes());
    }

    // On the x86 dev host the wrap + W^X install path runs but execution is
    // skipped — exercises ExecBuffer alloc/write/make_executable end to end
    // without running ARM64 bytes. The real execution is covered by the
    // aarch64-gated e2e test (tests/exec_e2e.rs).
    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn execute_block_installs_then_reports_wrong_arch_off_target() {
        let body = 0xD503_201Fu32.to_le_bytes().to_vec(); // nop
        let mut state = CpuStateFrame::default();
        assert!(matches!(
            execute_block(&body, &mut state),
            Err(ExecError::WrongArch)
        ));
    }
}
