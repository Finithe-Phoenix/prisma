//! Execution dispatcher.
//!
//! Placeholder implementation for migration scaffolding.

use std::collections::HashSet;

use prisma_cache::cache::{fnv1a_64, LookupResult, MissReason};
use prisma_cache::{CacheEntry, CacheKey, TranslationCache};
use prisma_ir::{BasicBlock, Function, Op};

use crate::SmcGuard;

/// High-level dispatcher stub.
#[derive(Debug, Default, Clone)]
pub struct Dispatcher {
    active: bool,
    dispatched_blocks: usize,
}

impl Dispatcher {
    /// Creates a new dispatcher.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: false,
            dispatched_blocks: 0,
        }
    }

    /// Marks dispatcher as active.
    pub const fn start(&mut self) {
        self.active = true;
    }

    /// Marks dispatcher as inactive.
    pub const fn stop(&mut self) {
        self.active = false;
    }

    /// Number of blocks currently dispatched.
    #[must_use]
    pub const fn dispatched_count(&self) -> usize {
        self.dispatched_blocks
    }

    /// Runs one fetch/cache/translate dispatcher step without executing JIT code.
    ///
    /// Pending SMC invalidations are applied before probing the cache.
    pub fn run_with_callbacks<F, T>(
        &mut self,
        cache: &mut TranslationCache,
        guard: &mut SmcGuard,
        entry_pc: u64,
        max_steps: usize,
        mut fetch: F,
        mut translate: T,
    ) -> DispatchRunOutcome
    where
        F: FnMut(u64) -> Option<Vec<u8>>,
        T: FnMut(u64, &[u8]) -> Option<Vec<u8>>,
    {
        self.run_with_adapters(
            cache,
            guard,
            entry_pc,
            max_steps,
            &mut fetch,
            &mut translate,
        )
    }

    /// Runs one no-execute dispatcher step through stable fetch/translate traits.
    ///
    /// This is the migration seam that later grows into `Dispatcher::run()`.
    pub fn run_with_adapters<F, T>(
        &mut self,
        cache: &mut TranslationCache,
        guard: &mut SmcGuard,
        entry_pc: u64,
        max_steps: usize,
        fetch: &mut F,
        translate: &mut T,
    ) -> DispatchRunOutcome
    where
        F: GuestFetcher,
        T: GuestTranslator,
    {
        if max_steps == 0 {
            return DispatchRunOutcome::StepLimit;
        }

        self.start();
        let invalidated_pages = apply_smc_invalidations(cache, guard);
        let Some(guest_bytes) = fetch.fetch(entry_pc) else {
            self.stop();
            return DispatchRunOutcome::FetchFailed { invalidated_pages };
        };

        match probe_cache(cache, entry_pc, &guest_bytes) {
            DispatchCacheProbe::Hit(entry) => {
                self.dispatched_blocks = self.dispatched_blocks.saturating_add(1);
                self.stop();
                DispatchRunOutcome::CacheHit {
                    code_size: entry.code_size,
                    invalidated_pages,
                }
            }
            DispatchCacheProbe::Miss(reason) => {
                let Some(code_bytes) = translate.translate(entry_pc, &guest_bytes) else {
                    self.stop();
                    return DispatchRunOutcome::TranslateFailed {
                        miss: reason,
                        invalidated_pages,
                    };
                };
                let Some(key) =
                    install_translation(cache, guard, entry_pc, &guest_bytes, &code_bytes)
                else {
                    self.stop();
                    return DispatchRunOutcome::TranslateFailed {
                        miss: reason,
                        invalidated_pages,
                    };
                };
                self.dispatched_blocks = self.dispatched_blocks.saturating_add(1);
                self.stop();
                DispatchRunOutcome::Installed {
                    key,
                    code_size: u32::try_from(code_bytes.len()).unwrap_or(u32::MAX),
                    miss: reason,
                    invalidated_pages,
                }
            }
        }
    }
}

/// Guest byte source used by the no-execute dispatcher contract.
pub trait GuestFetcher {
    fn fetch(&mut self, guest_pc: u64) -> Option<Vec<u8>>;
}

impl<F> GuestFetcher for F
where
    F: FnMut(u64) -> Option<Vec<u8>>,
{
    fn fetch(&mut self, guest_pc: u64) -> Option<Vec<u8>> {
        self(guest_pc)
    }
}

/// Translation source used by the no-execute dispatcher contract.
pub trait GuestTranslator {
    fn translate(&mut self, guest_pc: u64, guest_bytes: &[u8]) -> Option<Vec<u8>>;
}

