#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCategory {
    SingleThreaded,
    LockFree,
    SharedMutable,
    IO,
    Unknown,
}

pub struct TsoClassifier;

impl Default for TsoClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl TsoClassifier {
    pub fn new() -> Self {
        Self
    }

    pub fn classify(&self, region_base: u64, size: u64) -> MemoryCategory {
        let region_end = region_base.saturating_add(size);
        if region_base >= 0x7FFF_0000 && region_end <= 0x8000_0000 {
            MemoryCategory::SingleThreaded
        } else {
            MemoryCategory::Unknown
        }
    }
}
