//! Prisma execution session — the composition root (RFC 0017).
//!
//! The one place `orchestrator` (load), `prisma-translator` (translate), and
//! `prisma-runtime` (dispatch/execute) meet. A [`Session`] loads a PE into guest
//! memory, then drives the dispatcher's `run_with_callbacks` seam with two
//! adapters — a fetch over the mapped image and a translate over the
//! `Translator` — so a guest block goes from bytes to an installed ARM64
//! translation. The dispatcher owns the guest-PC stepping (which is why
//! `CpuStateFrame` has no `RIP`); this crate is just the wiring + ownership.

pub mod guest_io;
pub mod info_syscalls;
pub mod io_syscalls;
pub mod resource_syscalls;
pub mod sig_syscalls;
pub mod syscall_dispatch;
pub mod time_syscalls;

use std::collections::HashSet;

use prisma_cache::TranslationCache;
use prisma_orchestrator::address_space::AddressSpaceError;
use prisma_orchestrator::guest_layout::{layout_sections, GuestLayout};
use prisma_orchestrator::load_pe::{load_pe_with_image, LoadError};
use prisma_orchestrator::module_table::ModuleTable;
use prisma_orchestrator::pe_loader::{MappedImage, PeImage};
use prisma_runtime::dispatcher::DispatchRunOutcome;
use prisma_runtime::{Dispatcher, SmcGuard};
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
        self.translator
            .translate_block(guest_pc, &bytes, MAX_INSNS)
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
}
