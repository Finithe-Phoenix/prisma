//! Guest signal handling scaffold.

/// Minimal guest signal queue carried by the runtime.
#[derive(Debug, Default, Clone)]
pub struct GuestSignalQueue {
    /// Pending signal numbers in FIFO order.
    pending: Vec<u32>,
}

impl GuestSignalQueue {
    /// Creates an empty guest-signal queue.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Pushes a signal into the queue.
    pub fn push(&mut self, signal: u32) {
        self.pending.push(signal);
    }

    /// Removes and returns the next pending signal, if any.
    #[must_use]
    pub fn pop(&mut self) -> Option<u32> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }

    /// Remove and return the first pending signal that is NOT in `blocked`,
    /// preserving order and leaving blocked signals queued. The mask-aware
    /// counterpart of [`pop`](Self::pop) that signal delivery uses.
    #[must_use]
    pub fn pop_deliverable(&mut self, blocked: &Sigset) -> Option<u32> {
        let idx = self.pending.iter().position(|&s| !blocked.contains(s))?;
        Some(self.pending.remove(idx))
    }

    /// Clears all pending signals and returns the number cleared.
    #[must_use]
    pub fn drain(&mut self) -> usize {
        let count = self.pending.len();
        self.pending.clear();
        count
    }

    /// Current queued signal count.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether there are no queued signals.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The queued signals collected into a [`Sigset`] mask (order is lost —
    /// `rt_sigpending` reports a set, not a sequence).
    fn to_sigset(&self) -> Sigset {
        let mut set = Sigset::empty();
        for &sig in &self.pending {
            set.insert(sig);
        }
        set
    }
}

/// SIGKILL — never blockable.
const SIGKILL: u32 = 9;
/// SIGSTOP — never blockable.
const SIGSTOP: u32 = 19;

/// A guest signal mask — the kernel `sigset_t`.
///
/// On x86-64 Linux this is a single 64-bit word with one bit per signal (`sig`
/// occupies bit `sig - 1`, for `sig` in `1..=64`). Used by `rt_sigprocmask` /
/// `rt_sigaction` to track which signals are blocked.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sigset(u64);

/// How an `rt_sigprocmask` call combines its argument with the current mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigprocmaskHow {
    /// `SIG_BLOCK` (0): add the argument's signals to the blocked set.
    Block,
    /// `SIG_UNBLOCK` (1): remove the argument's signals from the blocked set.
    Unblock,
    /// `SIG_SETMASK` (2): replace the blocked set with the argument.
    SetMask,
}

impl SigprocmaskHow {
    /// Decode the raw `how` argument, or `None` (guest `EINVAL`) if unknown.
    #[must_use]
    pub const fn from_raw(how: i32) -> Option<Self> {
        match how {
            0 => Some(Self::Block),
            1 => Some(Self::Unblock),
            2 => Some(Self::SetMask),
            _ => None,
        }
    }
}

impl Sigset {
    /// On-wire size of the kernel `sigset_t` in guest memory.
    pub const SIZE: usize = 8;

    /// The empty mask (nothing blocked).
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Construct from a raw 64-bit bitmask.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw 64-bit bitmask.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// The bit for `sig`, or `None` if `sig` is outside `1..=64`.
    const fn bit(sig: u32) -> Option<u64> {
        if matches!(sig, 1..=64) {
            Some(1u64 << (sig - 1))
        } else {
            None
        }
    }

    /// Whether `sig` is in the set.
    #[must_use]
    pub const fn contains(self, sig: u32) -> bool {
        match Self::bit(sig) {
            Some(b) => self.0 & b != 0,
            None => false,
        }
    }

    /// Add `sig`; returns `false` (no-op) if `sig` is out of range.
    pub fn insert(&mut self, sig: u32) -> bool {
        match Self::bit(sig) {
            Some(b) => {
                self.0 |= b;
                true
            }
            None => false,
        }
    }

    /// Remove `sig`; returns `false` (no-op) if `sig` is out of range.
    pub fn remove(&mut self, sig: u32) -> bool {
        match Self::bit(sig) {
            Some(b) => {
                self.0 &= !b;
                true
            }
            None => false,
        }
    }