impl<F> GuestTranslator for F
where
    F: FnMut(u64, &[u8]) -> Option<Vec<u8>>,
{
    fn translate(&mut self, guest_pc: u64, guest_bytes: &[u8]) -> Option<Vec<u8>> {
        self(guest_pc, guest_bytes)
    }
}

/// Minimal Rust-only decode -> backend translator for dispatcher contract tests.
#[derive(Debug, Default, Clone)]
pub struct RustSmokeTranslator {
    lowerer: prisma_backend::Lowerer,
}

impl RustSmokeTranslator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lowerer: prisma_backend::Lowerer::new(),
        }
    }
}

impl GuestTranslator for RustSmokeTranslator {
    fn translate(&mut self, guest_pc: u64, guest_bytes: &[u8]) -> Option<Vec<u8>> {
        let mut blocks = Vec::new();
        let mut emitted_block_ids = HashSet::<u32>::new();

        let mut cursor = 0usize;
        while cursor < guest_bytes.len() {
            let instruction_guest_pc = guest_pc.wrapping_add(cursor as u64);
            let decoded =
                prisma_decoder::decode_one_at(guest_bytes, cursor, instruction_guest_pc).ok()?;
            if decoded.bytes_consumed == 0 {
                return None;
            }

            let block_id = u32::try_from(instruction_guest_pc).ok()?;
            emitted_block_ids.insert(block_id);
            blocks.push(BasicBlock {
                id: block_id,
                stmts: decoded.stmts,
            });

            cursor = cursor.saturating_add(decoded.bytes_consumed);
        }

        let mut missing_block_ids = HashSet::<u32>::new();
        for block in &blocks {
            for stmt in &block.stmts {
                if let Op::CondJumpRel(jump) = &stmt.op {
                    if let Ok(target_id) = u32::try_from(jump.target_guest_pc) {
                        if !emitted_block_ids.contains(&target_id) {
                            missing_block_ids.insert(target_id);
                        }
                    } else {
                        return None;
                    }
                    if let Ok(fallthrough_id) = u32::try_from(jump.fallthrough_guest_pc) {
                        if !emitted_block_ids.contains(&fallthrough_id) {
                            missing_block_ids.insert(fallthrough_id);
                        }
                    } else {
                        return None;
                    }
                }
            }
        }

        let mut synthetic_ids = missing_block_ids.into_iter().collect::<Vec<_>>();
        synthetic_ids.sort_unstable();
        for synthetic_id in synthetic_ids {
            emitted_block_ids.insert(synthetic_id);
            blocks.push(BasicBlock {
                id: synthetic_id,
                stmts: Vec::new(),
            });
        }

        let func = Function {
            entry: u32::try_from(guest_pc).ok()?,
            blocks,
        };
        let words = self.lowerer.lower_function(&func).ok()?;
        Some(words_to_le_bytes(&words))
    }
}

fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// Result of the no-execute dispatcher state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchRunOutcome {
    StepLimit,
    FetchFailed {
        invalidated_pages: usize,
    },
    TranslateFailed {
        miss: DispatchCacheMiss,
        invalidated_pages: usize,
    },
    CacheHit {
        code_size: u32,
        invalidated_pages: usize,
    },
    Installed {
        key: CacheKey,
        code_size: u32,
        miss: DispatchCacheMiss,
        invalidated_pages: usize,
    },
}

/// Runtime-visible cache probe outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchCacheProbe {
    Hit(CacheEntry),
    Miss(DispatchCacheMiss),
}

/// Runtime-visible cache miss reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchCacheMiss {
    UnknownAddress,
    StaleContent,
}

impl From<MissReason> for DispatchCacheMiss {
    fn from(value: MissReason) -> Self {
        match value {
            MissReason::UnknownAddress => Self::UnknownAddress,
            MissReason::StaleContent => Self::StaleContent,
        }
    }
}

/// Probe the translation cache through the dispatcher-facing contract.
#[must_use]
pub fn probe_cache(
    cache: &mut TranslationCache,
    guest_addr: u64,
    guest_bytes: &[u8],
) -> DispatchCacheProbe {
    match cache.lookup(guest_addr, guest_bytes) {
        LookupResult::Hit(entry) => DispatchCacheProbe::Hit(entry),
        LookupResult::Miss(reason) => DispatchCacheProbe::Miss(reason.into()),
    }
}

