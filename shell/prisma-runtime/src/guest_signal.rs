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
}

#[cfg(test)]
mod tests {
    use super::GuestSignalQueue;

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
}
