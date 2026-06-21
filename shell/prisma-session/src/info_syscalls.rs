//! System-information syscalls.
//!
//! Handlers that report fixed facts about the (emulated) system to the guest.
//! They write a packed struct through a [`GuestRegion`], so a bad pointer is a
//! guest `EFAULT` rather than an out-of-bounds host write.

use std::time::Instant;

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_mem::GuestMem;
use prisma_runtime::guest_structs::{Rusage, Sysinfo, Utsname};

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
pub fn uname(mem: &mut impl GuestMem, buf: u64) -> Result<(), RangeError> {
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
pub fn getrusage(mem: &mut impl GuestMem, who: i32, usage: u64) -> Result<(), RusageError> {
    if !matches!(who, RUSAGE_SELF | RUSAGE_CHILDREN | RUSAGE_THREAD) {
        return Err(RusageError::InvalidWho);
    }
    mem.write(usage, &Rusage::ZERO.to_guest_bytes())
        .map_err(RusageError::Fault)
}

/// `getcpu(cpu, node, tcache)`: report the CPU and NUMA node the caller is
/// running on. The session presents a single NUMA node, so it always reports
/// CPU 0 / node 0; each out pointer is optional (skipped when null). The legacy
/// `tcache` argument is unused on modern Linux and ignored here.
///
/// # Errors
/// [`RangeError`] if a non-null `cpu`/`node` pointer is not writable guest
/// memory (guest `EFAULT`).
pub fn getcpu(mem: &mut impl GuestMem, cpu: u64, node: u64) -> Result<(), RangeError> {
    if cpu != 0 {
        mem.write(cpu, &0u32.to_le_bytes())?;
    }
    if node != 0 {
        mem.write(node, &0u32.to_le_bytes())?;
    }
    Ok(())
}

/// Synthetic total RAM reported by `sysinfo` (2 GiB). The session does not model
/// real guest memory accounting, so it reports fixed, plausible figures rather
/// than zero (which a guest could misread as "no memory").
const SYNTH_TOTALRAM: u64 = 2 * 1024 * 1024 * 1024;
/// Synthetic free RAM reported by `sysinfo` (1 GiB).
const SYNTH_FREERAM: u64 = 1024 * 1024 * 1024;

/// `sysinfo(buf)`: write system statistics to the guest `struct sysinfo` at
/// `buf`. `uptime` is the real elapsed time since `monotonic_start`; the memory
/// figures are fixed synthetic values (`mem_unit` = 1 byte) and load averages
/// are zero — real accounting is not modelled.
///
/// # Errors
/// [`RangeError`] if `buf` is not writable guest memory for [`Sysinfo::SIZE`]
/// bytes (guest `EFAULT`).
pub fn sysinfo(
    mem: &mut impl GuestMem,
    buf: u64,
    monotonic_start: Instant,
) -> Result<(), RangeError> {
    let info = Sysinfo {
        uptime: i64::try_from(monotonic_start.elapsed().as_secs()).unwrap_or(i64::MAX),
        loads: [0; 3],
        totalram: SYNTH_TOTALRAM,
        freeram: SYNTH_FREERAM,
        sharedram: 0,
        bufferram: 0,
        totalswap: 0,
        freeswap: 0,
        procs: 1,
        totalhigh: 0,
        freehigh: 0,
        mem_unit: 1,
    };
    mem.write(buf, &info.to_guest_bytes())
}

#[cfg(test)]
mod tests {
    use super::{getcpu, getrusage, sysinfo, uname, RusageError, GUEST_UTSNAME};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::{Rusage, Sysinfo, Utsname};
    use std::time::Instant;

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

    #[test]
    fn getcpu_reports_cpu_zero_node_zero() {
        let mut buf = [0xffu8; 8];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // cpu at 0x1000, node at 0x1004.
        getcpu(&mut mem, 0x1000, 0x1004).expect("write ok");
        assert_eq!(mem.read(0x1000, 4).unwrap(), &0u32.to_le_bytes());
        assert_eq!(mem.read(0x1004, 4).unwrap(), &0u32.to_le_bytes());
    }

    #[test]
    fn getcpu_skips_null_pointers() {
        let mut buf = [0xffu8; 4];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // Both null: nothing written, still succeeds.
        getcpu(&mut mem, 0, 0).expect("ok");
        assert_eq!(mem.read(0x1000, 4).unwrap(), &[0xff; 4]);
        // Only node requested: cpu pointer (null) is skipped.
        getcpu(&mut mem, 0, 0x1000).expect("ok");
        assert_eq!(mem.read(0x1000, 4).unwrap(), &0u32.to_le_bytes());
    }

    #[test]
    fn getcpu_faults_on_an_unwritable_pointer() {
        let mut buf = [0u8; 4];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        // 0x9000 is outside the mapped region.
        assert!(getcpu(&mut mem, 0x9000, 0).is_err());
    }

    #[test]
    fn sysinfo_writes_synthetic_memory_and_a_unit_of_one() {
        let mut buf = [0xffu8; Sysinfo::SIZE];
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        sysinfo(&mut mem, 0x1000, Instant::now()).expect("write ok");
        let out = mem.read(0x1000, Sysinfo::SIZE).unwrap();
        // totalram (offset 32) is the synthetic 2 GiB; mem_unit (104) is 1.
        assert_eq!(&out[32..40], &(2u64 * 1024 * 1024 * 1024).to_le_bytes());
        assert_eq!(&out[104..108], &1u32.to_le_bytes());
        // procs (offset 80) is 1.
        assert_eq!(&out[80..82], &1u16.to_le_bytes());
    }

    #[test]
    fn sysinfo_faults_on_an_unwritable_pointer() {
        let mut buf = [0u8; 8]; // too small for the struct
        let mut mem = GuestRegion::new(0x1000, Protection::ReadWrite, &mut buf);
        assert!(sysinfo(&mut mem, 0x1000, Instant::now()).is_err());
    }
}
