//! Prisma execution session — the composition root (RFC 0017).
//!
//! The one place `orchestrator` (load), `prisma-translator` (translate), and
//! `prisma-runtime` (dispatch/execute) meet. A [`Session`] loads a PE into guest
//! memory, then drives the dispatcher's `run_with_callbacks` seam with two
//! adapters — a fetch over the mapped image and a translate over the
//! `Translator` — so a guest block goes from bytes to an installed ARM64
//! translation. The dispatcher owns the guest-PC stepping (which is why
//! `CpuStateFrame` has no `RIP`); this crate is just the wiring + ownership.

pub mod fs_syscalls;
pub mod guest_io;
pub mod info_syscalls;
pub mod io_syscalls;
pub mod mem_syscalls;
pub mod process_syscalls;
pub mod resource_syscalls;
pub mod sched_syscalls;
pub mod sig_syscalls;
pub mod syscall_dispatch;
pub mod time_syscalls;
pub mod tty_syscalls;

use std::collections::HashSet;

use prisma_cache::TranslationCache;
use prisma_orchestrator::address_space::AddressSpaceError;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_orchestrator::guest_layout::{
    backed_layout_sections, layout_sections, populate_backed, BackedGuestLayout, GuestLayout,
};
use prisma_orchestrator::guest_stack::DEFAULT_STACK_SIZE;
use prisma_orchestrator::init_stack::{build_initial_stack, ProcessStackParams, StackBuildError};
use prisma_orchestrator::load_pe::{load_pe_with_image, LoadError};
use prisma_orchestrator::module_table::ModuleTable;
use prisma_orchestrator::pe_loader::{MappedImage, PeImage};
use prisma_runtime::dispatcher::DispatchRunOutcome;
use prisma_runtime::executor::{
    execute_block, gpr, CpuStateFrame, ExecError, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL,
};
use prisma_runtime::{Dispatcher, SmcGuard};

use crate::syscall_dispatch::{dispatch, SyscallContext};
use prisma_translator::{decode_block_successors, BlockTranslation, Translator};

/// How many guest bytes a single fetch hands the translator. The translator
/// decodes until a block terminator (or its own instruction cap) within this
/// window; one page is far more than a straight-line block.
const FETCH_WINDOW: usize = 4096;

/// Per-block instruction cap handed to the translator (a straight-line block is
/// far shorter; this bounds a pathological run-on).
const MAX_INSNS: usize = 1024;

/// A loaded, runnable guest program: the mapped image plus the translate +
/// dispatch machinery, owned together so teardown is deterministic.
///
/// Resource discipline (RFC 0017): the translated blocks' W^X JIT buffers are
/// owned by the cache and unmapped on eviction / when the session (hence the
/// cache and translator) drops; the guest [`MappedImage`] frees its bytes on
/// drop. Dropping a `Session` releases all of it — nothing survives a restart.
pub struct Session {
    pe_image: PeImage,
    image: MappedImage,
    translator: Translator,
    cache: TranslationCache,
    guard: SmcGuard,
    dispatcher: Dispatcher,
}

/// Why [`Session::prepare`] could not build a runnable initial state.
#[derive(Debug)]
pub enum PrepareError {
    /// The guest address space could not be laid out (section overlap/overflow).
    Layout(AddressSpaceError),
    /// The initial process stack did not fit (argument image larger than the
    /// stack region).
    Stack(StackBuildError),
    /// The contiguous host arena backing the guest space could not be allocated
    /// (arena mode only, [`Session::prepare_arena`]).
    Arena(std::io::Error),
}

impl From<AddressSpaceError> for PrepareError {
    fn from(e: AddressSpaceError) -> Self {
        Self::Layout(e)
    }
}

impl From<std::io::Error> for PrepareError {
    fn from(e: std::io::Error) -> Self {
        Self::Arena(e)
    }
}

impl From<StackBuildError> for PrepareError {
    fn from(e: StackBuildError) -> Self {
        Self::Stack(e)
    }
}

