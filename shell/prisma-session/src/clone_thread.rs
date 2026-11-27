//! Prepare an x86-64 Linux pthread-style clone before publishing either TID.

use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use prisma_runtime::executor::{gpr, CpuStateFrame, EXIT_NORMAL};

use crate::syscall_dispatch::errno;

const CLONE_VM: u64 = 0x100;
const CLONE_FS: u64 = 0x200;
const CLONE_FILES: u64 = 0x400;
const CLONE_SIGHAND: u64 = 0x800;
const CLONE_THREAD: u64 = 0x1_0000;
const CLONE_SYSVSEM: u64 = 0x4_0000;
const CLONE_SETTLS: u64 = 0x8_0000;
const CLONE_PARENT_SETTID: u64 = 0x10_0000;
const CLONE_CHILD_CLEARTID: u64 = 0x20_0000;
const CLONE_CHILD_SETTID: u64 = 0x100_0000;
const THREAD_FLAGS: u64 = CLONE_VM | CLONE_SIGHAND | CLONE_THREAD;
const SHARED_RESOURCES: u64 = CLONE_FS | CLONE_FILES;
const SUPPORTED_FLAGS: u64 = THREAD_FLAGS
    | SHARED_RESOURCES
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_CHILD_SETTID;

pub(crate) struct PreparedClone {
    pub frame: CpuStateFrame,
    pub tid: u32,
    pub next_pc: u64,
    pub clear_child_tid: u64,
    parent_tid_out: Option<u64>,
    child_tid_out: Option<u64>,
}

impl PreparedClone {
    fn check_outputs(&self, mem: &BackedAddressSpace) -> Result<(), i64> {
        for addr in [self.parent_tid_out, self.child_tid_out]
            .into_iter()
            .flatten()
        {
            if addr == 0 || mem.ensure_writable(addr, size_of::<u32>()).is_err() {
                return Err(errno::EFAULT);
            }
        }
        Ok(())
    }

    /// Call with the child created but parked. Recheck both mappings under the
    /// same exclusive memory borrow that covers both writes.
    pub(crate) fn publish_tid(&self, mem: &mut BackedAddressSpace) -> Result<(), i64> {
        self.check_outputs(mem)?;
        for addr in [self.parent_tid_out, self.child_tid_out]
            .into_iter()
            .flatten()
        {
            mem.write(addr, &self.tid.to_le_bytes())
                .map_err(|_| errno::EFAULT)?;
        }
        Ok(())
    }
}

