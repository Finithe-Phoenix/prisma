//! Linux x86-64 futexes scoped to one guest address space.
//!
//! The driver holds the same process mutex while executing guest code,
//! comparing the guest word, changing this queue, and testing the wake
//! predicate before parking on its condition variable.

use crate::syscall_dispatch::errno::{EAGAIN, EFAULT, EINVAL, ENOSYS};
use prisma_orchestrator::backed_address_space::BackedAddressSpace;
use std::collections::VecDeque;

const WAIT: u32 = 0;
const WAKE: u32 = 1;
const PRIVATE: u32 = 128;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum FutexAction {
    Return(i64),
    Park,
}

struct Waiter {
    tid: u32,
    address: u64,
    woken: bool,
}

/// At most one entry per guest thread. Only waiters present when WAKE runs
/// receive a grant, even if another thread arrives before they resume.
#[derive(Default)]
pub(crate) struct FutexQueues {
    waiters: Vec<Waiter>,
}

impl FutexQueues {
    #[allow(clippy::cast_possible_truncation)] // Linux takes 32-bit op and val.
    pub(crate) fn syscall(
        &mut self,
        tid: u32,
        memory: &BackedAddressSpace,
        args: [u64; 6],
    ) -> FutexAction {
        let [address, op, value, timeout, _, _] = args;
        let command = (op as u32) & !PRIVATE;
        if !matches!(command, WAIT | WAKE) || (command == WAIT && timeout != 0) {
            return FutexAction::Return(ENOSYS);
        }
        if address & 3 != 0 {
            return FutexAction::Return(EINVAL);
        }
        let Ok(word) = memory.read(address, 4) else {
            return FutexAction::Return(EFAULT);
        };
        let value = value as u32;
        if command == WAKE {
            return FutexAction::Return(i64::from(self.wake(address, value)));
        }
        if word != value.to_le_bytes() {
            return FutexAction::Return(EAGAIN);
        }
        if self.waiters.iter().any(|waiter| waiter.tid == tid) {
            return FutexAction::Return(EINVAL);
        }
        self.waiters.push(Waiter {
            tid,
            address,
            woken: false,
        });
        FutexAction::Park
    }

    /// Grant at most `limit` wakes, including for a successful clear-child-TID
    /// write on thread exit. The caller notifies the process condition variable.
    pub(crate) fn wake(&mut self, address: u64, limit: u32) -> u32 {
        let mut granted = 0;
        for waiter in &mut self.waiters {
            if granted == limit {
                break;
            }
            if waiter.address == address && !waiter.woken {
                waiter.woken = true;
                granted += 1;
            }
        }
        granted
    }

    /// Move all wake grantees onto the runnable queue in arrival order. The
    /// caller still holds the process mutex used to grant those wakes.
    pub(crate) fn enqueue_woken(&mut self, ready: &mut VecDeque<u32>) {
        self.waiters.retain(|waiter| {
            if waiter.woken {
                ready.push_back(waiter.tid);
            }
            !waiter.woken
        });
    }

    pub(crate) fn remove(&mut self, tid: u32) {
        self.waiters.retain(|waiter| waiter.tid != tid);
    }

    pub(crate) fn parked_count(&self) -> usize {
        self.waiters.iter().filter(|waiter| !waiter.woken).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_orchestrator::address_space::Protection;

    const ADDRESS: u64 = 0x4000;
    const WORD: u32 = 0x0302_0100;

    fn memory() -> BackedAddressSpace {
        let mut mem = BackedAddressSpace::new();
        mem.map_with_bytes(ADDRESS, &[0, 1, 2, 3, 0, 1, 2, 3], Protection::ReadWrite)
            .unwrap();
        mem
    }

    fn args(address: u64, op: u32, val: u32) -> [u64; 6] {
        [address, u64::from(op), u64::from(val), 0, 0, 0]
    }

    #[test]
    fn checks_guest_word_flags_and_timeout_before_parking() {
        let mut queues = FutexQueues::default();
        let mem = memory();
        for flag in [0, PRIVATE] {
            for cmd in [WAIT, WAKE] {
                assert_eq!(
                    queues.syscall(1, &mem, args(ADDRESS + 1, cmd | flag, WORD)),
                    FutexAction::Return(EINVAL)
                );
                assert_eq!(
                    queues.syscall(1, &mem, args(ADDRESS + 8, cmd | flag, WORD)),
                    FutexAction::Return(EFAULT)
                );
            }
            assert_eq!(
                queues.syscall(1, &mem, args(ADDRESS, WAIT | flag, 0)),
                FutexAction::Return(EAGAIN)
            );
            assert_eq!(
                queues.syscall(1, &mem, args(ADDRESS, WAKE | flag, 2)),
                FutexAction::Return(0)
            );
        }
        for op in [3, 0x100, PRIVATE | 9] {
            assert_eq!(
                queues.syscall(1, &mem, args(0, op, 0)),
                FutexAction::Return(ENOSYS)
            );
        }
        let mut timed = args(ADDRESS, WAIT, WORD);
        timed[3] = u64::MAX;
        assert_eq!(queues.syscall(1, &mem, timed), FutexAction::Return(ENOSYS));
        assert_eq!(queues.parked_count(), 0);
    }

    #[test]
    fn bounded_wakes_belong_to_existing_waiters_until_consumed() {
        let mut queues = FutexQueues::default();
        let mem = memory();
        let wait = args(ADDRESS, WAIT | PRIVATE, WORD);
        for tid in 1..=3 {
            assert_eq!(queues.syscall(tid, &mem, wait), FutexAction::Park);
        }
        assert_eq!(queues.syscall(1, &mem, wait), FutexAction::Return(EINVAL));
        assert_eq!(queues.wake(ADDRESS, 0), 0);
        assert_eq!(
            queues.syscall(8, &mem, args(ADDRESS, WAKE, 2)),
            FutexAction::Return(2)
        );
        assert_eq!(queues.parked_count(), 1);
        assert_eq!(queues.syscall(4, &mem, wait), FutexAction::Park);
        assert_eq!(
            queues.syscall(5, &mem, args(ADDRESS + 4, WAIT, WORD)),
            FutexAction::Park
        );
        let mut ready = VecDeque::from([9]);
        queues.enqueue_woken(&mut ready);
        assert_eq!(ready, [9, 1, 2]);
        queues.enqueue_woken(&mut ready);
        assert_eq!(ready, [9, 1, 2]);

        queues.remove(3);
        assert_eq!(queues.wake(ADDRESS, u32::MAX), 1);
        queues.enqueue_woken(&mut ready);
        assert_eq!(ready, [9, 1, 2, 4]);
        assert_eq!(queues.wake(ADDRESS, u32::MAX), 0);
        assert_eq!(queues.parked_count(), 1);
        queues.remove(5);
        assert_eq!(queues.parked_count(), 0);
    }
}