    /// Decode a `sigset_t` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self(u64::from_le_bytes(raw.try_into().ok()?)))
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        self.0.to_le_bytes()
    }

    /// Apply an `rt_sigprocmask` operation against `arg`. SIGKILL and SIGSTOP
    /// can never be blocked, so they are masked out of the resulting blocked
    /// set regardless of the request (kernel behaviour — the call succeeds but
    /// the two signals stay deliverable).
    pub fn apply(&mut self, how: SigprocmaskHow, arg: Self) {
        match how {
            SigprocmaskHow::Block => self.0 |= arg.0,
            SigprocmaskHow::Unblock => self.0 &= !arg.0,
            SigprocmaskHow::SetMask => self.0 = arg.0,
        }
        // SIGKILL / SIGSTOP are never blockable.
        if let (Some(k), Some(s)) = (Self::bit(SIGKILL), Self::bit(SIGSTOP)) {
            self.0 &= !(k | s);
        }
    }
}

/// A guest signal disposition — the kernel `struct sigaction` a guest registers
/// via `rt_sigaction`.
///
/// x86-64 layout: `sa_handler`(8), `sa_flags`(8), `sa_restorer`(8),
/// `sa_mask`(8) = 32 bytes. `handler` is the guest entry point (or `SIG_DFL`=0
/// / `SIG_IGN`=1); `mask` is the extra set blocked while the handler runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SigAction {
    /// Guest handler entry point (or `SIG_DFL` / `SIG_IGN`).
    pub handler: u64,
    /// `SA_*` flags.
    pub flags: u64,
    /// Restorer trampoline address (`SA_RESTORER`).
    pub restorer: u64,
    /// Signals blocked for the duration of the handler.
    pub mask: Sigset,
}

impl SigAction {
    /// On-wire size of the kernel `struct sigaction` in guest memory.
    pub const SIZE: usize = 32;

    /// Decode a `sigaction` from the front of `bytes`, or `None` if too short.
    #[must_use]
    pub fn from_guest_bytes(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.get(..Self::SIZE)?;
        Some(Self {
            handler: u64::from_le_bytes(raw[0..8].try_into().ok()?),
            flags: u64::from_le_bytes(raw[8..16].try_into().ok()?),
            restorer: u64::from_le_bytes(raw[16..24].try_into().ok()?),
            mask: Sigset::from_bits(u64::from_le_bytes(raw[24..32].try_into().ok()?)),
        })
    }

    /// Encode to the guest wire form.
    #[must_use]
    pub fn to_guest_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..8].copy_from_slice(&self.handler.to_le_bytes());
        out[8..16].copy_from_slice(&self.flags.to_le_bytes());
        out[16..24].copy_from_slice(&self.restorer.to_le_bytes());
        out[24..32].copy_from_slice(&self.mask.bits().to_le_bytes());
        out
    }
}

/// A guest thread's signal state: the pending queue plus the blocked mask.
///
/// Delivery honours the mask — a blocked signal stays pending until it is
/// unblocked, while SIGKILL/SIGSTOP (which [`Sigset`] never lets into the mask)
/// are always deliverable. This composes the pending queue and the `sigset_t`
/// into the model `rt_sigprocmask` and signal dispatch operate on.
#[derive(Debug, Default, Clone)]
pub struct SignalState {
    pending: GuestSignalQueue,
    blocked: Sigset,
    /// Non-default dispositions registered via `rt_sigaction`; an absent signal
    /// is `SIG_DFL`. Keyed by signal number (1..=64).
    actions: std::collections::HashMap<u32, SigAction>,
}

impl SignalState {
    /// Empty pending queue, nothing blocked.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `sig` pending against the guest thread.
    pub fn raise(&mut self, sig: u32) {
        self.pending.push(sig);
    }

    /// Apply an `rt_sigprocmask` operation to the blocked mask.
    pub fn set_mask(&mut self, how: SigprocmaskHow, set: Sigset) {
        self.blocked.apply(how, set);
    }

    /// The current blocked mask (for `rt_sigprocmask`'s old-set out-argument).
    #[must_use]
    pub const fn blocked(&self) -> Sigset {
        self.blocked
    }

    /// Pop the next deliverable (unblocked) pending signal, if any. Blocked
    /// signals stay queued until they are unblocked.
    #[must_use]
    pub fn next_deliverable(&mut self) -> Option<u32> {
        self.pending.pop_deliverable(&self.blocked)
    }

