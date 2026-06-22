//! System V AMD64 initial process stack (`argc`/`argv`/`envp`/`auxv`).
//!
//! The kernel hands a fresh process a stack already populated with its
//! arguments, environment, and auxiliary vector; `_start` reads them straight
//! off `RSP`. Emulating a real program therefore means building that exact image
//! before the run loop seeds `RSP`. This lays it out in a [`BackedAddressSpace`]
//! stack region (the byte-backed space the run loop executes against) and returns
//! the `RSP` value to seed.
//!
//! Layout, from `RSP` upward (low to high address), per the System V AMD64 ABI
//! process-initialization sequence:
//!
//! ```text
//!   RSP ->  argc                      (8 bytes)
//!           argv[0..argc] pointers    (8 each)
//!           NULL                      (8)
//!           envp[0..] pointers        (8 each)
//!           NULL                      (8)
//!           auxv: (type, value) pairs (16 each)
//!           AT_NULL (0, 0)            (16)
//!           ...alignment padding...
//!           string area (argv then envp strings, NUL-terminated)
//!   stack_top
//! ```
//!
//! `RSP` is 16-byte aligned at entry, as `_start` requires. The argument/env
//! strings live at the high end (just below `stack_top`); the pointer vectors
//! point up into them.

use crate::address_space::RangeError;
use crate::backed_address_space::BackedAddressSpace;

/// The argument, environment, and auxiliary-vector inputs for a process stack.
///
/// `argv`/`envp` are the raw strings without their terminating NUL (added here).
/// `auxv` is the `(a_type, a_val)` pairs *excluding* the terminating `AT_NULL`,
/// which is appended. Pointer-valued aux entries (e.g. `AT_RANDOM`) must already
/// reference memory the caller placed; this lays out the vector verbatim.
pub struct ProcessStackParams<'a> {
    pub argv: &'a [&'a [u8]],
    pub envp: &'a [&'a [u8]],
    pub auxv: &'a [(u64, u64)],
}

/// Why building the initial stack failed.
#[derive(Debug)]
pub enum StackBuildError {
    /// A write ran off the stack region (the stack is too small for the
    /// argument/environment/auxv image).
    Range(RangeError),
    /// Address arithmetic underflowed below address 0 while descending from
    /// `stack_top` (an absurdly large argument image).
    Underflow,
}

impl From<RangeError> for StackBuildError {
    fn from(e: RangeError) -> Self {
        Self::Range(e)
    }
}

/// Descend `top` by `n`, failing on underflow rather than wrapping.
fn sub(top: u64, n: u64) -> Result<u64, StackBuildError> {
    top.checked_sub(n).ok_or(StackBuildError::Underflow)
}

