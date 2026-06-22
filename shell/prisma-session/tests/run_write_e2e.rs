//! End-to-end: a real guest program prints through the full DBT.
//!
//! The guest does `write(fd, msg, len); exit(0)` — the first observable output
//! produced by translating x86 to ARM64, executing it, and servicing the
//! resulting syscalls through `Session::run`. The message lives in the guest's
//! `.text` (after the code) and is passed to `write` by absolute VA; the syscall
//! layer reads it out of guest memory and writes it to a `File`-backed fd, which
//! the test reads back.
//!
//! Execution is `aarch64`-gated (the translated bytes are ARM64 machine code):
//! on the ARM64 CI runner the program runs to `exit(0)` and the file holds the
//! message; on the x86 dev host the load/translate/prepare path still runs and
//! `run` reports `WrongArch` before any syscall. The file is always removed —
//! deterministic resource release.

use std::io::Read;

use prisma_orchestrator::module_table::ModuleTable;
use prisma_runtime::fd_table::FdEntry;
use prisma_session::{RunOutcome, Session};

/// Image base + entry RVA of the test PE (mirrors the load_pe fixture).
const ENTRY_VA: u64 = 0x1_4000_1000;
/// The bytes of the program's instructions, before the appended message.
const INSN_LEN: u64 = 39;
const MESSAGE: &[u8] = b"Prisma DBT\n";

/// A minimal PE32+ with one `.text` section (base 0x1_4000_0000, entry RVA
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
    buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual size
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // RVA
    let raw_off = u32::try_from(buf.len()).unwrap();
    buf[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

/// `write(fd, MESSAGE, len); exit(0)`, with `MESSAGE` appended after the code.
fn write_then_exit_program(fd: u32) -> Vec<u8> {
    let msg_va = ENTRY_VA + INSN_LEN;
    let mut code = Vec::new();
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]); // mov eax, 1 (write)
    code.extend_from_slice(&[0xBF]); // mov edi, imm32 (fd)
    code.extend_from_slice(&fd.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xBE]); // movabs rsi, imm64 (msg ptr)
    code.extend_from_slice(&msg_va.to_le_bytes());
    code.extend_from_slice(&[0xBA]); // mov edx, imm32 (len)
    code.extend_from_slice(&(MESSAGE.len() as u32).to_le_bytes());
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    code.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]); // mov eax, 60 (exit)
    code.extend_from_slice(&[0xBF, 0x00, 0x00, 0x00, 0x00]); // mov edi, 0 (status 0)
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    assert_eq!(
        code.len() as u64,
        INSN_LEN,
        "INSN_LEN must match the program"
    );
    code.extend_from_slice(MESSAGE); // the message data at msg_va
    code
}

#[test]
fn guest_write_then_exit_prints_through_the_dbt() {
    let mut s = Session::load(
        &pe_with_code(&write_then_exit_program(3)),
        &ModuleTable::new(),
    )
    .expect("load the guest program");
    let mut p = s.prepare(0x2_0000_0000, &[b"prog"], &[]).expect("prepare");

    // A File-backed guest fd to capture the write. Allocating it after the
    // standard 0/1/2 yields fd 3 — the fd the program writes to.
    let path = std::env::temp_dir().join(format!("prisma_write_e2e_{}.out", std::process::id()));
    let file = std::fs::File::create(&path).expect("create capture file");
    let fd = p
        .ctx
        .fds
        .allocate(FdEntry::File(file))
        .expect("allocate fd");
    assert_eq!(fd, 3, "the capture fd must match the program's fd");

    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 8);

    if cfg!(target_arch = "aarch64") {
        assert!(matches!(outcome, RunOutcome::Exited(0)), "{outcome:?}");
        // Drop the fd so the File flushes/closes, then read the captured bytes.
        let _ = p.ctx.fds.close(3);
        let mut got = Vec::new();
        std::fs::File::open(&path)
            .expect("reopen capture file")
            .read_to_end(&mut got)
            .expect("read capture file");
        assert_eq!(got, MESSAGE, "the guest's write must reach the fd verbatim");
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "off-target the run reports execution is unavailable: {outcome:?}"
        );
    }

    // Deterministic cleanup regardless of arch.
    std::fs::remove_file(&path).ok();
}
