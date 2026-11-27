//! Linux thread attributes carried across syscall dispatch for a CLONE_THREAD guest.

use prisma_runtime::guest_signal::ThreadSignals;
use prisma_runtime::guest_structs::SigAltStack;

use crate::process_syscalls::COMM_LEN;
use crate::syscall_dispatch::SyscallContext;

const DISABLED_ALTSTACK: SigAltStack = SigAltStack {
    sp: 0,
    flags: 2, // Linux SS_DISABLE
    size: 0,
};

#[derive(Debug)]
pub(crate) struct ThreadAttributes {
    signals: ThreadSignals,
    altstack: SigAltStack,
    comm: [u8; COMM_LEN],
}

impl ThreadAttributes {
    pub(crate) fn capture_leader(ctx: &mut SyscallContext) -> Self {
        let mut leader = Self {
            signals: ThreadSignals::default(),
            altstack: DISABLED_ALTSTACK,
            comm: [0; COMM_LEN],
        };
        leader.swap_with(ctx);
        leader
    }

    pub(crate) const fn for_child(&self) -> Self {
        Self {
            signals: self.signals.for_child(),
            altstack: DISABLED_ALTSTACK,
            comm: self.comm,
        }
    }

    pub(crate) fn swap_with(&mut self, ctx: &mut SyscallContext) {
        ctx.signals.swap_thread(&mut self.signals);
        std::mem::swap(&mut ctx.altstack, &mut self.altstack);
        std::mem::swap(&mut ctx.comm, &mut self.comm);
    }
}

#[cfg(test)]
mod tests {
    use super::{ThreadAttributes, DISABLED_ALTSTACK};
    use crate::syscall_dispatch::SyscallContext;
    use prisma_runtime::guest_signal::{SigAction, SigprocmaskHow, Sigset};
    use prisma_runtime::guest_structs::SigAltStack;

    #[test]
    fn clone_inherits_mask_and_name_and_shares_only_signal_actions() {
        let mut ctx = SyscallContext::new();
        let old_mask = Sigset::from_bits(1 << 11);
        ctx.signals.set_mask(SigprocmaskHow::SetMask, old_mask);
        ctx.signals.raise(12);
        ctx.signals.raise(15);
        ctx.comm[..4].copy_from_slice(b"main");
        let parent_stack = SigAltStack {
            sp: 0x4000,
            size: 8192,
            flags: 0,
        };
        ctx.altstack = parent_stack;
        let mut parent = ThreadAttributes::capture_leader(&mut ctx);
        let mut child = parent.for_child();
        child.swap_with(&mut ctx);

        assert_eq!(ctx.signals.pending_count(), 0);
        assert_eq!(ctx.signals.blocked(), old_mask);
        assert_eq!(ctx.altstack, DISABLED_ALTSTACK);
        assert_eq!(&ctx.comm[..4], b"main");

        ctx.comm[..4].copy_from_slice(b"work");
        ctx.signals
            .set_mask(SigprocmaskHow::SetMask, Sigset::empty());
        ctx.signals.raise(10);
        let action = SigAction {
            handler: 0x2000,
            ..SigAction::default()
        };
        assert!(ctx.signals.set_action(15, action));
        child.swap_with(&mut ctx);
        parent.swap_with(&mut ctx);
        assert_eq!(ctx.signals.blocked(), old_mask);
        assert_eq!(ctx.signals.pending_count(), 2);
        assert_eq!(ctx.signals.next_deliverable(), Some(15));
        assert_eq!(ctx.signals.next_deliverable(), None);
        assert_eq!(ctx.signals.action(15), Some(action));
        assert_eq!(ctx.altstack, parent_stack);
        assert_eq!(&ctx.comm[..4], b"main");

        parent.swap_with(&mut ctx);
        child.swap_with(&mut ctx);
        assert_eq!(ctx.signals.next_deliverable(), Some(10));
        assert_eq!(ctx.signals.pending_count(), 0);
        assert_eq!(&ctx.comm[..4], b"work");
    }
}
