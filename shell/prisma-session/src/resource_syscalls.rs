//! Resource-limit syscalls (`getrlimit`/`setrlimit`/`prlimit64`).
//!
//! The session enforces fixed per-resource limits — it does not change a guest's
//! soft limit — so these report a constant [`Rlimit`] per resource and accept
//! (but do not store) a new limit. Pointers are checked through a
//! [`GuestRegion`], so a bad address is a guest `EFAULT`.

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_structs::Rlimit;

/// Resource selectors (`<asm-generic/resource.h>`, identical on x86-64 Linux).
const RLIMIT_STACK: u32 = 3;
const RLIMIT_NOFILE: u32 = 7;
/// One past the last resource id; `resource >= RLIMIT_NLIMITS` is `EINVAL`.
const RLIMIT_NLIMITS: u32 = 16;

/// The initial-thread stack limit reported for `RLIMIT_STACK` (8 MiB soft, the
/// conventional Linux default; hard is unlimited).
const STACK_CUR: u64 = 8 * 1024 * 1024;
/// The open-file limits reported for `RLIMIT_NOFILE` (soft 1024 / hard 4096, the
/// conventional Linux defaults).
const NOFILE_CUR: u64 = 1024;
const NOFILE_MAX: u64 = 4096;

/// The fixed limit the session reports for `resource`. Unmodelled resources are
/// unlimited — the honest value when nothing constrains them here.
fn limit_for(resource: u32) -> Rlimit {
    match resource {
        RLIMIT_NOFILE => Rlimit {
            cur: NOFILE_CUR,
            max: NOFILE_MAX,
        },
        RLIMIT_STACK => Rlimit {
            cur: STACK_CUR,
            max: Rlimit::INFINITY,
        },
        _ => Rlimit {
            cur: Rlimit::INFINITY,
            max: Rlimit::INFINITY,
        },
    }
}

/// Why a resource-limit syscall failed (each maps to a guest errno at routing
/// time).
#[derive(Debug)]
pub enum RlimitError {
    /// `resource` is not a known limit id — guest `EINVAL`.
    InvalidResource,
    /// A limit pointer is not accessible guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `getrlimit(resource, rlim)`: write the session's fixed limit for `resource`
/// to the guest `rlimit` at `rlim`.
///
/// # Errors
/// [`RlimitError::InvalidResource`] for an unknown resource,
/// [`RlimitError::Fault`] if `rlim` is not writable guest memory.
pub fn getrlimit(mem: &mut GuestRegion, resource: u32, rlim: u64) -> Result<(), RlimitError> {
    if resource >= RLIMIT_NLIMITS {
        return Err(RlimitError::InvalidResource);
    }
    mem.write(rlim, &limit_for(resource).to_guest_bytes())
        .map_err(RlimitError::Fault)
}

/// `prlimit64(resource, new_limit, old_limit)`: write the current limit to
/// `old_limit` (when non-null) and accept a `new_limit` (when non-null) without
/// storing it — the session's limits are fixed.
///
/// A non-null `new_limit` is still range-checked and parsed, so a malformed
/// pointer faults rather than being silently ignored.
///
/// # Errors
/// [`RlimitError::InvalidResource`] for an unknown resource,
/// [`RlimitError::Fault`] if either non-null pointer is not accessible guest
/// memory.
pub fn prlimit64(
    mem: &mut GuestRegion,
    resource: u32,
    new_limit: u64,
    old_limit: u64,
) -> Result<(), RlimitError> {
    if resource >= RLIMIT_NLIMITS {
        return Err(RlimitError::InvalidResource);
    }
    // Report the current (fixed) limit first, before honouring any new one.
    if old_limit != 0 {
        mem.write(old_limit, &limit_for(resource).to_guest_bytes())
            .map_err(RlimitError::Fault)?;
    }
    // Validate a supplied new limit so a bad pointer faults; the value is not
    // stored (limits are fixed in this model).
    if new_limit != 0 {
        let bytes = mem
            .read(new_limit, Rlimit::SIZE)
            .map_err(RlimitError::Fault)?;
        Rlimit::from_guest_bytes(bytes).ok_or(RlimitError::Fault(RangeError::Unmapped))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{getrlimit, prlimit64, RlimitError, NOFILE_CUR, NOFILE_MAX};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::Rlimit;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn getrlimit_reports_the_nofile_limits() {
        const RLIMIT_NOFILE: u32 = 7;
        let mut buf = [0u8; Rlimit::SIZE];
        let mut mem = region(&mut buf);
        getrlimit(&mut mem, RLIMIT_NOFILE, 0x1000).expect("write ok");
        let r = Rlimit::from_guest_bytes(mem.read(0x1000, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((r.cur, r.max), (NOFILE_CUR, NOFILE_MAX));
    }

    #[test]
    fn getrlimit_reports_infinity_for_an_unmodelled_resource() {
        const RLIMIT_CPU: u32 = 0;
        let mut buf = [0u8; Rlimit::SIZE];
        let mut mem = region(&mut buf);
        getrlimit(&mut mem, RLIMIT_CPU, 0x1000).expect("write ok");
        let r = Rlimit::from_guest_bytes(mem.read(0x1000, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((r.cur, r.max), (Rlimit::INFINITY, Rlimit::INFINITY));
    }

    #[test]
    fn getrlimit_rejects_an_unknown_resource() {
        let mut buf = [0u8; Rlimit::SIZE];
        let mut mem = region(&mut buf);
        assert!(matches!(
            getrlimit(&mut mem, 16, 0x1000),
            Err(RlimitError::InvalidResource)
        ));
    }

    #[test]
    fn prlimit64_writes_old_and_accepts_new_without_storing() {
        const RLIMIT_NOFILE: u32 = 7;
        let mut buf = [0u8; 2 * Rlimit::SIZE];
        let mut mem = region(&mut buf);
        // Put a would-be new limit at 0x1010; request old at 0x1000.
        let newr = Rlimit { cur: 1, max: 2 };
        mem.write(0x1010, &newr.to_guest_bytes()).unwrap();
        prlimit64(&mut mem, RLIMIT_NOFILE, 0x1010, 0x1000).expect("ok");
        // old_limit got the fixed NOFILE limit, not the new one.
        let old = Rlimit::from_guest_bytes(mem.read(0x1000, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((old.cur, old.max), (NOFILE_CUR, NOFILE_MAX));
        // A second read still reports the fixed limit (the new one was not stored).
        let mut buf2 = [0u8; Rlimit::SIZE];
        let mut mem2 = region(&mut buf2);
        getrlimit(&mut mem2, RLIMIT_NOFILE, 0x1000).unwrap();
        let again = Rlimit::from_guest_bytes(mem2.read(0x1000, Rlimit::SIZE).unwrap()).unwrap();
        assert_eq!((again.cur, again.max), (NOFILE_CUR, NOFILE_MAX));
    }

    #[test]
    fn prlimit64_faults_on_an_unwritable_old_pointer() {
        let mut buf = [0u8; 8]; // too small for an Rlimit
        let mut mem = region(&mut buf);
        assert!(matches!(
            prlimit64(&mut mem, 7, 0, 0x1000),
            Err(RlimitError::Fault(_))
        ));
    }
}
