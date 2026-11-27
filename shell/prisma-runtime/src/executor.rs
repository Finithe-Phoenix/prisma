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
//! can only run on an ARM64 host. Other hosts can wrap and install blocks
//! and test the executable mapping lifecycle. This mirrors the C++ core's
//! `constexpr is_arm64` execution gate: behavioural validation runs on the
//! `ffi-link-arm64` CI runner, not on the x86 dev host.

use crate::jit_memory::ExecBuffer;
use prisma_backend::abi;
use prisma_backend::Arm64Assembler;

/// Size of the guest GPR file in bytes (16 × u64).
const GPR_BYTES: usize = 16 * 8;
/// Byte offset of `fs_base` inside the frame (matches the lowerer's
/// `FS_BASE_OFFSET`); `gs_base` follows 8 bytes later.
const FS_BASE_OFFSET: usize = 792;

/// [`CpuStateFrame::exit_reason`] value: the block ran to a normal terminator
/// (a branch/return). The run loop continues from the next guest PC.
pub const EXIT_NORMAL: u64 = 0;
/// [`CpuStateFrame::exit_reason`] value: the block ended on a guest `SYSCALL`.
///
/// The run loop reads the syscall number/args from the GPRs, services it, writes
/// the result to `rax`, advances the PC past the 2-byte `SYSCALL`, and resumes.
pub const EXIT_SYSCALL: u64 = 1;
/// [`CpuStateFrame::exit_reason`] value: the block ended on a direct relative
/// branch.
///
/// The taken `JumpRel`/`CondJumpRel` target is computed by the block and stored
/// in [`CpuStateFrame::next_pc`]; the run loop resumes there. This is how blocks
/// chain across branches without re-entering the host translator's sibling-block
/// path.
pub const EXIT_BRANCH: u64 = 2;

/// Guest CPU state, laid out exactly as the lowerer addresses it.
///
/// Relative to [`abi::K_STATE_PTR_REG`] (x27): `gpr[i]` at byte `i * 8` (x86 GPR
/// order Rax..R15), `fs_base` at 792, `gs_base` at 800. The reserved span stands
/// in for the vector state the C++ `CpuStateFrame` holds in between — it is
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
    /// Persistent x86 RFLAGS subset at byte offset 816, matching the lowerer's
    /// `RFLAGS_OFFSET`. Bit 1 is forced by PUSHFQ; currently CF is synchronized
    /// with [`CpuStateFrame::cf`] and the field is ready for ZF/SF/OF expansion.
    pub rflags: u64,
    /// Why the block returned to the host ([`EXIT_NORMAL`] / [`EXIT_SYSCALL`]),
    /// at byte offset 824 (matching the lowerer's `EXIT_REASON_OFFSET`). A guest
    /// `SYSCALL` block stores [`EXIT_SYSCALL`] here before returning; a normal
    /// block leaves it untouched, so the run loop clears it to [`EXIT_NORMAL`]
    /// before each block.
    pub exit_reason: u64,
    /// The guest PC to resume at when `exit_reason` is [`EXIT_BRANCH`], at byte
    /// offset 832 (matching the lowerer's `NEXT_PC_OFFSET`). A branch block
    /// computes its taken target (via `CSEL` for a conditional branch) and stores
    /// it here before returning to the run loop.
    pub next_pc: u64,
    /// Host base address the guest address space is mapped at, at byte offset 840
    /// (matching the lowerer's `MEM_BASE_OFFSET`). Every guest memory access is
    /// lowered to `host = mem_base + guest_va`, so a contiguous host arena that is
    /// not identity-mapped to the guest VAs is still reachable from the JIT (RFC
    /// 0020). The default 0 reproduces the legacy `host == guest` behaviour, which
    /// is why GPR-only blocks and existing tests are unaffected.
    pub mem_base: u64,
    _tail: [u8; 32],
}

