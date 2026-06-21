//! Full-pipeline integration: load -> lay out -> seed (with TEB) -> translate.
//!
//! Each subsystem has unit + per-seam tests; this is the widest check — that the
//! whole set of merged pieces composes coherently on one realistic code-bearing
//! PE: the loader maps it, the layout gives it a stack, the thread is seeded
//! with RSP in that stack and GS at a materialised TEB whose stack bounds match
//! the layout, and the session translates the reachable code from the entry.

use prisma_orchestrator::guest_layout::layout;
use prisma_orchestrator::load_pe::load_pe;
use prisma_orchestrator::module_table::ModuleTable;
use prisma_runtime::executor::gpr;
use prisma_runtime::guest_thread::GuestThread;
use prisma_runtime::teb::Teb;
use prisma_session::Session;

/// A minimal PE32+ whose file-backed `.text` holds `code`, mapped at the entry.
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

#[test]
fn load_layout_seed_translate_all_agree() {
    // mov rax, rcx ; ret  — one block, terminates on RET.
    let pe = pe_with_code(&[0x48, 0x89, 0xC8, 0xC3]);

    // 1. Load: the loader maps the image and reports the entry.
    let image = load_pe(&pe, &ModuleTable::new()).expect("load");
    assert_eq!(image.entry_pc, 0x1_4000_1000);

    // 2. Lay out: image + a stack in one address space.
    let gl = layout(&image, 0x2_0000_0000).expect("layout");

    // 3. Seed the thread: RSP in the stack, GS at a TEB we materialise whose
    //    stack bounds come from the same layout.
    let teb_addr = 0x3_0000_0000;
    let mut frame = GuestThread::initial(gl.stack.top);
    frame.gs_base = teb_addr; // GS = TEB base (x64 convention)
    let teb = Teb {
        addr: teb_addr,
        stack_base: gl.stack.top,
        stack_limit: gl.stack.base,
        peb: 0,
    };
    let teb_bytes = teb.to_bytes();

    // RSP is inside the mapped stack; GS points at the TEB.
    let rsp = frame.gpr[gpr::RSP];
    let (region, _) = gl
        .space
        .translate(rsp - 1)
        .expect("RSP inside a mapped region");
    assert_eq!(region.name, "stack");
    assert_eq!(frame.gs_base, teb_addr);
    // The TEB's StackBase/StackLimit bracket RSP — gs:[8]/gs:[10] are coherent.
    let stack_base = u64::from_le_bytes(teb_bytes[8..16].try_into().unwrap());
    let stack_limit = u64::from_le_bytes(teb_bytes[16..24].try_into().unwrap());
    assert!(stack_limit <= rsp && rsp <= stack_base);

    // 4. Translate: the session walks the reachable CFG from the entry.
    let mut session = Session::load(&pe, &ModuleTable::new()).expect("session load");
    let visited = session.translate_reachable(16);
    assert_eq!(visited, vec![image.entry_pc]);
}