/// Applies page invalidations drained from `SmcGuard` to the translation cache.
///
/// Returns the number of guest pages applied to the cache.
pub fn apply_smc_invalidations(cache: &mut TranslationCache, guard: &mut SmcGuard) -> usize {
    let Ok(page_size) = usize::try_from(guard.page_size()) else {
        return 0;
    };
    let pages = guard.drain_pending_pages();
    let applied = pages.len();
    for page in pages {
        cache.invalidate_page(page, page_size);
    }
    applied
}

/// Installs translated code into the cache and registers its guest range with SMC tracking.
pub fn install_translation(
    cache: &mut TranslationCache,
    guard: &mut SmcGuard,
    guest_addr: u64,
    guest_bytes: &[u8],
    code_bytes: &[u8],
) -> Option<CacheKey> {
    let guest_size = u32::try_from(guest_bytes.len()).ok()?;
    let code_size = u32::try_from(code_bytes.len()).ok()?;
    let content_hash = fnv1a_64(guest_bytes);
    let key = (guest_addr, content_hash);
    let entry = CacheEntry {
        guest_addr,
        guest_size,
        code_size,
        code_bytes: code_bytes.to_vec().into_boxed_slice(),
        hit_count: 0,
        last_used: 0,
    };
    cache.upsert(key, entry);
    guard.on_translate(guest_addr, guest_size, content_hash);
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entry(bytes: &[u8], guest_size: usize) -> CacheEntry {
        CacheEntry {
            guest_addr: 0,
            guest_size: u32::try_from(guest_size).expect("test guest size fits in u32"),
            code_size: u32::try_from(bytes.len()).expect("test code size fits in u32"),
            code_bytes: bytes.to_vec().into_boxed_slice(),
            hit_count: 0,
            last_used: 0,
        }
    }

    #[test]
    fn cache_probe_surfaces_hit() {
        let guest = [0x90, 0xC3];
        let mut cache = TranslationCache::new();
        cache.insert(
            (0x1000, fnv1a_64(&guest)),
            entry(&[0xAA, 0xBB], guest.len()),
        );

        match probe_cache(&mut cache, 0x1000, &guest) {
            DispatchCacheProbe::Hit(hit) => {
                assert_eq!(hit.guest_addr, 0x1000);
                assert_eq!(&*hit.code_bytes, &[0xAA, 0xBB]);
                assert_eq!(hit.hit_count, 1);
            }
            DispatchCacheProbe::Miss(reason) => panic!("unexpected miss: {reason:?}"),
        }
    }

    #[test]
    fn cache_probe_surfaces_unknown_address_miss() {
        let mut cache = TranslationCache::new();
        assert_eq!(
            probe_cache(&mut cache, 0x2000, &[0xC3]),
            DispatchCacheProbe::Miss(DispatchCacheMiss::UnknownAddress)
        );
    }

    #[test]
    fn cache_probe_surfaces_stale_content_miss() {
        let guest = [0xC3];
        let mut cache = TranslationCache::new();
        cache.insert((0x3000, fnv1a_64(&guest)), entry(&[0xCC], guest.len()));

        assert_eq!(
            probe_cache(&mut cache, 0x3000, &[0x90, 0xC3]),
            DispatchCacheProbe::Miss(DispatchCacheMiss::StaleContent)
        );
    }

    #[test]
    fn cache_probe_hits_after_save_load_reuse() {
        let guest = [0x48, 0xC3];
        let path = std::env::temp_dir().join(format!(
            "prisma-runtime-cache-probe-{}.bin",
            std::process::id()
        ));

        let mut cache = TranslationCache::new();
        cache.insert(
            (0x4000, fnv1a_64(&guest)),
            entry(&[0x11, 0x22], guest.len()),
        );
        assert!(cache.save_to_file(&path).is_none());

        let mut loaded = TranslationCache::new();
        assert!(loaded.load_from_file(&path).is_none());
        let _ = fs::remove_file(&path);

        match probe_cache(&mut loaded, 0x4000, &guest) {
            DispatchCacheProbe::Hit(hit) => assert_eq!(&*hit.code_bytes, &[0x11, 0x22]),
            DispatchCacheProbe::Miss(reason) => panic!("unexpected miss: {reason:?}"),
        }
    }

    #[test]
    fn smc_invalidations_apply_to_matching_cache_page() {
        let guest_a = [0x90, 0xC3];
        let guest_b = [0x48, 0xC3];
        let mut cache = TranslationCache::new();
        cache.insert((0x1000, fnv1a_64(&guest_a)), entry(&[0xAA], guest_a.len()));
        cache.insert((0x2000, fnv1a_64(&guest_b)), entry(&[0xBB], guest_b.len()));

        let mut guard = SmcGuard::new();
        guard.enable();
        guard.on_translate(
            0x1000,
            u32::try_from(guest_a.len()).expect("test guest size fits in u32"),
            fnv1a_64(&guest_a),
        );
        guard.on_translate(
            0x2000,
            u32::try_from(guest_b.len()).expect("test guest size fits in u32"),
            fnv1a_64(&guest_b),
        );
        assert!(guard.handle_fault(0x1001));

        assert_eq!(apply_smc_invalidations(&mut cache, &mut guard), 1);
        assert_eq!(
            probe_cache(&mut cache, 0x1000, &guest_a),
            DispatchCacheProbe::Miss(DispatchCacheMiss::UnknownAddress)
        );
        assert!(matches!(
            probe_cache(&mut cache, 0x2000, &guest_b),
            DispatchCacheProbe::Hit(_)
        ));
        assert_eq!(apply_smc_invalidations(&mut cache, &mut guard), 0);
    }

    #[test]
    fn install_translation_ties_cache_probe_and_smc_invalidation() {
        let guest = [0x90, 0x90, 0xC3];
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();

        let key = install_translation(&mut cache, &mut guard, 0x5000, &guest, &[0xDD, 0xEE])
            .expect("small test translation installs");
        assert_eq!(key, (0x5000, fnv1a_64(&guest)));
        assert!(guard.is_tracked(0x5000));
        assert!(matches!(
            probe_cache(&mut cache, 0x5000, &guest),
            DispatchCacheProbe::Hit(_)
        ));

        assert!(guard.handle_fault(0x5001));
        assert_eq!(apply_smc_invalidations(&mut cache, &mut guard), 1);
        assert_eq!(
            probe_cache(&mut cache, 0x5000, &guest),
            DispatchCacheProbe::Miss(DispatchCacheMiss::UnknownAddress)
        );
    }

    #[test]
    fn run_with_callbacks_installs_on_cache_miss() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();

        let outcome = dispatcher.run_with_callbacks(
            &mut cache,
            &mut guard,
            0x6000,
            1,
            |pc| (pc == 0x6000).then(|| vec![0x90, 0xC3]),
            |_pc, guest| (guest == [0x90, 0xC3]).then(|| vec![0xAA]),
        );

        assert_eq!(
            outcome,
            DispatchRunOutcome::Installed {
                key: (0x6000, fnv1a_64(&[0x90, 0xC3])),
                code_size: 1,
                miss: DispatchCacheMiss::UnknownAddress,
                invalidated_pages: 0,
            }
        );
        assert_eq!(dispatcher.dispatched_count(), 1);
        assert!(guard.is_tracked(0x6000));
        assert!(matches!(
            probe_cache(&mut cache, 0x6000, &[0x90, 0xC3]),
            DispatchCacheProbe::Hit(_)
        ));
    }

    #[test]
    fn run_with_callbacks_uses_cache_hit_without_translate() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();
        install_translation(&mut cache, &mut guard, 0x7000, &[0xC3], &[0xBB])
            .expect("small test translation installs");

        let outcome = dispatcher.run_with_callbacks(
            &mut cache,
            &mut guard,
            0x7000,
            1,
            |pc| (pc == 0x7000).then(|| vec![0xC3]),
            |_pc, _guest| panic!("translate callback must not run on cache hit"),
        );

        assert_eq!(
            outcome,
            DispatchRunOutcome::CacheHit {
                code_size: 1,
                invalidated_pages: 0,
            }
        );
        assert_eq!(dispatcher.dispatched_count(), 1);
    }

    #[test]
    fn run_with_callbacks_applies_smc_invalidations_before_probe() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();
        install_translation(&mut cache, &mut guard, 0x8000, &[0x90], &[0x11])
            .expect("small test translation installs");
        assert!(guard.handle_fault(0x8000));

        let outcome = dispatcher.run_with_callbacks(
            &mut cache,
            &mut guard,
            0x8000,
            1,
            |pc| (pc == 0x8000).then(|| vec![0x90]),
            |_pc, guest| (guest == [0x90]).then(|| vec![0x22]),
        );

        assert_eq!(
            outcome,
            DispatchRunOutcome::Installed {
                key: (0x8000, fnv1a_64(&[0x90])),
                code_size: 1,
                miss: DispatchCacheMiss::UnknownAddress,
                invalidated_pages: 1,
            }
        );
        assert_eq!(dispatcher.dispatched_count(), 1);
        match probe_cache(&mut cache, 0x8000, &[0x90]) {
            DispatchCacheProbe::Hit(hit) => assert_eq!(&*hit.code_bytes, &[0x22]),
            DispatchCacheProbe::Miss(reason) => panic!("unexpected miss: {reason:?}"),
        }
    }

    #[test]
    fn run_with_callbacks_reports_fetch_translate_and_step_failures() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();

        assert_eq!(
            dispatcher.run_with_callbacks(
                &mut cache,
                &mut guard,
                0x9000,
                0,
                |_pc| Some(vec![0xC3]),
                |_pc, _guest| Some(vec![0xAA]),
            ),
            DispatchRunOutcome::StepLimit
        );
        assert_eq!(
            dispatcher.run_with_callbacks(
                &mut cache,
                &mut guard,
                0x9000,
                1,
                |_pc| None,
                |_pc, _guest| Some(vec![0xAA]),
            ),
            DispatchRunOutcome::FetchFailed {
                invalidated_pages: 0,
            }
        );
        assert_eq!(
            dispatcher.run_with_callbacks(
                &mut cache,
                &mut guard,
                0x9000,
                1,
                |_pc| Some(vec![0xC3]),
                |_pc, _guest| None,
            ),
            DispatchRunOutcome::TranslateFailed {
                miss: DispatchCacheMiss::UnknownAddress,
                invalidated_pages: 0,
            }
        );
    }

    struct StaticFetcher {
        pc: u64,
        bytes: Vec<u8>,
    }

    impl GuestFetcher for StaticFetcher {
        fn fetch(&mut self, guest_pc: u64) -> Option<Vec<u8>> {
            (guest_pc == self.pc).then(|| self.bytes.clone())
        }
    }

    #[test]
    fn run_with_adapters_accepts_stable_fetch_translate_traits() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();
        let mut fetcher = StaticFetcher {
            pc: 0xA000,
            bytes: vec![0xC3],
        };
        let mut translator = |_pc: u64, guest: &[u8]| (guest == [0xC3]).then(|| vec![0xFE]);

        let outcome = dispatcher.run_with_adapters(
            &mut cache,
            &mut guard,
            0xA000,
            1,
            &mut fetcher,
            &mut translator,
        );

        assert_eq!(
            outcome,
            DispatchRunOutcome::Installed {
                key: (0xA000, fnv1a_64(&[0xC3])),
                code_size: 1,
                miss: DispatchCacheMiss::UnknownAddress,
                invalidated_pages: 0,
            }
        );
    }

    #[test]
    fn rust_smoke_translator_wires_decoder_to_backend_without_jit() {
        let mut translator = RustSmokeTranslator::new();
        assert_eq!(translator.translate(0xB000, &[0x90]), Some(Vec::new()));
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0xB8, 0x42, 0, 0, 0, 0, 0, 0, 0]),
            Some(words_to_le_bytes(&[0xD280_0849, 0xF900_0369]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0x89, 0xC8]),
            Some(words_to_le_bytes(&[0xF940_0769, 0xF900_0369]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0x01, 0xC8]),
            Some(words_to_le_bytes(&[
                0xF940_0769,
                0xF940_036A,
                0x8B09_014B,
                0xF900_036B,
            ]))
        );
        assert!(translator
            .translate(0xB000, &[0x48, 0x39, 0xC8, 0x74, 0x02])
            .is_some_and(|bytes| !bytes.is_empty()));
        assert_eq!(translator.translate(0xB000, &[0xFF]), None);
    }

    #[test]
    fn run_with_adapters_uses_rust_smoke_translator() {
        let mut dispatcher = Dispatcher::new();
        let mut cache = TranslationCache::new();
        let mut guard = SmcGuard::new();
        guard.enable();
        let mut fetcher = StaticFetcher {
            pc: 0xB000,
            bytes: vec![0x90],
        };
        let mut translator = RustSmokeTranslator::new();

        let outcome = dispatcher.run_with_adapters(
            &mut cache,
            &mut guard,
            0xB000,
            1,
            &mut fetcher,
            &mut translator,
        );

        assert_eq!(
            outcome,
            DispatchRunOutcome::Installed {
                key: (0xB000, fnv1a_64(&[0x90])),
                code_size: 0,
                miss: DispatchCacheMiss::UnknownAddress,
                invalidated_pages: 0,
            }
        );
        assert!(guard.is_tracked(0xB000));
        assert!(matches!(
            probe_cache(&mut cache, 0xB000, &[0x90]),
            DispatchCacheProbe::Hit(_)
        ));
    }
}
