//! Scheduler-parameter syscalls (`sched_getparam`/`sched_setparam`).
//!
//! The session presents a single scheduling policy, `SCHED_OTHER` (the normal
//! time-sharing policy), whose static priority is always 0. So `sched_getparam`
//! reports priority 0, and `sched_setparam` accepts only priority 0 — any other
//! value is `EINVAL`, exactly as the kernel rejects a non-zero static priority
//! for `SCHED_OTHER`. Pointers are checked through a [`GuestRegion`].

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_mem::GuestMem;
use prisma_runtime::guest_structs::SchedParam;

/// `SCHED_OTHER` — the only scheduling policy the session models. Its valid
/// static priority is 0.
pub const SCHED_OTHER: i64 = 0;

/// Why a scheduler-parameter syscall failed (each maps to a guest errno at
/// routing time).
#[derive(Debug)]
pub enum SchedError {
    /// A requested priority is not valid for the policy — guest `EINVAL`.
    InvalidParam,
    /// The parameter pointer is not accessible guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `sched_getparam(pid, param)`: write the static scheduling priority for `pid`
/// to the guest `sched_param` at `param`. Under the single `SCHED_OTHER` policy
/// this is always 0.
///
/// # Errors
/// [`RangeError`] if `param` is not writable guest memory (guest `EFAULT`).
pub fn sched_getparam(mem: &mut impl GuestMem, param: u64) -> Result<(), RangeError> {
    mem.write(param, &SchedParam { priority: 0 }.to_guest_bytes())
}

/// `sched_setparam(pid, param)`: set the static scheduling priority for `pid`
/// from the guest `sched_param` at `param`. Only priority 0 is valid for
/// `SCHED_OTHER`; the value is otherwise rejected (and not stored — the policy
/// is fixed).
///
/// # Errors
/// [`SchedError::Fault`] if `param` is not readable guest memory,
/// [`SchedError::InvalidParam`] if the requested priority is not 0.
pub fn sched_setparam(mem: &impl GuestMem, param: u64) -> Result<(), SchedError> {
    let bytes = mem
        .read(param, SchedParam::SIZE)
        .map_err(SchedError::Fault)?;
    let p = SchedParam::from_guest_bytes(bytes).ok_or(SchedError::Fault(RangeError::Unmapped))?;
    if p.priority != 0 {
        return Err(SchedError::InvalidParam);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sched_getparam, sched_setparam, SchedError};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::guest_structs::SchedParam;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn getparam_reports_priority_zero() {
        let mut buf = [0xffu8; SchedParam::SIZE];
        let mut mem = region(&mut buf);
        sched_getparam(&mut mem, 0x1000).expect("write ok");
        let p = SchedParam::from_guest_bytes(mem.read(0x1000, SchedParam::SIZE).unwrap()).unwrap();
        assert_eq!(p.priority, 0);
    }

    #[test]
    fn setparam_accepts_zero_and_rejects_nonzero() {
        let mut buf = [0u8; SchedParam::SIZE];
        // priority 0 is accepted.
        {
            let mut mem = region(&mut buf);
            mem.write(0x1000, &SchedParam { priority: 0 }.to_guest_bytes())
                .unwrap();
            assert!(sched_setparam(&mem, 0x1000).is_ok());
        }
        // any non-zero priority is EINVAL under SCHED_OTHER.
        let mut mem = region(&mut buf);
        mem.write(0x1000, &SchedParam { priority: 5 }.to_guest_bytes())
            .unwrap();
        assert!(matches!(
            sched_setparam(&mem, 0x1000),
            Err(SchedError::InvalidParam)
        ));
    }

    #[test]
    fn syscalls_fault_on_a_bad_pointer() {
        let mut buf = [0u8; 2]; // too small for a sched_param
        let mut mem = region(&mut buf);
        assert!(sched_getparam(&mut mem, 0x1000).is_err());
        assert!(matches!(
            sched_setparam(&mem, 0x1000),
            Err(SchedError::Fault(_))
        ));
    }
}