    /// Number of pending signals, blocked or not.
    #[must_use]
    pub const fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// The set of currently-pending signals as a mask — what `rt_sigpending`
    /// reports (each raised-but-undelivered signal sets its bit).
    #[must_use]
    pub fn pending_set(&self) -> Sigset {
        self.pending.to_sigset()
    }

    /// The disposition registered for `sig` (the default `SIG_DFL` action if
    /// none was set), or `None` if `sig` is outside the valid `1..=64` range.
    /// What `rt_sigaction` reports in its `oldact` out-argument.
    #[must_use]
    pub fn action(&self, sig: u32) -> Option<SigAction> {
        if matches!(sig, 1..=64) {
            Some(self.actions.get(&sig).copied().unwrap_or_default())
        } else {
            None
        }
    }

    /// Register `act` as the disposition for `sig`. Returns `false` (no change)
    /// for a signal outside `1..=64` or for SIGKILL/SIGSTOP, whose disposition
    /// the kernel never lets a process change.
    pub fn set_action(&mut self, sig: u32, act: SigAction) -> bool {
        if !matches!(sig, 1..=64) || sig == SIGKILL || sig == SIGSTOP {
            return false;
        }
        self.actions.insert(sig, act);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{GuestSignalQueue, SigAction, SignalState, SigprocmaskHow, Sigset};

    #[test]
    fn queue_roundtrip() {
        let mut q = GuestSignalQueue::new();
        assert!(q.is_empty());
        q.push(11);
        q.push(12);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(11));
        assert_eq!(q.pop(), Some(12));
        assert!(q.is_empty());
        assert_eq!(q.drain(), 0);
    }

    #[test]
    fn sigset_membership_and_bounds() {
        let mut s = Sigset::empty();
        assert!(!s.contains(11));
        assert!(s.insert(11)); // SIGSEGV
        assert!(s.contains(11));
        assert!(s.remove(11));
        assert!(!s.contains(11));
        // Out-of-range signals are rejected, not silently aliased.
        assert!(!s.insert(0));
        assert!(!s.insert(65));
        assert!(!s.contains(0));
    }

    #[test]
    fn sigset_round_trips_through_eight_le_bytes() {
        let mut s = Sigset::empty();
        s.insert(1); // bit 0
        s.insert(64); // bit 63
        let bytes = s.to_guest_bytes();
        assert_eq!(bytes.len(), Sigset::SIZE);
        assert_eq!(bytes[0] & 1, 1); // signal 1 -> bit 0 of byte 0
        assert_eq!(bytes[7] & 0x80, 0x80); // signal 64 -> bit 7 of byte 7
        assert_eq!(Sigset::from_guest_bytes(&bytes), Some(s));
        assert!(Sigset::from_guest_bytes(&[0u8; 7]).is_none());
    }

    #[test]
    fn sigprocmask_how_decodes_and_rejects_unknown() {
        assert_eq!(SigprocmaskHow::from_raw(0), Some(SigprocmaskHow::Block));
        assert_eq!(SigprocmaskHow::from_raw(1), Some(SigprocmaskHow::Unblock));
        assert_eq!(SigprocmaskHow::from_raw(2), Some(SigprocmaskHow::SetMask));
        assert_eq!(SigprocmaskHow::from_raw(3), None);
    }

    #[test]
    fn sigprocmask_block_unblock_setmask() {
        let mut mask = Sigset::empty();
        let mut arg = Sigset::empty();
        arg.insert(11);
        arg.insert(12);
        mask.apply(SigprocmaskHow::Block, arg);
        assert!(mask.contains(11) && mask.contains(12));
        let mut one = Sigset::empty();
        one.insert(11);
        mask.apply(SigprocmaskHow::Unblock, one);
        assert!(!mask.contains(11) && mask.contains(12));
        mask.apply(SigprocmaskHow::SetMask, Sigset::empty());
        assert_eq!(mask, Sigset::empty());
    }

