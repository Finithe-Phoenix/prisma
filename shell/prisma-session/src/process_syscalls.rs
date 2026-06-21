//! Process-control syscalls (`prctl`).
//!
//! Only the small, commonly-called subset is modelled. The thread name
//! (`PR_SET_NAME`/`PR_GET_NAME`) is the one piece of state, owned by the caller
//! (the session's `SyscallContext`) and threaded in as `&mut [u8; COMM_LEN]` so
//! this handler stays decoupled from the dispatcher. Pointers are checked through
//! a [`GuestRegion`].

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_mem::GuestMem;

/// `prctl` options we model.
const PR_SET_DUMPABLE: i32 = 4;
const PR_GET_DUMPABLE: i32 = 3;
const PR_SET_NAME: i32 = 15;
const PR_GET_NAME: i32 = 16;

/// Length of the thread name buffer (`TASK_COMM_LEN`): 15 chars + NUL.
pub const COMM_LEN: usize = 16;

/// Why `prctl` failed (each maps to a guest errno at routing time).
#[derive(Debug)]
pub enum PrctlError {
    /// The `option` is one we do not model — guest `EINVAL`.
    Unsupported,
    /// A name pointer is not accessible guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// `prctl(option, arg2, ...)`: the modelled subset.
///
/// - `PR_SET_NAME` copies a `COMM_LEN`-byte name from `arg2` into `comm`
///   (forcing a trailing NUL, as the kernel does).
/// - `PR_GET_NAME` writes `comm` back to `arg2`.
/// - `PR_GET_DUMPABLE` reports 1 (the process is dumpable); `PR_SET_DUMPABLE`
///   is accepted (not modelled) and returns 0.
/// - any other option is `EINVAL`.
///
/// # Errors
/// [`PrctlError::Unsupported`] for an unmodelled option, [`PrctlError::Fault`]
/// if a name pointer is not accessible guest memory.
pub fn prctl(
    comm: &mut [u8; COMM_LEN],
    mem: &mut impl GuestMem,
    option: i32,
    arg2: u64,
) -> Result<i64, PrctlError> {
    match option {
        PR_SET_NAME => {
            let bytes = mem.read(arg2, COMM_LEN).map_err(PrctlError::Fault)?;
            comm.copy_from_slice(bytes);
            comm[COMM_LEN - 1] = 0; // the name is always NUL-terminated
            Ok(0)
        }
        PR_GET_NAME => {
            mem.write(arg2, comm).map_err(PrctlError::Fault)?;
            Ok(0)
        }
        PR_GET_DUMPABLE => Ok(1),
        PR_SET_DUMPABLE => Ok(0),
        _ => Err(PrctlError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::{prctl, PrctlError, COMM_LEN};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;

    const PR_SET_NAME: i32 = 15;
    const PR_GET_NAME: i32 = 16;
    const PR_GET_DUMPABLE: i32 = 3;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn set_name_then_get_name_round_trips() {
        let mut comm = [0u8; COMM_LEN];
        let mut buf = [0u8; 32];
        // Lay a NUL-terminated name in guest memory and PR_SET_NAME it.
        let name = b"worker-1\0";
        let mut mem = region(&mut buf);
        mem.write(0x1000, name).unwrap();
        assert_eq!(prctl(&mut comm, &mut mem, PR_SET_NAME, 0x1000).unwrap(), 0);
        assert_eq!(&comm[0..8], b"worker-1");
        assert_eq!(comm[COMM_LEN - 1], 0);
        // PR_GET_NAME writes the stored name back to a different address.
        assert_eq!(prctl(&mut comm, &mut mem, PR_GET_NAME, 0x1010).unwrap(), 0);
        assert_eq!(&mem.read(0x1010, 8).unwrap(), b"worker-1");
    }

    #[test]
    fn get_dumpable_reports_one() {
        let mut comm = [0u8; COMM_LEN];
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        assert_eq!(prctl(&mut comm, &mut mem, PR_GET_DUMPABLE, 0).unwrap(), 1);
    }

    #[test]
    fn unmodelled_option_is_unsupported() {
        let mut comm = [0u8; COMM_LEN];
        let mut buf = [0u8; 8];
        let mut mem = region(&mut buf);
        // PR_SET_PDEATHSIG (1) is not modelled.
        assert!(matches!(
            prctl(&mut comm, &mut mem, 1, 0),
            Err(PrctlError::Unsupported)
        ));
    }

    #[test]
    fn set_name_faults_on_an_unreadable_pointer() {
        let mut comm = [0u8; COMM_LEN];
        let mut buf = [0u8; 8]; // too small for COMM_LEN
        let mut mem = region(&mut buf);
        assert!(matches!(
            prctl(&mut comm, &mut mem, PR_SET_NAME, 0x1000),
            Err(PrctlError::Fault(_))
        ));
    }
}
