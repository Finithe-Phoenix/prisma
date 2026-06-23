//! End-to-end: `arch_prctl` sets and reads the FS segment base through the run
//! loop, and the value round-trips through real guest memory (RFC 0020).
//!
//! ```text
//!   arch_prctl(ARCH_SET_FS, 42)    ; fs_base = 42
//!   mov rbx, rsp                   ; a writable stack slot
//!   arch_prctl(ARCH_GET_FS, rbx)   ; *rbx = fs_base
//!   mov rdi, [rbx]                 ; load it back
//!   exit(rdi)                      ; exit(42) iff set+get worked
//! ```
//!
//! The GET writes via the syscall layer (`mem.write`) and the load reads via the
//! JIT's `mem_base` rebasing — the same arena, so the round-trip also exercises
//! syscall/JIT memory coherence. aarch64-gated; x86 reports execution
//! unavailable after translating.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::{RunOutcome, Session};

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
const TLS_PROGRAM: [u8; 45] = [
    0xB8, 0x9E, 0x00, 0x00, 0x00, // mov eax, 158   (arch_prctl)
    0xBF, 0x02, 0x10, 0x00, 0x00, // mov edi, 0x1002 (ARCH_SET_FS)
    0xBE, 0x2A, 0x00, 0x00, 0x00, // mov esi, 42     (fs_base = 42)
    0x0F, 0x05,                   // syscall
    0x48, 0x89, 0xE3,             // mov rbx, rsp
    0xB8, 0x9E, 0x00, 0x00, 0x00, // mov eax, 158   (arch_prctl)
    0xBF, 0x03, 0x10, 0x00, 0x00, // mov edi, 0x1003 (ARCH_GET_FS)
    0x48, 0x89, 0xDE,             // mov rsi, rbx    (out-pointer)
    0x0F, 0x05,                   // syscall         (*rbx = fs_base)
    0x48, 0x8B, 0x3B,             // mov rdi, [rbx]  (load it back)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60     (exit)
    0x0F, 0x05,                   // syscall         (exit(rdi))
];

const STACK_BASE: u64 = 0x1_4002_0000;

#[test]
fn guest_arch_prctl_fs_base_round_trips() {
    let mut s = Session::load(&pe_with_code(&TLS_PROGRAM), &ModuleTable::new())
        .expect("load the TLS program");
    let mut p = s
        .prepare_arena(STACK_BASE, &[b"prog"], &[])
        .expect("prepare against a contiguous arena");

    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 16);

    if cfg!(target_arch = "aarch64") {
        assert_eq!(p.state.fs_base, 42, "ARCH_SET_FS set the frame's fs_base");
        // GET_FS wrote 42 to [rbx]; the load read it back, so exit == 42.
        assert!(
            matches!(outcome, RunOutcome::Exited(42)),
            "outcome={:?} fs_base={}",
            outcome,
            p.state.fs_base,
        );
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "{outcome:?}"
        );
    }
}