/// A guest ready to run: the byte-backed memory, a CPU frame with `RSP` seeded
/// at the initial stack, and a fresh syscall context — the three mutable inputs
/// [`Session::run`] borrows. Owning them together keeps teardown deterministic
/// (the `BackedAddressSpace` frees its bytes on drop).
#[derive(Debug)]
pub struct PreparedRun {
    pub mem: BackedAddressSpace,
    pub state: CpuStateFrame,
    pub ctx: SyscallContext,
}

/// Why [`Session::run`] stopped.
#[derive(Debug)]
pub enum RunOutcome {
    /// The guest called `exit`/`exit_group`; the value is its exit status.
    Exited(i32),
    /// The step budget (total blocks executed) was exhausted before the guest
    /// exited — e.g. a non-terminating loop.
    StepLimit,
    /// A block ended on a terminator that can't be followed yet (a `ret` or an
    /// indirect jump — a dynamic target needs the guest stack / register, which
    /// the run loop doesn't read yet) at `pc`.
    NonSyscallExit { pc: u64 },
    /// No block could be translated at `pc` (unmapped / undecodable).
    UnmappedPc(u64),
    /// Execution is unavailable on this host (e.g. [`ExecError::WrongArch`] on a
    /// non-ARM64 dev host); the translate path still ran.
    ExecUnavailable(ExecError),
}

impl Session {
    /// Load `file` against `modules` and build a session ready to run from the
    /// image's entry point.
    pub fn load(file: &[u8], modules: &ModuleTable) -> Result<Self, LoadError> {
        let (pe_image, image) = load_pe_with_image(file, modules)?;
        Ok(Self {
            pe_image,
            image,
            translator: Translator::new(),
            cache: TranslationCache::new(),
            guard: SmcGuard::new(),
            dispatcher: Dispatcher::new(),
        })
    }

    /// Build the guest W^X address space — each PE section mapped at its own
    /// protection (`.text` RX, `.data` RW, ...) plus an initial-thread stack
    /// based at `stack_base`. This is the memory model execution runs against:
    /// code pages not writable, data pages not executable.
    pub fn layout(&self, stack_base: u64) -> Result<GuestLayout, AddressSpaceError> {
        layout_sections(&self.pe_image, &self.image, stack_base)
    }

    /// Build the *byte-backed* guest address space — each section's content
    /// copied into its own region (W^X) plus a zeroed stack — the memory a run
    /// loop executes against and that `brk`/`mmap` can grow (RFC 0019). The
    /// byte-backed counterpart of [`Session::layout`].
    ///
    /// # Errors
    /// [`AddressSpaceError`] if a section base overflows or any mapping overlaps.
    pub fn backed_layout(&self, stack_base: u64) -> Result<BackedGuestLayout, AddressSpaceError> {
        backed_layout_sections(&self.pe_image, &self.image, stack_base)
    }

    /// Build a runnable initial state: lay out byte-backed guest memory at
    /// `stack_base`, populate the initial process stack with `argv`/`envp` (plus
    /// a minimal auxv), and seed `RSP` at its top — everything [`Session::run`]
    /// needs, composed in one call.
    ///
    /// The auxv carries only `AT_PAGESZ` here; thread-local / loader-specific
    /// entries (`AT_RANDOM`, `AT_PHDR`, ...) come with their own follow-on work
    /// (RFC 0019 scopes TLS out), so this targets a freestanding program that
    /// reads its own `argc`/`argv`/`envp` rather than a full dynamically-linked
    /// glibc start-up.
    ///
    /// # Errors
    /// [`PrepareError::Layout`] if the address space cannot be laid out;
    /// [`PrepareError::Stack`] if the argument image does not fit the stack.
    pub fn prepare(
        &self,
        stack_base: u64,
        argv: &[&[u8]],
        envp: &[&[u8]],
    ) -> Result<PreparedRun, PrepareError> {
        let layout = self.backed_layout(stack_base)?;
        let mut mem = layout.space;
        const AT_PAGESZ: u64 = 6;
        let auxv = [(AT_PAGESZ, 4096)];
        let params = ProcessStackParams {
            argv,
            envp,
            auxv: &auxv,
        };
        let rsp = build_initial_stack(&mut mem, layout.stack_top, &params)?;
        let mut state = CpuStateFrame::default();
        state.gpr[gpr::RSP] = rsp;
        Ok(PreparedRun {
            mem,
            state,
            ctx: SyscallContext::new(),
        })
    }

