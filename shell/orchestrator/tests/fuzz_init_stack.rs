//! Property-based robustness fuzzing for the System V initial process stack.
//!
//! `build_initial_stack` writes guest-controlled data (argv/envp/auxv) into a
//! fixed-size stack region, so its load-bearing properties are: it never panics
//! for any input, it never writes outside the stack region (a too-large image
//! must fail cleanly rather than corrupt a neighbour), and on success the result
//! is a well-formed stack — `RSP` 16-byte aligned, inside the region, with
//! `argc` and the argv/envp pointers reading back exactly what went in. These
//! assert that over arbitrary arguments, environment, auxv, and stack sizes.

use proptest::prelude::*;

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_orchestrator::init_stack::{build_initial_stack, ProcessStackParams};

const STACK_BASE: u64 = 0x10_0000;

/// A mapped read-write stack of `len` bytes and its top.
fn stack(len: u64) -> (BackedAddressSpace, u64) {
    let mut mem = BackedAddressSpace::new();
    mem.map(STACK_BASE, len, Protection::ReadWrite)
        .expect("map stack");
    (mem, STACK_BASE + len)
}

fn read_u64(mem: &BackedAddressSpace, addr: u64) -> u64 {
    let b = mem.read(addr, 8).expect("read");
    u64::from_le_bytes(b.try_into().unwrap())
}

proptest! {
    /// The headline property: for any argv/envp/auxv and any stack size,
    /// building never panics and never writes outside the stack region. On
    /// success the stack is well-formed — `RSP` is 16-aligned and inside the
    /// region, and `argc` reads back the argv count.
    #[test]
    fn build_never_panics_or_escapes_and_is_well_formed(
        argv in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..24), 0..8),
        envp in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..24), 0..8),
        auxv in prop::collection::vec((any::<u64>(), any::<u64>()), 0..8),
        stack_len in 64u64..0x4000,
    ) {
        let (mut mem, top) = stack(stack_len);
        let argv_refs: Vec<&[u8]> = argv.iter().map(Vec::as_slice).collect();
        let envp_refs: Vec<&[u8]> = envp.iter().map(Vec::as_slice).collect();
        let params = ProcessStackParams {
            argv: &argv_refs,
            envp: &envp_refs,
            auxv: &auxv,
        };
        // Must not panic; either a clean error or a well-formed RSP.
        if let Ok(rsp) = build_initial_stack(&mut mem, top, &params) {
            prop_assert_eq!(rsp & 0xF, 0, "RSP must be 16-byte aligned");
            prop_assert!(rsp >= STACK_BASE && rsp < top, "RSP must be in the stack");
            prop_assert_eq!(read_u64(&mem, rsp), argv.len() as u64, "argc");
        }
    }

    /// On success every argv/envp pointer in the vector references a NUL-
    /// terminated copy of the original string inside the stack — the guest can
    /// actually dereference what `argc`/`argv` advertise.
    #[test]
    fn argv_envp_pointers_reach_their_strings(
        argv in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..16), 0..6),
        envp in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..16), 0..6),
    ) {
        // A roomy stack so the image fits and we exercise the success path.
        let (mut mem, top) = stack(0x4000);
        let argv_refs: Vec<&[u8]> = argv.iter().map(Vec::as_slice).collect();
        let envp_refs: Vec<&[u8]> = envp.iter().map(Vec::as_slice).collect();
        let params = ProcessStackParams {
            argv: &argv_refs,
            envp: &envp_refs,
            auxv: &[],
        };
        let rsp = build_initial_stack(&mut mem, top, &params).expect("roomy stack fits");

        // argv pointers start at RSP+8; after argc(+8) + argv(+8*n) + NULL(+8)
        // come the envp pointers.
        for (i, s) in argv.iter().enumerate() {
            let p = read_u64(&mem, rsp + 8 + (i as u64) * 8);
            prop_assert_eq!(mem.read(p, s.len()).unwrap(), s.as_slice());
            prop_assert_eq!(mem.read(p + s.len() as u64, 1).unwrap(), &[0u8]); // NUL
        }
        let envp_base = rsp + 8 + (argv.len() as u64 + 1) * 8;
        for (i, s) in envp.iter().enumerate() {
            let p = read_u64(&mem, envp_base + (i as u64) * 8);
            prop_assert_eq!(mem.read(p, s.len()).unwrap(), s.as_slice());
            prop_assert_eq!(mem.read(p + s.len() as u64, 1).unwrap(), &[0u8]);
        }
    }
}
