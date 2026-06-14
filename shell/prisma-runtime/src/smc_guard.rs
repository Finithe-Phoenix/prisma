//! SMC guard scaffold.

use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_PAGE_SIZE: u64 = 4096;

/// Guard object to protect self-modifying code sections.
#[derive(Debug, Clone)]
pub struct SmcGuard {
    enabled: bool,
    page_size: u64,
    pages: BTreeMap<u64, BTreeSet<u64>>,
    pending: Vec<u64>,
    pending_pages: Vec<u64>,
}

impl Default for SmcGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl SmcGuard {
    /// Creates a guard instance.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            enabled: false,
            page_size: DEFAULT_PAGE_SIZE,
            pages: BTreeMap::new(),
            pending: Vec::new(),
            pending_pages: Vec::new(),
        }
    }

    /// Enables guard tracking.
    pub const fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables guard tracking.
    pub const fn disable(&mut self) {
        self.enabled = false;
    }

    /// Guard state flag.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Tracks a translated guest byte range under its cache key.
    pub fn on_translate(&mut self, addr: u64, len: u32, cache_key: u64) {
        if !self.enabled || len == 0 {
            return;
        }

        let start_page = self.page_base(addr);
        let last_addr = addr.saturating_add(u64::from(len).saturating_sub(1));
        let end_page = self.page_base(last_addr);
        let mut page = start_page;

        loop {
            self.pages.entry(page).or_default().insert(cache_key);
            if page == end_page {
                break;
            }
            page = page.saturating_add(self.page_size);
        }
    }

    /// Invalidates the tracked page containing `page_addr`.
    #[must_use]
    pub fn invalidate_page(&mut self, page_addr: u64) -> bool {
        let page = self.page_base(page_addr);
        let Some(keys) = self.pages.remove(&page) else {
            return false;
        };

        for key in keys {
            if !self.pending.contains(&key) {
                self.pending.push(key);
            }
        }
        if !self.pending_pages.contains(&page) {
            self.pending_pages.push(page);
        }
        true
    }

    /// Handles a write/fault address by invalidating its tracked page.
    #[must_use]
    pub fn handle_fault(&mut self, fault_addr: u64) -> bool {
        self.enabled && self.invalidate_page(fault_addr)
    }

    /// Drains cache keys invalidated since the last drain.
    #[must_use]
    pub fn drain_pending(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.pending)
    }

    /// Drains guest pages invalidated since the last drain.
    #[must_use]
    pub fn drain_pending_pages(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.pending_pages)
    }

    /// Number of tracked guest pages.
    #[must_use]
    pub fn tracked_page_count(&self) -> usize {
        self.pages.len()
    }

    /// Returns whether the page containing `addr` is currently tracked.
    #[must_use]
    pub fn is_tracked(&self, addr: u64) -> bool {
        self.pages.contains_key(&self.page_base(addr))
    }

    /// Number of pending invalidated cache keys.
    #[must_use]
    pub const fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Guest page size used by this guard.
    #[must_use]
    pub const fn page_size(&self) -> u64 {
        self.page_size
    }

    const fn page_base(&self, addr: u64) -> u64 {
        addr - (addr % self.page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_guard_does_not_track_translations() {
        let mut guard = SmcGuard::new();
        guard.on_translate(0x1000, 4, 1);

        assert_eq!(guard.tracked_page_count(), 0);
        assert!(!guard.handle_fault(0x1000));
        assert!(guard.drain_pending().is_empty());
        assert!(guard.drain_pending_pages().is_empty());
    }

    #[test]
    fn invalidates_only_matching_page_keys() {
        let mut guard = SmcGuard::new();
        guard.enable();
        guard.on_translate(0x1000, 8, 0xA);
        guard.on_translate(0x2000, 8, 0xB);

        assert_eq!(guard.tracked_page_count(), 2);
        assert!(guard.handle_fault(0x1004));
        assert!(!guard.is_tracked(0x1000));
        assert!(guard.is_tracked(0x2000));
        assert_eq!(guard.drain_pending(), vec![0xA]);
        assert_eq!(guard.drain_pending_pages(), vec![0x1000]);
        assert!(guard.drain_pending().is_empty());
        assert!(guard.drain_pending_pages().is_empty());
    }

    #[test]
    fn range_crossing_pages_invalidates_each_page_once() {
        let mut guard = SmcGuard::new();
        guard.enable();
        guard.on_translate(0x1FF0, 0x30, 0xC);

        assert_eq!(guard.tracked_page_count(), 2);
        assert!(guard.handle_fault(0x1000));
        assert_eq!(guard.pending_count(), 1);
        assert!(guard.handle_fault(0x2008));
        assert_eq!(guard.drain_pending(), vec![0xC]);
        assert_eq!(guard.drain_pending_pages(), vec![0x1000, 0x2000]);
        assert!(guard.drain_pending().is_empty());
        assert!(guard.drain_pending_pages().is_empty());
    }
}