/// Build the initial process stack into the stack region of `mem` and return the
/// `RSP` to seed.
///
/// The strings are written at the high end (descending from `stack_top`), then
/// the `argc`/pointer/auxv vector is written below them at a 16-byte-aligned
/// `RSP`. Every write is bounds-checked against the region, so a stack too small
/// for the image fails with [`StackBuildError::Range`] rather than corrupting a
/// neighbour.
///
/// # Errors
/// [`StackBuildError`] if the image does not fit the stack region, or if
/// descending from `stack_top` underflows.
pub fn build_initial_stack(
    mem: &mut BackedAddressSpace,
    stack_top: u64,
    params: &ProcessStackParams,
) -> Result<u64, StackBuildError> {
    // 1. Lay the strings out at the top of the stack, descending, recording the
    //    guest pointer to each. argv strings first, then envp strings.
    let mut cursor = stack_top;
    let mut argv_ptrs = Vec::with_capacity(params.argv.len());
    let mut envp_ptrs = Vec::with_capacity(params.envp.len());
    for (src, ptrs) in [(params.argv, &mut argv_ptrs), (params.envp, &mut envp_ptrs)] {
        for s in src {
            cursor = sub(cursor, s.len() as u64 + 1)?; // +1 for the NUL
            mem.write(cursor, s)?;
            mem.write(cursor + s.len() as u64, &[0u8])?;
            ptrs.push(cursor);
        }
    }

    // 2. Size the vector block: argc + argv ptrs + NULL + envp ptrs + NULL +
    //    auxv pairs + AT_NULL. Then place it below the strings at a 16-aligned
    //    RSP so _start sees an aligned stack.
    let ptr_words = 1 // argc
        + params.argv.len() + 1 // argv pointers + NULL terminator
        + params.envp.len() + 1; // envp pointers + NULL terminator
    let auxv_words = (params.auxv.len() + 1) * 2; // (type,val) pairs + AT_NULL
    let block_bytes = (ptr_words + auxv_words) as u64 * 8;
    let rsp = sub(cursor, block_bytes)? & !0xF; // 16-byte align (round down)

    // 3. Write the vector block ascending from RSP.
    let mut at = rsp;
    let mut put = |mem: &mut BackedAddressSpace, v: u64| -> Result<(), StackBuildError> {
        mem.write(at, &v.to_le_bytes())?;
        at += 8;
        Ok(())
    };
    put(mem, params.argv.len() as u64)?; // argc
    for p in &argv_ptrs {
        put(mem, *p)?;
    }
    put(mem, 0)?; // argv NULL
    for p in &envp_ptrs {
        put(mem, *p)?;
    }
    put(mem, 0)?; // envp NULL
    for (ty, val) in params.auxv {
        put(mem, *ty)?;
        put(mem, *val)?;
    }
    put(mem, 0)?; // AT_NULL type
    put(mem, 0)?; // AT_NULL value
    Ok(rsp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address_space::Protection;

    // Map a generous stack [base, base+len) and return the space + its top.
    fn stack_space(base: u64, len: u64) -> (BackedAddressSpace, u64) {
        let mut mem = BackedAddressSpace::new();
        mem.map(base, len, Protection::ReadWrite)
            .expect("map stack");
        (mem, base + len)
    }

    // Read a u64 from the guest stack.
    fn read_u64(mem: &BackedAddressSpace, addr: u64) -> u64 {
        let bytes = mem.read(addr, 8).expect("read");
        u64::from_le_bytes(bytes.try_into().unwrap())
    }

    #[test]
    fn lays_out_argc_argv_envp_auxv_with_aligned_rsp() {
        let (mut mem, top) = stack_space(0x10_0000, 0x4000);
        let argv: [&[u8]; 2] = [b"prog", b"--flag"];
        let envp: [&[u8]; 1] = [b"PATH=/bin"];
        let auxv = [(6u64, 0x1000u64), (25u64, 0xBEEFu64)]; // AT_PAGESZ, AT_RANDOM
        let params = ProcessStackParams {
            argv: &argv,
            envp: &envp,
            auxv: &auxv,
        };
        let rsp = build_initial_stack(&mut mem, top, &params).expect("build");

        // RSP is 16-byte aligned and inside the stack.
        assert_eq!(rsp & 0xF, 0);
        assert!(rsp >= 0x10_0000 && rsp < top);

        // argc at RSP.
        assert_eq!(read_u64(&mem, rsp), 2);

        // argv[0] / argv[1] point to the NUL-terminated strings.
        let argv0 = read_u64(&mem, rsp + 8);
        let argv1 = read_u64(&mem, rsp + 16);
        assert_eq!(mem.read(argv0, 5).unwrap(), b"prog\0");
        assert_eq!(mem.read(argv1, 7).unwrap(), b"--flag\0");
        // argv NULL terminator.
        assert_eq!(read_u64(&mem, rsp + 24), 0);

        // envp[0] then its NULL terminator.
        let envp0 = read_u64(&mem, rsp + 32);
        assert_eq!(mem.read(envp0, 9).unwrap(), b"PATH=/bin");
        assert_eq!(read_u64(&mem, rsp + 40), 0);

        // auxv pairs follow, then AT_NULL.
        assert_eq!(read_u64(&mem, rsp + 48), 6); // AT_PAGESZ type
        assert_eq!(read_u64(&mem, rsp + 56), 0x1000);
        assert_eq!(read_u64(&mem, rsp + 64), 25); // AT_RANDOM type
        assert_eq!(read_u64(&mem, rsp + 72), 0xBEEF);
        assert_eq!(read_u64(&mem, rsp + 80), 0); // AT_NULL type
        assert_eq!(read_u64(&mem, rsp + 88), 0); // AT_NULL value
    }

    #[test]
    fn empty_argv_envp_auxv_is_just_argc_and_terminators() {
        let (mut mem, top) = stack_space(0x20_0000, 0x1000);
        let params = ProcessStackParams {
            argv: &[],
            envp: &[],
            auxv: &[],
        };
        let rsp = build_initial_stack(&mut mem, top, &params).expect("build");
        assert_eq!(rsp & 0xF, 0);
        assert_eq!(read_u64(&mem, rsp), 0); // argc = 0
        assert_eq!(read_u64(&mem, rsp + 8), 0); // argv NULL
        assert_eq!(read_u64(&mem, rsp + 16), 0); // envp NULL
        assert_eq!(read_u64(&mem, rsp + 24), 0); // AT_NULL type
        assert_eq!(read_u64(&mem, rsp + 32), 0); // AT_NULL value
    }

    #[test]
    fn a_stack_too_small_for_the_image_fails_cleanly() {
        // A 32-byte stack cannot hold a long argument plus its vector.
        let (mut mem, top) = stack_space(0x30_0000, 0x20);
        let big = [b"this-argument-is-far-too-long-for-the-tiny-stack" as &[u8]];
        let params = ProcessStackParams {
            argv: &big,
            envp: &[],
            auxv: &[],
        };
        assert!(matches!(
            build_initial_stack(&mut mem, top, &params),
            Err(StackBuildError::Range(_))
        ));
    }
}
