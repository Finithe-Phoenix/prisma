//! System-information syscalls.
//!
//! Handlers that report fixed facts about the (emulated) system to the guest.
//! They write a packed struct through a [`GuestRegion`], so a bad pointer is a
//! guest `EFAULT` rather than an out-of-bounds host write.

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_structs::{Rusage, Utsname};

/// The fixed identity the guest sees: an x86-64 Linux system. The syscall layer
/// models the x86-64 Linux ABI, so `uname` reports a matching kernel rather than
/// the real ARM64/Windows host underneath.
const GUEST_UTSNAME: Utsname = Utsname {
    sysname: "Linux",
    nodename: "prisma",
    release: "6.1.0-prisma",
    version: "#1 SMP PREEMPT_DYNAMIC Prisma",
    machine: "x86_64",
    domainname: "(none)",
};

/// `uname(buf)`: write the system identification (`new_utsname`) to the guest
/// struct at `buf`.
///
/// # Errors
/// [`RangeError`] if `buf` is not writable guest memory for the full
/// [`Utsname::SIZE`] bytes (guest `EFAULT`).
pub fn uname(mem: &mut GuestRegion, buf: u64) -> Result<(), RangeError> {
    mem.write(buf, &GUEST_UTSNAME.to_guest_bytes())
}

/// `who` selectors `getrusage` accepts.
const RUSAGE_SELF: i32 = 0;
const RUSAGE_CHILDREN: i32 = -1;
const RUSAGE_THREAD: i32 = 1;

/// Why `getrusage` failed (each maps to a guest errno at routing time).
#[derive(Debug)]
pub enum RusageError {
    /// `who` is not `RUSAGE_SELF`/`CHILDREN`/`THREAD` — guest `EINVAL`.
    InvalidWho,
    /// The out pointer is not writable guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `getrusage(who, usage)`: write the resource-usage counters for `who` to the
/// guest `rusage` at `usage`.
///
/// No per-process resource accounting is modelled yet, so an all-zero `rusage`
/// is reported regardless of `who` — the honest value when nothing is tracked.
///
/// # Errors
/// [`RusageError::InvalidWho`] if `who` is not a recognised selector,
/// [`RusageError::Fault`] if `usage` is not writable guest memory.
pub fn getrusage(mem: &mut GuestRegion, who: i32, usage: u64) -> Result<(), RusageError> {
    if !matches!(who, RUSAGE_SELF | RUSAGE_CHILDREN | RUSAGE_THREAD) {
        return Err(RusageError::InvalidWho);
    }
    mem.write(usage, &Rusage::ZERO.to_guest_bytes())
        .map_err(RusageError::Fault)
}

#[cfg(test)]
mod tests {
    use super::{getrusage, uname, RusageError, GUEST_UTSNAME};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::{Rusage, Utsname};

    #[test]
    fn uname_writes_the_guest_identity() {
        let mut buf = [0xffu8; Utsname::SIZE];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        uname(&mut mem, 0x1000).expect("write ok");
        let out = mem.read(0x1000, Utsname::SIZE).unwrap();
        assert_eq!(out, &GUEST_UTSNAME.to_guest_bytes()[..]);
        // sysname reads back as the NUL-terminated "Linux".
        assert_eq!(&out[0..5], b"Linux");
        assert_eq!(out[5], 0);
        // machine (field 4) reads back as "x86_64".
        let m = 4 * Utsname::FIELD;
        assert_eq!(&out[m..m + 6], b"x86_64");
    }

    #[test]
    fn uname_faults_on_an_unwritable_pointer() {
        // A region too small for the struct: the write must fault, not panic.
        let mut buf = [0u8; 8];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert!(uname(&mut mem, 0x1000).is_err());
    }

    #[test]
    fn getrusage_writes_an_all_zero_rusage_for_self() {
        const RUSAGE_SELF: i32 = 0;
        let mut buf = [0xffu8; Rusage::SIZE];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        getrusage(&mut mem, RUSAGE_SELF, 0x1000).expect("write ok");
        let out = mem.read(0x1000, Rusage::SIZE).unwrap();
        assert!(out.iter().all(|&b| b == 0));
    }

    #[test]
    fn getrusage_rejects_an_unknown_who() {
        let mut buf = [0u8; Rusage::SIZE];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // who = 5 is not SELF/CHILDREN/THREAD -> EINVAL, nothing written.
        assert!(matches!(
            getrusage(&mut mem, 5, 0x1000),
            Err(RusageError::InvalidWho)
        ));
    }

    #[test]
    fn getrusage_faults_on_an_unwritable_pointer() {
        let mut buf = [0u8; 8]; // too small for the struct
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert!(matches!(
            getrusage(&mut mem, 0, 0x1000),
            Err(RusageError::Fault(_))
        ));
    }
}