    /// Like [`prepare`](Self::prepare), but backs the guest with a single
    /// contiguous host arena (RFC 0020) and seeds `state.mem_base`, so the JIT's
    /// memory accesses and the syscall layer share one backing — the Stage 2B
    /// memory model. The arena window spans `[image_base, stack_top)`; pass a
    /// `stack_base` close to the image so the window stays compact (the whole
    /// window is one host mapping — a far-apart stack would reserve a huge span).
    ///
    /// On a non-ARM64 host the arena is still allocated and the program laid out
    /// (so the layout is testable everywhere); only [`run`](Self::run)'s
    /// execution is ARM64-gated.
    ///
    /// # Errors
    /// [`PrepareError::Arena`] if the host arena cannot be allocated,
    /// [`PrepareError::Layout`] if the address space cannot be laid out (e.g. a
    /// region falls outside the window), or [`PrepareError::Stack`] if the
    /// argument image does not fit the stack.
    pub fn prepare_arena(
        &self,
        stack_base: u64,
        argv: &[&[u8]],
        envp: &[&[u8]],
    ) -> Result<PreparedRun, PrepareError> {
        let image_base = self.image.base;
        let stack_top = stack_base.saturating_add(DEFAULT_STACK_SIZE);
        // One page below the image through the stack top, page-rounded — the whole
        // guest lives in this single contiguous host window.
        let window_base = image_base & !0xFFF;
        let window_size = usize::try_from(stack_top.saturating_sub(window_base)).map_err(|_| {
            PrepareError::Layout(AddressSpaceError::Overflow(window_base, stack_top))
        })?;
        let mut mem = BackedAddressSpace::with_arena(window_base, window_size)?;
        populate_backed(&mut mem, &self.pe_image, &self.image, stack_base)?;
        const AT_PAGESZ: u64 = 6;
        let auxv = [(AT_PAGESZ, 4096)];
        let params = ProcessStackParams {
            argv,
            envp,
            auxv: &auxv,
        };
        let rsp = build_initial_stack(&mut mem, stack_top, &params)?;
        let mut state = CpuStateFrame::default();
        state.gpr[gpr::RSP] = rsp;
        // The single offset the JIT rebases every guest memory access through.
        state.mem_base = mem.mem_base().expect("arena-mode space exposes a mem_base");
        Ok(PreparedRun {
            mem,
            state,
            ctx: SyscallContext::new(),
        })
    }

