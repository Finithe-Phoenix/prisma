//! System-information syscalls.
//!
//! Handlers that report fixed facts about the (emulated) system to the guest.
//! They write a packed struct through a [`GuestRegion`], so a bad pointer is a
//! guest `EFAULT` rather than an out-of-bounds host write.

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_structs::Utsname;

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

#[cfg(test)]
mod tests {
    use super::{uname, GUEST_UTSNAME};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::Utsname;

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
}