pub(crate) fn prepare_clone(
    parent: &CpuStateFrame,
    mem: &BackedAddressSpace,
    args: [u64; 6],
    next_pc: u64,
    child_tid: u32,
) -> Result<PreparedClone, i64> {
    let [flags, stack, parent_tid_ptr, child_tid_ptr, tls, _] = args;
    if flags & CLONE_THREAD == 0 {
        return Err(errno::ENOSYS); // this driver only creates guest threads
    }
    if flags & THREAD_FLAGS != THREAD_FLAGS || flags & 0xff != 0 {
        return Err(errno::EINVAL);
    }
    // The session owns one descriptor table and one filesystem context. A
    // clone that requests private copies cannot use this driver.
    if flags & SHARED_RESOURCES != SHARED_RESOURCES || flags & !SUPPORTED_FLAGS != 0 {
        return Err(errno::ENOSYS);
    }
    if child_tid == 0 || child_tid > i32::MAX as u32 {
        return Err(errno::EAGAIN);
    }

    let mut frame = parent.clone();
    frame.gpr[gpr::RAX] = 0;
    if stack != 0 {
        // clone hands userspace precisely this stack pointer. In particular a
        // one-past-the-top stack address is normal, and it is not rounded down.
        frame.gpr[gpr::RSP] = stack;
    }
    if flags & CLONE_SETTLS != 0 {
        frame.fs_base = tls;
    }
    frame.exit_reason = EXIT_NORMAL;
    frame.next_pc = next_pc;
    let child = PreparedClone {
        frame,
        tid: child_tid,
        next_pc,
        clear_child_tid: if flags & CLONE_CHILD_CLEARTID != 0 {
            child_tid_ptr
        } else {
            0
        },
        parent_tid_out: (flags & CLONE_PARENT_SETTID != 0).then_some(parent_tid_ptr),
        child_tid_out: (flags & CLONE_CHILD_SETTID != 0).then_some(child_tid_ptr),
    };
    child.check_outputs(mem)?;
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall_dispatch::LinuxThreadState;
    use prisma_orchestrator::address_space::Protection;

    const TID: u32 = 123;
    const PARENT_WORD: u64 = 0x1000;
    const CHILD_WORD: u64 = 0x2000;
    const FLAGS: u64 = SUPPORTED_FLAGS;

    fn space() -> BackedAddressSpace {
        let mut mem = BackedAddressSpace::new();
        for addr in [PARENT_WORD, CHILD_WORD] {
            mem.map_with_bytes(addr, &[0x55; 8], Protection::ReadWrite)
                .unwrap();
        }
        mem
    }

    #[test]
    fn clone_publishes_after_creation_and_only_patches_child_registers() {
        let mut parent = CpuStateFrame::default();
        parent.gpr[gpr::RAX] = 56;
        parent.gpr[gpr::RSP] = 0x8100;
        parent.gpr[gpr::RDI] = FLAGS;
        parent.gs_base = 999;
        parent.fs_base = 0x5000;
        parent.mem_base = 0x4000;
        parent.cf = 1;
        let mut mem = space();
        let args = [FLAGS, 0x2011, PARENT_WORD, CHILD_WORD, 0x6010, 0];
        let child = prepare_clone(&parent, &mem, args, 0x9000, TID).unwrap();
        assert_eq!(parent.gpr[gpr::RAX], 56);
        assert_eq!(parent.fs_base, 0x5000);
        assert_eq!(child.frame.gpr[gpr::RAX], 0);
        assert_eq!(child.frame.gpr[gpr::RSP], args[1]);
        assert_eq!(child.frame.gpr[gpr::RDI], FLAGS);
        assert_eq!(child.frame.fs_base, args[4]);
        assert_eq!(child.frame.gs_base, parent.gs_base);
        assert_eq!(child.frame.cf, parent.cf);
        assert_eq!(child.frame.mem_base, parent.mem_base);
        assert_eq!(child.next_pc, 0x9000);
        assert_eq!(mem.read(PARENT_WORD, 4).unwrap(), &[0x55; 4]);
        assert_eq!(mem.read(CHILD_WORD, 4).unwrap(), &[0x55; 4]);
        child.publish_tid(&mut mem).unwrap();
        assert_eq!(mem.read(PARENT_WORD, 4).unwrap(), &TID.to_le_bytes());
        assert_eq!(mem.read(CHILD_WORD, 4).unwrap(), &TID.to_le_bytes());
        let mut local = LinuxThreadState::child(child.tid, child.clear_child_tid);
        assert_eq!(local.clear_at_exit(&mut mem), Some(CHILD_WORD));
        assert_eq!(mem.read(CHILD_WORD, 4).unwrap(), &[0; 4]);
        assert_eq!(local.clear_at_exit(&mut mem), None);
    }

    #[test]
    fn failed_validation_never_writes_the_other_tid() {
        let parent = CpuStateFrame::default();
        let mut mem = space();
        for (parent_out, child_out) in [(0, CHILD_WORD), (PARENT_WORD, CHILD_WORD + 6)] {
            let args = [FLAGS, 0, parent_out, child_out, 0, 0];
            assert!(matches!(
                prepare_clone(&parent, &mem, args, 1, TID),
                Err(errno::EFAULT)
            ));
        }
        let args = [FLAGS, 0, PARENT_WORD, CHILD_WORD, 0, 0];
        let child = prepare_clone(&parent, &mem, args, 1, TID).unwrap();
        mem.unmap(CHILD_WORD).unwrap();
        assert_eq!(child.publish_tid(&mut mem), Err(errno::EFAULT));
        assert_eq!(mem.read(PARENT_WORD, 4).unwrap(), &[0x55; 4]);
        let mut local = LinuxThreadState::child(TID, CHILD_WORD);
        assert_eq!(local.clear_at_exit(&mut mem), None);
    }

    #[test]
    fn inherited_stack_tls_and_rejected_flag_combinations() {
        let mut parent = CpuStateFrame::default();
        parent.gpr[gpr::RSP] = 0x8000;
        parent.fs_base = 0x4000;
        let mem = space();
        let args = [
            THREAD_FLAGS | SHARED_RESOURCES,
            0,
            u64::MAX,
            u64::MAX,
            0x6000,
            0,
        ];
        let child = prepare_clone(&parent, &mem, args, 123, TID).unwrap();
        assert_eq!(child.frame.gpr[gpr::RSP], 0x8000);
        assert_eq!(child.frame.fs_base, 0x4000);
        assert_eq!(child.clear_child_tid, 0);
        for (flags, expected) in [
            (0, errno::ENOSYS),
            (THREAD_FLAGS | CLONE_FS, errno::ENOSYS),
            (THREAD_FLAGS | CLONE_FILES, errno::ENOSYS),
            (CLONE_THREAD | CLONE_VM, errno::EINVAL),
            (THREAD_FLAGS | 17, errno::EINVAL),
            (THREAD_FLAGS | SHARED_RESOURCES | 1 << 63, errno::ENOSYS),
        ] {
            let mut invalid = args;
            invalid[0] = flags;
            assert!(
                matches!(prepare_clone(&parent, &mem, invalid, 123, TID), Err(e) if e == expected)
            );
        }
    }
}