    /// Run the guest from its entry point, chaining blocks across branches and
    /// servicing each `SYSCALL`, until it calls `exit`/`exit_group` (or a
    /// limit/fault). Bounded by `max_steps` total block executions.
    ///
    /// The OS-ABI execution loop (RFC 0019): translate the block at the current
    /// PC, execute it (ARM64), then resume by exit reason:
    /// - [`EXIT_SYSCALL`] — read the number/args from the GPRs, dispatch it, write
    ///   the result to `rax`, and resume past the 2-byte `SYSCALL`; stop when the
    ///   guest sets [`SyscallContext::exit_status`].
    /// - [`EXIT_BRANCH`] — a relative branch recorded its taken target in
    ///   `next_pc`; resume there (this is how loops/`if`s run).
    /// - [`EXIT_NORMAL`] — a block cut at the fetch budget falls through to the
    ///   next sequential PC; a block ending in a `ret`/indirect jump (a dynamic
    ///   target the loop can't follow yet) halts with [`RunOutcome::NonSyscallExit`].
    ///
    /// Execution is ARM64-gated: on a non-ARM64 host the first block returns
    /// [`ExecError::WrongArch`] and this reports [`RunOutcome::ExecUnavailable`].
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn run(
        &mut self,
        ctx: &mut SyscallContext,
        mem: &mut BackedAddressSpace,
        state: &mut CpuStateFrame,
        max_steps: usize,
    ) -> RunOutcome {
        let mut pc = self.entry_pc();
        for _ in 0..max_steps {
            let Some(block) = self.translate_at(pc) else {
                return RunOutcome::UnmappedPc(pc);
            };
            state.exit_reason = EXIT_NORMAL;
            if let Err(e) = execute_block(&block.code, state) {
                return RunOutcome::ExecUnavailable(e);
            }
            match state.exit_reason {
                EXIT_SYSCALL => {
                    let number = state.gpr[gpr::RAX];
                    let args = [
                        state.gpr[gpr::RDI],
                        state.gpr[gpr::RSI],
                        state.gpr[gpr::RDX],
                        state.gpr[gpr::R10],
                        state.gpr[gpr::R8],
                        state.gpr[gpr::R9],
                    ];
                    state.gpr[gpr::RAX] = dispatch(ctx, mem, number, args) as u64;
                    if let Some(code) = ctx.exit_status {
                        return RunOutcome::Exited(code);
                    }
                    pc = pc.wrapping_add(block.guest_bytes as u64); // past SYSCALL
                }
                EXIT_BRANCH => pc = state.next_pc, // resume at the taken target
                _ if !block.ended_at_terminator => {
                    // A block cut at the fetch budget: continue at the next PC.
                    pc = pc.wrapping_add(block.guest_bytes as u64);
                }
                _ => return RunOutcome::NonSyscallExit { pc }, // ret / indirect
            }
        }
        RunOutcome::StepLimit
    }

    /// Guest virtual address execution begins at.
    #[must_use]
    pub fn entry_pc(&self) -> u64 {
        self.image.entry_pc
    }

    /// The guest bytes at `guest_pc` (up to a page), or `None` if `guest_pc` is
    /// not inside the mapped image. This is the dispatcher's fetch source.
    #[must_use]
    pub fn fetch(&self, guest_pc: u64) -> Option<Vec<u8>> {
        fetch_window(&self.image, guest_pc)
    }

    /// Drive one dispatcher step from the entry point: fetch the entry block,
    /// probe the cache, translate on a miss, install the ARM64 translation. The
    /// install/execute path is ARM64-gated inside the dispatcher (mirrors the
    /// executor's `is_arm64` gate); on other hosts the fetch + translate run and
    /// the install is the no-op path, so this stays safe to call anywhere.
    pub fn step_entry(&mut self) -> DispatchRunOutcome {
        let entry = self.image.entry_pc;
        // Disjoint field borrows so the fetch (over the image) and translate
        // (over the translator) closures can coexist with the dispatcher call.
        let image = &self.image;
        let translator = &mut self.translator;
        let cache = &mut self.cache;
        let guard = &mut self.guard;
        let dispatcher = &mut self.dispatcher;

        let fetch = |pc: u64| fetch_window(image, pc);
        let translate = |pc: u64, bytes: &[u8]| {
            translator
                .translate_block(pc, bytes, MAX_INSNS)
                .ok()
                .map(|b| b.code)
        };

        // One dispatcher step: fetch the entry block, probe the cache, translate
        // on a miss, install the translation (M1 is a single step).
        dispatcher.run_with_callbacks(cache, guard, entry, 1, fetch, translate)
    }

    /// Translate the straight-line block at `guest_pc` to ARM64, or `None` if
    /// the PC is unmapped or its bytes don't translate. The per-block primitive
    /// the run loop drives; unlike [`Session::step_entry`] it does not touch the
    /// dispatcher cache, so it is a pure query of "what does this PC translate
    /// to" usable on any host.
    pub fn translate_at(&mut self, guest_pc: u64) -> Option<BlockTranslation> {
        let bytes = fetch_window(&self.image, guest_pc)?;
        // Use the FUSED path: it renumbers SSA refs across the block and routes
        // relative branches through the frame's next_pc (with_branch_exits) so the
        // run loop can chain. The per-instruction translate_block lowers each
        // instruction independently and mis-handles a multi-instruction block that
        // ends in a conditional branch (it produced wrong loop results on ARM64).
        self.translator
            .translate_fused_block(guest_pc, &bytes, MAX_INSNS)
            .ok()
    }
}

