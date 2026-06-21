//! Cross-crate integration: the execution-seed pieces compose coherently.
//!
//! Loading a PE (orchestrator) then laying out its address space + an initial
//! stack (orchestrator) then seeding the initial CPU frame (runtime) is the
//! path the session takes before running a guest. Each piece is unit-tested in
//! isolation; this asserts the seam between them — that the RSP the runtime
//! seeds actually lands, 16-aligned, inside the stack region the loader mapped.

use prisma_orchestrator::guest_layout::layout;
use prisma_orchestrator::load_pe::load_pe;
use prisma_orchestrator::module_table::ModuleTable;
use prisma_runtime::executor::gpr;
use prisma_runtime::guest_thread::GuestThread;

/// Minimal valid PE32+ (no imports), image base 0x1_4000_0000, size 0x10000.
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
fn seeded_rsp_lands_aligned_inside_the_mapped_stack() {
    // Load -> lay out image + stack -> seed the initial frame.
    let image = load_pe(&minimal_pe(), &ModuleTable::new()).expect("load");
    let stack_base = 0x2_0000_0000;
    let gl = layout(&image, stack_base).expect("layout");
    let frame = GuestThread::initial(gl.stack.top);

    let rsp = frame.gpr[gpr::RSP];
    // 16-aligned per the x64 ABI at the entry boundary.
    assert_eq!(rsp % 16, 0, "RSP must be 16-aligned");
    // And it must point inside the stack region the loader actually mapped, so
    // the very first push writes mapped memory rather than faulting.
    let (region, _off) = gl
        .space
        .translate(rsp.saturating_sub(1))
        .expect("the seeded stack pointer is inside a mapped region");
    assert_eq!(region.name, "stack");
    assert!(region.prot.is_writable(), "the stack must be writable");
    // The image is mapped in the same space and does not overlap the stack.
    let (img_region, _) = gl.space.translate(image.entry_pc).expect("entry mapped");
    assert_eq!(img_region.name, "image");
}

#[test]
fn stack_and_image_are_disjoint_in_the_seeded_space() {
    let image = load_pe(&minimal_pe(), &ModuleTable::new()).expect("load");
    let gl = layout(&image, 0x2_0000_0000).expect("layout");
    // Exactly two regions, non-overlapping (the address space guarantees it).
    assert_eq!(gl.space.regions().len(), 2);
    assert_eq!(gl.image_base, image.base);
}
