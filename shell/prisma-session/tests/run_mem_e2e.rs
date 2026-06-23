//! End-to-end: a guest program writes to memory and reads it back, run through
//! the DBT against a contiguous host arena ([`Session::prepare_arena`], RFC
//! 0020). The store and the load go through the JIT's `mem_base` rebasing into
//! the same arena that backs the address space — so this proves the Stage 2B
//! memory model: translated code reaches real guest memory, coherently.
//!
//! ```text
//!   mov rbx, rsp        ; a stack pointer in a plain base register
//!   mov eax, 42         ; the value
//!   mov [rbx-8], rax    ; store it to stack memory
//!   mov rdi, [rbx-8]    ; load it back
//!   mov eax, 60         ; exit
//!   syscall             ; exit(rdi) — exit(42) iff the round-trip worked
//! ```
//!
//! Two addressing modes are covered: a plain base register (`mov rbx,rsp;
//! [rbx-8]`) and `[rsp-8]` directly (a SIB byte with the no-index encoding). The
//! latter regression-guards the decoder fix in this change — `[rsp+disp]` used to
//! decode as base+base+disp (2*rsp-8), faulting outside the arena on ARM64.
//!
//! If memory were mis-addressed (the pre-RFC-0020 host==guest assumption, or a
//! divergent second backing) the load would fault or read garbage and the exit
//! status would not be 42. aarch64-gated; on the x86 dev host the load /
//! translate / arena-prepare path runs and `run` reports execution unavailable.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::{RunOutcome, Session};

/// A minimal PE32+ with one `.text` section (image base 0x1_4000_0000, entry RVA
/// 0x1000, virtual size 0x10000) whose file-backed bytes are `code`.
fn pe_with_code(code: &[u8]) -> Vec<u8> {
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
    let raw_off = u32::try_from(buf.len()).unwrap();
    buf[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

#[rustfmt::skip]
const MEM_PROGRAM: [u8; 23] = [
    0x48, 0x89, 0xE3,             // mov rbx, rsp
    0xB8, 0x2A, 0x00, 0x00, 0x00, // mov eax, 42
    0x48, 0x89, 0x43, 0xF8,       // mov [rbx-8], rax
    0x48, 0x8B, 0x7B, 0xF8,       // mov rdi, [rbx-8]   (load it back)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60        (exit)
    0x0F, 0x05,                   // syscall            (exit(rdi))
];

// A stack base just above the image's `.text` (image 0x1_4000_0000, section
// [..1000, ..11000)) keeps the arena window compact (~1.1 MiB) so it allocates
// eagerly on every host — the arena is one contiguous mapping.
const STACK_BASE: u64 = 0x1_4002_0000;

#[test]
fn guest_round_trips_through_real_memory() {
    let mut s = Session::load(&pe_with_code(&MEM_PROGRAM), &ModuleTable::new())
        .expect("load the mem program");
    let mut p = s
        .prepare_arena(STACK_BASE, &[b"prog"], &[])
        .expect("prepare against a contiguous arena");
    // The arena exposes a single rebase offset the JIT uses for every access.
    assert_ne!(p.state.mem_base, 0, "arena mode seeds a non-zero mem_base");

    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 8);

    if cfg!(target_arch = "aarch64") {
        // The value stored to [rbx-8] was read back into rdi, so exit == 42.
        assert!(
            matches!(outcome, RunOutcome::Exited(42)),
            "outcome={:?} rdi={}",
            outcome,
            p.state.gpr[7], // RDI
        );
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "off-target the run reports execution is unavailable: {outcome:?}"
        );
    }
}

// The same round-trip, but addressed through `[rsp-8]` directly — a SIB byte
// with the no-index encoding (index field 0b100). This exercises the decoder
// fix: before it, `[rsp+disp]` decoded as base+base+disp (2*rsp-8), landing
// outside the arena and faulting on ARM64. Now it is a single base+disp and the
// round-trip exits 42.
#[rustfmt::skip]
const SIB_PROGRAM: [u8; 22] = [
    0xB8, 0x2A, 0x00, 0x00, 0x00, // mov eax, 42
    0x48, 0x89, 0x44, 0x24, 0xF8, // mov [rsp-8], rax   (SIB, no index)
    0x48, 0x8B, 0x7C, 0x24, 0xF8, // mov rdi, [rsp-8]   (load it back)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60        (exit)
    0x0F, 0x05,                   // syscall            (exit(rdi))
];

#[test]
fn guest_round_trips_through_rsp_relative_memory() {
    let mut s = Session::load(&pe_with_code(&SIB_PROGRAM), &ModuleTable::new())
        .expect("load the SIB mem program");
    let mut p = s
        .prepare_arena(STACK_BASE, &[b"prog"], &[])
        .expect("prepare against a contiguous arena");

    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 8);

    if cfg!(target_arch = "aarch64") {
        // [rsp-8] now decodes as a single base+disp, so the round-trip exits 42.
        assert!(
            matches!(outcome, RunOutcome::Exited(42)),
            "outcome={:?} rdi={}",
            outcome,
            p.state.gpr[7], // RDI
        );
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "{outcome:?}"
        );
    }
}