impl Session {
    /// Translate the statically-reachable control-flow graph from the entry
    /// point, returning the guest PCs of the blocks translated (in visit order).
    ///
    /// A worklist walk (RFC 0017 M2): translate the block at a PC, enqueue its
    /// static successors (relative branch/call targets + fall-through), and
    /// repeat, skipping already-seen PCs and stopping at `max_blocks`. Dynamic
    /// transfers (indirect jump/call, return) contribute no static successors,
    /// so the walk follows only what is known ahead of execution. Translating is
    /// pure (no ARM64 needed), so this runs on any host.
    pub fn translate_reachable(&mut self, max_blocks: usize) -> Vec<u64> {
        let mut seen: HashSet<u64> = HashSet::new();
        let mut queue: Vec<u64> = vec![self.image.entry_pc];
        let mut translated: Vec<u64> = Vec::new();

        while let Some(pc) = queue.pop() {
            if translated.len() >= max_blocks {
                break;
            }
            if !seen.insert(pc) {
                continue;
            }
            let Some(bytes) = fetch_window(&self.image, pc) else {
                continue; // PC outside the mapped image
            };
            let Ok(block) = self.translator.translate_block(pc, &bytes, MAX_INSNS) else {
                continue; // undecodable / unlowerable at this PC
            };
            translated.push(pc);
            for &succ in &block.successors {
                if !seen.contains(&succ) {
                    queue.push(succ);
                }
            }
        }

        translated
    }

    /// Discover the statically-reachable block PCs from the entry by DECODING
    /// only — independent of whether the backend can lower each block.
    ///
    /// Unlike [`Session::translate_reachable`], this does not require a block to
    /// translate (lower) to follow its successors, so it walks the whole CFG
    /// even across terminators the lowerer cannot yet emit (relative branches).
    /// It is the CFG-discovery counterpart: returns the reachable block PCs in
    /// visit order, bounded by `max_blocks`, deduped against a seen-set so a
    /// self- or back-edge terminates.
    pub fn reachable_blocks(&self, max_blocks: usize) -> Vec<u64> {
        let mut seen: HashSet<u64> = HashSet::new();
        let mut queue: Vec<u64> = vec![self.image.entry_pc];
        let mut reached: Vec<u64> = Vec::new();

        while let Some(pc) = queue.pop() {
            if reached.len() >= max_blocks {
                break;
            }
            if !seen.insert(pc) {
                continue;
            }
            let Some(bytes) = fetch_window(&self.image, pc) else {
                continue; // PC outside the mapped image
            };
            reached.push(pc);
            for succ in decode_block_successors(pc, &bytes, MAX_INSNS) {
                if !seen.contains(&succ) {
                    queue.push(succ);
                }
            }
        }

        reached
    }
}