    #[test]
    fn signal_state_delivery_honours_the_blocked_mask() {
        let mut st = SignalState::new();
        st.raise(10);
        st.raise(11); // SIGSEGV — we'll block this one
        st.raise(12);
        // Block signal 11: delivery skips it and returns the others in order.
        let mut block11 = Sigset::empty();
        block11.insert(11);
        st.set_mask(SigprocmaskHow::Block, block11);
        assert_eq!(st.next_deliverable(), Some(10));
        assert_eq!(st.next_deliverable(), Some(12));
        // 11 is still blocked -> nothing deliverable, but it stays pending.
        assert_eq!(st.next_deliverable(), None);
        assert_eq!(st.pending_count(), 1);
        // Unblock 11: it becomes deliverable.
        st.set_mask(SigprocmaskHow::Unblock, block11);
        assert_eq!(st.next_deliverable(), Some(11));
        assert_eq!(st.pending_count(), 0);
    }

    #[test]
    fn pending_set_reports_every_queued_signal() {
        let mut st = SignalState::new();
        assert_eq!(st.pending_set(), Sigset::empty());
        st.raise(11);
        st.raise(17);
        let set = st.pending_set();
        assert!(set.contains(11) && set.contains(17));
        assert!(!set.contains(12));
    }

    #[test]
    fn signal_state_never_blocks_sigkill() {
        let mut st = SignalState::new();
        st.raise(9); // SIGKILL
                     // Even an explicit attempt to block it leaves it deliverable.
        let mut all = Sigset::empty();
        all.insert(9);
        st.set_mask(SigprocmaskHow::Block, all);
        assert_eq!(st.next_deliverable(), Some(9));
    }

    #[test]
    fn pop_deliverable_on_empty_or_all_blocked_is_none() {
        let mut q = GuestSignalQueue::new();
        assert_eq!(q.pop_deliverable(&Sigset::empty()), None);
        q.push(15);
        let mut block15 = Sigset::empty();
        block15.insert(15);
        assert_eq!(q.pop_deliverable(&block15), None); // blocked -> not popped
        assert_eq!(q.len(), 1); // stays queued
        assert_eq!(q.pop_deliverable(&Sigset::empty()), Some(15));
    }

    #[test]
    fn sigkill_and_sigstop_can_never_be_blocked() {
        let mut mask = Sigset::empty();
        let mut arg = Sigset::empty();
        arg.insert(9); // SIGKILL
        arg.insert(19); // SIGSTOP
        arg.insert(15); // SIGTERM (blockable)
        mask.apply(SigprocmaskHow::Block, arg);
        assert!(!mask.contains(9), "SIGKILL must stay deliverable");
        assert!(!mask.contains(19), "SIGSTOP must stay deliverable");
        assert!(mask.contains(15));
        // Even SetMask cannot install them.
        mask.apply(SigprocmaskHow::SetMask, arg);
        assert!(!mask.contains(9) && !mask.contains(19));
    }

    #[test]
    fn sigaction_round_trips_through_its_32_byte_layout() {
        let mut mask = Sigset::empty();
        mask.insert(13);
        let act = SigAction {
            handler: 0x1_4000_3000,
            flags: 0x0400_0004, // SA_RESTORER | SA_SIGINFO, illustrative
            restorer: 0x1_4000_9000,
            mask,
        };
        let bytes = act.to_guest_bytes();
        assert_eq!(bytes.len(), SigAction::SIZE);
        // Field order: handler / flags / restorer / mask, little-endian.
        assert_eq!(&bytes[0..8], &0x1_4000_3000u64.to_le_bytes());
        assert_eq!(&bytes[24..32], &mask.bits().to_le_bytes());
        assert_eq!(SigAction::from_guest_bytes(&bytes), Some(act));
        // A short buffer is rejected, not read past.
        assert!(SigAction::from_guest_bytes(&[0u8; 31]).is_none());
    }

    #[test]
    fn signal_state_stores_and_reports_dispositions() {
        let mut st = SignalState::new();
        // Unset signal reports SIG_DFL (the default action).
        assert_eq!(st.action(11), Some(SigAction::default()));
        let act = SigAction {
            handler: 0x1_4000_5000,
            flags: 4,
            restorer: 0,
            mask: Sigset::empty(),
        };
        assert!(st.set_action(11, act));
        assert_eq!(st.action(11), Some(act));
        // SIGKILL/SIGSTOP dispositions can never be changed.
        assert!(!st.set_action(9, act));
        assert!(!st.set_action(19, act));
        assert_eq!(st.action(9), Some(SigAction::default()));
        // Out-of-range signals have no disposition.
        assert!(st.action(0).is_none());
        assert!(st.action(65).is_none());
        assert!(!st.set_action(65, act));
    }
}