impl Default for CpuStateFrame {
    fn default() -> Self {
        Self {
            gpr: [0; 16],
            _reserved: [0; FS_BASE_OFFSET - GPR_BYTES],
            fs_base: 0,
            gs_base: 0,
            cf: 0,
            rflags: 2,
            exit_reason: 0,
            next_pc: 0,
            mem_base: 0,
            _tail: [0; 32],
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

/// A translated ARM64 block with its own read/execute mapping.
/// Dropping the block releases the mapping through [`ExecBuffer`].
#[derive(Debug)]
pub struct InstalledBlock {
    buffer: ExecBuffer,
}

impl InstalledBlock {
    /// Wrap the backend's block body in the guest-call ABI and install it once.
    ///
    /// # Errors
    /// Returns an [`ExecError`] if allocation, writing or protection fails.
    pub fn install(body: &[u8]) -> Result<Self, ExecError> {
        let code = wrap_block(body);
        let mut buffer = ExecBuffer::alloc(code.len()).map_err(ExecError::Alloc)?;
        if !buffer.write(&code) {
            return Err(ExecError::Write);
        }
        buffer.make_executable().map_err(ExecError::Protect)?;
        Ok(Self { buffer })
    }

    /// Execute this block against the supplied guest state.
    ///
    /// # Safety
    /// On ARM64 the installed body must be trusted code conforming to the block
    /// ABI in [`wrap_block`]. It must preserve the host calling convention and
    /// return through the matching epilogue. Every memory access it can perform
    /// with this frame must be valid for the access size and permissions,
    /// including accesses based on guest register values and `mem_base`. The
    /// caller must keep those mappings alive and synchronize conflicting accesses.
    ///
    /// # Errors
    /// Returns [`ExecError::WrongArch`] on hosts other than ARM64.
    pub unsafe fn execute(&self, state: &mut CpuStateFrame) -> Result<(), ExecError> {
        let entry = self.buffer.as_ptr();
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: the installed backend body uses the block ABI. Its prologue
            // sets x27 to the frame and its epilogue restores the callee-saved
            // registers. The borrow of self keeps the RX mapping alive for the call.
            let call: extern "C" fn(*mut CpuStateFrame) = unsafe { core::mem::transmute(entry) };
            call(state);
            Ok(())
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = (entry, state);
            Err(ExecError::WrongArch)
        }
    }
}

/// Install a translated block body, execute it against `state` and release it.
///
/// This compatibility entry point expects trusted backend output and a frame
/// whose referenced guest memory satisfies the [`InstalledBlock::execute`]
/// contract. It performs no validation of machine instructions or guest addresses.
///
/// # Errors
/// Returns installation errors or [`ExecError::WrongArch`] on a non-ARM64 host.
pub fn execute_block(body: &[u8], state: &mut CpuStateFrame) -> Result<(), ExecError> {
    let installed = InstalledBlock::install(body)?;
    // SAFETY: this legacy entry point relies on its callers to provide a trusted
    // block and keep all memory it uses accessible throughout execution.
    unsafe { installed.execute(state) }
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
            assert_eq!(
                std::ptr::addr_of!(frame.rflags)
                    .cast::<u8>()
                    .offset_from(base),
                816
            );
            assert_eq!(
                std::ptr::addr_of!(frame.exit_reason)
                    .cast::<u8>()
                    .offset_from(base),
                824
            );
            assert_eq!(
                std::ptr::addr_of!(frame.next_pc)
                    .cast::<u8>()
                    .offset_from(base),
                832
            );
            assert_eq!(
                std::ptr::addr_of!(frame.mem_base)
                    .cast::<u8>()
                    .offset_from(base),
                840
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

    #[test]
    fn installed_block_can_execute_against_separate_frames() {
        // mov rax, rcx
        let translated = prisma_translator::Translator::new()
            .translate_fused_block(0x1000, &[0x48, 0x89, 0xc8], 1)
            .expect("translate mov");
        let installed = InstalledBlock::install(&translated.code).expect("install once");
        for input in [17, 91] {
            let mut state = CpuStateFrame::default();
            state.gpr[gpr::RCX] = input;
            // SAFETY: the backend lowered a register-only move. The block
            // touches only GPR slots in this exclusively borrowed live frame.
            let result = unsafe { installed.execute(&mut state) };
            #[cfg(target_arch = "aarch64")]
            {
                result.expect("run the installed block");
                assert_eq!(state.gpr[gpr::RAX], input);
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                assert!(matches!(result, Err(ExecError::WrongArch)));
                assert_eq!(state.gpr[gpr::RAX], 0);
            }
        }
    }

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