/// Read up to [`FETCH_WINDOW`] bytes of the mapped image starting at `guest_pc`.
/// `None` if the address is below the image base or past its end.
fn fetch_window(image: &MappedImage, guest_pc: u64) -> Option<Vec<u8>> {
    let offset = usize::try_from(guest_pc.checked_sub(image.base)?).ok()?;
    // `offset == len` (one past the last byte, i.e. exactly the image end) is
    // outside the image — reject it rather than return an empty window.
    if offset >= image.bytes.len() {
        return None;
    }
    let end = offset.checked_add(FETCH_WINDOW)?.min(image.bytes.len());
    image.bytes.get(offset..end).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid PE32+ with one `.text` section, image base 0x1_4000_0000,
    /// entry RVA 0x1000, size 0x10000. (Mirrors load_pe's own test fixture.)
    fn minimal_pe() -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
        buf[64..68].copy_from_slice(b"PE\0\0");
        let coff = 68;
        buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = coff + 20;
        buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
        buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes());
        let sec = opt + 240;
        buf[sec..sec + 5].copy_from_slice(b".text");
        buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        buf
    }

    #[test]
    fn loads_and_reports_entry_pc() {
        let s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        assert_eq!(s.entry_pc(), 0x1_4000_1000);
    }

    #[test]
    fn fetch_reads_inside_the_image_and_rejects_outside() {
        let s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        // Entry is inside the image: a window comes back, capped at the page.
        let at_entry = s.fetch(s.entry_pc()).expect("entry is mapped");
        assert_eq!(at_entry.len(), FETCH_WINDOW);
        // The base resolves; one byte below it does not.
        assert!(s.fetch(0x1_4000_0000).is_some());
        assert!(s.fetch(0x1_3FFF_FFFF).is_none());
        // Past the image end is rejected.
        assert!(s.fetch(0x1_4001_0000).is_none());
    }

    #[test]
    fn fetch_near_image_end_is_truncated_not_overrun() {
        let s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        // 8 bytes before the end -> exactly 8 bytes, not a page.
        let near_end = s.fetch(0x1_4000_0000 + 0x10000 - 8).expect("mapped");
        assert_eq!(near_end.len(), 8);
    }

    #[test]
    fn translate_at_resolves_a_mapped_pc_and_rejects_an_unmapped_one() {
        let mut s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        // The entry PC is mapped, so a block translates (its bytes decode).
        let block = s.translate_at(s.entry_pc());
        assert!(block.is_some(), "a mapped entry PC should translate");
        // An unmapped PC yields nothing rather than panicking.
        assert!(s.translate_at(0x9_9999_0000).is_none());
    }

    /// A PE whose .text is file-backed with `code`, mapped at the entry RVA.
    fn pe_with_code(code: &[u8]) -> Vec<u8> {
        let mut buf = minimal_pe();
        let opt = 64 + 4 + 20;
        let sec = opt + 240;
        let raw_off = u32::try_from(buf.len()).unwrap();
        buf[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
        buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
        buf.extend_from_slice(code);
        buf
    }

    #[test]
    fn translate_reachable_walks_from_entry_until_a_return() {
        // mov rax, rcx ; ret  -> one block, RET is a dynamic transfer
        // (no static successor), so the walk visits exactly the entry block.
        let code = [0x48u8, 0x89, 0xC8, 0xC3];
        let mut s = Session::load(&pe_with_code(&code), &ModuleTable::new()).expect("load");
        let visited = s.translate_reachable(16);
        assert_eq!(visited, vec![s.entry_pc()]);
    }

    #[test]
    fn translate_reachable_is_bounded_and_terminates() {
        // Even on a zero-filled image the walk respects max_blocks and halts.
        let mut s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        let visited = s.translate_reachable(8);
        assert!(visited.len() <= 8);
    }

    #[test]
    fn layout_maps_the_section_and_a_stack() {
        let s = Session::load(&minimal_pe(), &ModuleTable::new()).expect("load");
        let gl = s.layout(0x2_0000_0000).expect("layout");
        // The .text section is mapped at the entry, and a stack alongside it.
        let (text, _) = gl.space.translate(s.entry_pc()).expect("entry mapped");
        assert_eq!(text.name, ".text");
        let (stack, _) = gl.space.translate(gl.stack.top - 1).expect("stack mapped");
        assert_eq!(stack.name, "stack");
    }

    #[test]
    fn backed_layout_gives_readable_code_and_a_writable_stack() {
        let s = Session::load(
            &pe_with_code(&[0x48, 0x89, 0xC8, 0xC3]),
            &ModuleTable::new(),
        )
        .expect("load");
        let mut bl = s.backed_layout(0x2_0000_0000).expect("backed layout");
        // The entry's code bytes are actually present in the byte-backed space.
        assert_eq!(
            bl.space.read(s.entry_pc(), 4).unwrap(),
            &[0x48, 0x89, 0xC8, 0xC3]
        );
        // The stack region (top is the RSP seed) is writable.
        assert!(bl.space.write(bl.stack_top - 8, &[0u8; 8]).is_ok());
    }

    // The OS-ABI run loop: a register-only program that loads the exit_group
    // number/status and traps. On ARM64 it runs to completion and reports the
    // guest's exit status; on the x86 dev host the translate path still runs and
    // the loop reports that execution is unavailable (WrongArch).
    #[test]
    fn run_drives_the_guest_to_its_exit_syscall() {
        // mov eax, 231 (exit_group); mov edi, 7 (status); syscall
        let code = [
            0xB8, 0xE7, 0x00, 0x00, 0x00, // mov eax, 231
            0xBF, 0x07, 0x00, 0x00, 0x00, // mov edi, 7
            0x0F, 0x05, // syscall
        ];
        let mut s = Session::load(&pe_with_code(&code), &ModuleTable::new()).expect("load");
        let mut mem = s.backed_layout(0x2_0000_0000).expect("backed layout").space;
        let mut ctx = SyscallContext::new();
        let mut state = CpuStateFrame::default();
        let outcome = s.run(&mut ctx, &mut mem, &mut state, 8);
        if cfg!(target_arch = "aarch64") {
            assert!(matches!(outcome, RunOutcome::Exited(7)), "{outcome:?}");
        } else {
            assert!(
                matches!(outcome, RunOutcome::ExecUnavailable(ExecError::WrongArch)),
                "{outcome:?}"
            );
        }
    }

    // prepare() composes layout + initial stack + RSP seed into a runnable
    // state: argc/argv land on the stack and RSP points at them, 16-aligned.
    #[test]
    fn prepare_seeds_rsp_at_a_well_formed_initial_stack() {
        let s = Session::load(
            &pe_with_code(&[0x48, 0x89, 0xC8, 0xC3]),
            &ModuleTable::new(),
        )
        .expect("load");
        let argv: [&[u8]; 2] = [b"prog", b"arg1"];
        let envp: [&[u8]; 1] = [b"HOME=/root"];
        let prepared = s
            .prepare(0x2_0000_0000, &argv, &envp)
            .expect("prepare a runnable state");
        let rsp = prepared.state.gpr[gpr::RSP];
        // RSP is 16-aligned and argc sits at the top of stack.
        assert_eq!(rsp & 0xF, 0);
        let argc = u64::from_le_bytes(prepared.mem.read(rsp, 8).unwrap().try_into().unwrap());
        assert_eq!(argc, 2);
        // argv[0] points to a NUL-terminated "prog".
        let argv0 = u64::from_le_bytes(prepared.mem.read(rsp + 8, 8).unwrap().try_into().unwrap());
        assert_eq!(prepared.mem.read(argv0, 5).unwrap(), b"prog\0");
    }

    // End-to-end through the composed path: prepare() then run() exit_group(9).
    // ARM64 runs it to the exit; the x86 dev host reports WrongArch — either way
    // prepare() produced a valid state and run() drove it.
    #[test]
    fn prepare_then_run_reaches_exit() {
        // mov eax, 231 (exit_group); mov edi, 9; syscall
        let code = [
            0xB8, 0xE7, 0x00, 0x00, 0x00, 0xBF, 0x09, 0x00, 0x00, 0x00, 0x0F, 0x05,
        ];
        let mut s = Session::load(&pe_with_code(&code), &ModuleTable::new()).expect("load");
        let argv: [&[u8]; 1] = [b"prog"];
        let mut p = s.prepare(0x2_0000_0000, &argv, &[]).expect("prepare");
        let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 8);
        if cfg!(target_arch = "aarch64") {
            assert!(matches!(outcome, RunOutcome::Exited(9)), "{outcome:?}");
        } else {
            assert!(
                matches!(outcome, RunOutcome::ExecUnavailable(ExecError::WrongArch)),
                "{outcome:?}"
            );
        }
    }
}
