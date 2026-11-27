//! Build a two-segment ELF in memory. No cross compiler or host executable needed.

use prisma_runtime::executor::{gpr, ExecError};
use prisma_session::{RunOutcome, Session};

const BASE: u64 = 0x400000;
const DATA: u64 = BASE + 0x1000;
const STACK: u64 = BASE + 0x4000;
const ENTRY: u64 = BASE + 0xe0;
const CODE: &[u8] = &[0xb8, 60, 0, 0, 0, 0xbf, 23, 0, 0, 0, 0x0f, 0x05];

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn elf() -> Vec<u8> {
    let mut bytes = vec![0; 0x1003];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut bytes, 16, 2);
    put16(&mut bytes, 18, 62);
    put32(&mut bytes, 20, 1);
    put64(&mut bytes, 24, ENTRY);
    put64(&mut bytes, 32, 64);
    put16(&mut bytes, 52, 64);
    put16(&mut bytes, 54, 56);
    put16(&mut bytes, 56, 2);
    for (index, flags, offset, va, filesz, memsz) in [
        (
            0,
            5,
            0,
            BASE,
            0xe0 + CODE.len() as u64,
            0xe0 + CODE.len() as u64,
        ),
        (1, 6, 0x1000, DATA, 3, 64),
    ] {
        let ph = 64 + index * 56;
        put32(&mut bytes, ph, 1);
        put32(&mut bytes, ph + 4, flags);
        put64(&mut bytes, ph + 8, offset);
        put64(&mut bytes, ph + 16, va);
        put64(&mut bytes, ph + 32, filesz);
        put64(&mut bytes, ph + 40, memsz);
        put64(&mut bytes, ph + 48, 0x1000);
    }
    bytes[0xe0..0xe0 + CODE.len()].copy_from_slice(CODE);
    bytes[0x1000..].copy_from_slice(b"abc");
    bytes
}

#[test]
fn maps_static_image_and_enters_the_linux_run_loop() {
    let mut session = Session::load_static_elf(&elf()).unwrap();
    assert_eq!(session.entry_pc(), ENTRY);
    assert_eq!(session.fetch(ENTRY).as_deref(), Some(CODE));
    assert!(session.fetch(DATA).is_none());
    assert!(session.fetch(ENTRY + CODE.len() as u64).is_none());
    assert!(session.translate_at(ENTRY).is_some());
    let layout = session.layout(STACK).unwrap();
    assert!(!layout.space.translate(DATA).unwrap().0.prot.is_executable());
    assert!(session.prepare_arena(BASE, &[], &[]).is_err());

    for mut run in [
        session.prepare(STACK, &[b"demo"], &[b"A=1"]).unwrap(),
        session.prepare_arena(STACK, &[b"demo"], &[b"A=1"]).unwrap(),
    ] {
        assert_eq!(run.mem.read(DATA, 6).unwrap(), b"abc\0\0\0");
        assert!(run.mem.write(DATA, b"A").is_ok());
        assert!(run.mem.write(ENTRY, &[0]).is_err());
        assert_eq!(run.ctx.brk, DATA + 0x1000);
        let rsp = run.state.gpr[gpr::RSP];
        assert_eq!(rsp % 16, 0);
        let word =
            |n: u64| u64::from_le_bytes(run.mem.read(rsp + 8 * n, 8).unwrap().try_into().unwrap());
        assert_eq!(word(0), 1);
        assert_eq!(run.mem.read(word(1), 5).unwrap(), b"demo\0");
        assert_eq!(run.mem.read(word(3), 4).unwrap(), b"A=1\0");
        assert_eq!((word(5), word(6)), (6, 4096));
        assert_eq!((word(7), word(8)), (9, ENTRY));
        assert_eq!((word(9), word(10)), (3, BASE + 64));
        assert_eq!((word(11), word(12)), (4, 56));
        assert_eq!((word(13), word(14)), (5, 2));
        assert_eq!((word(15), word(16)), (0, 0));
        if run.mem.mem_base().is_some() {
            let result = session.run(&mut run.ctx, &mut run.mem, &mut run.state, 8);
            if cfg!(target_arch = "aarch64") {
                assert!(matches!(result, RunOutcome::Exited(23)), "{result:?}");
            } else {
                assert!(
                    matches!(result, RunOutcome::ExecUnavailable(ExecError::WrongArch)),
                    "{result:?}"
                );
            }
        }
    }
}

#[test]
fn rejects_unsupported_and_inconsistent_images() {
    for (label, change) in [
        ("wrong machine", (18, 183u8)),
        ("PIE", (16, 3)),
        ("class", (4, 1)),
        ("endianness", (5, 2)),
        ("interpreter", (120, 3)),
        ("dynamic", (120, 2)),
        ("TLS", (120, 7)),
        ("writable code", (68, 7)),
        ("invalid phentsize", (54, 24)),
        ("bad alignment", (120 + 48, 3)),
        ("file exceeds memsz", (120 + 40, 1)),
    ] {
        let mut bytes = elf();
        bytes[change.0] = change.1;
        assert!(Session::load_static_elf(&bytes).is_err(), "{label}");
    }
    for (name, field, value) in [
        ("non-executable entry", 24, DATA),
        ("truncated headers", 32, 0x1000),
        ("overlap", 120 + 16, BASE),
        ("overflow", 120 + 40, u64::MAX),
        ("out of file", 120 + 8, 0x9000),
        ("sparse image", 120 + 16, BASE + (1 << 30)),
    ] {
        let mut bytes = elf();
        put64(&mut bytes, field, value);
        assert!(Session::load_static_elf(&bytes).is_err(), "{name}");
    }
    let bytes = elf();
    for n in 0..bytes.len() {
        assert!(Session::load_static_elf(&bytes[..n]).is_err(), "prefix {n}");
    }
}

#[test]
fn fetch_crosses_adjacent_executable_segments_only() {
    let mut bytes = elf();
    put64(&mut bytes, 64 + 40, 0x1000);
    put32(&mut bytes, 120 + 4, 5);
    let session = Session::load_static_elf(&bytes).unwrap();
    let window = session.fetch(DATA - 2).unwrap();
    assert_eq!(&window[..6], b"\0\0abc\0");
    assert_eq!(window.len(), 66);
    // The same bytes are readable, but no longer fetchable, as a RW segment.
    put32(&mut bytes, 120 + 4, 6);
    let session = Session::load_static_elf(&bytes).unwrap();
    assert_eq!(session.fetch(DATA - 2).unwrap(), vec![0; 2]);
    assert!(session.fetch(DATA).is_none());
}

#[test]
fn zero_filesize_segment_needs_no_file_mapping() {
    let mut bytes = elf();
    put64(&mut bytes, 120 + 8, 0x5000);
    put64(&mut bytes, 120 + 32, 0);
    let session = Session::load_static_elf(&bytes).unwrap();
    let prepared = session.prepare_arena(STACK, &[], &[]).unwrap();
    assert_eq!(prepared.mem.read(DATA, 64).unwrap(), &[0; 64]);
}
