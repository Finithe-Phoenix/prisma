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
                match &stmt.op {
                    Op::CondJumpRel(jump) => {
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
                    Op::JumpRel(jump) => {
                        if let Ok(target_id) = u32::try_from(jump.target_guest_pc) {
                            if !emitted_block_ids.contains(&target_id) {
                                missing_block_ids.insert(target_id);
                            }
                        } else {
                            return None;
                        }
                    }
                    Op::CallRel(call) => {
                        if let Ok(target_id) = u32::try_from(call.target_guest_pc) {
                            if !emitted_block_ids.contains(&target_id) {
                                missing_block_ids.insert(target_id);
                            }
                        } else {
                            return None;
                        }
                    }
                    _ => {}
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
    use prisma_backend::assembler::{
        add_x, add_x_imm, and_x, b, b_cond, clz_x, cmp_x, crc32cx, cset_x, ldr_w_unsigned,
        ldr_x_unsigned, ldrb_unsigned, lsl_x, lsr_x, mov_x, movk_x, movz_x, msr_nzcv, mul_x, orr_x,
        rbit_x, rev_w, rev_x, str_w_unsigned, str_x_unsigned, strb_unsigned, sub_x, sxtw_x,
    };
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
            translator.translate(0xB000, &[0x48, 0xC7, 0xC0, 0x34, 0x12, 0x00, 0x00]),
            Some(words_to_le_bytes(&[
                movz_x(9, 0x1234, 0),
                str_x_unsigned(9, 27, 0)
            ]))
        );
        assert!(translator
            .translate(0xB000, &[0x6A, 0x7F])
            .is_some_and(|bytes| !bytes.is_empty()));
        // Flag writers now append persistent-RFLAGS publication after the
        // arithmetic; pin the stable computation prefix only.
        let assert_prefix = |translated: Option<Vec<u8>>, prefix: &[u32], name: &str| {
            let translated = translated.expect(name);
            let prefix = words_to_le_bytes(prefix);
            assert!(translated.starts_with(&prefix), "{name} prefix");
        };
        assert_prefix(
            translator.translate(0xB000, &[0x48, 0x05, 0x34, 0x12, 0x00, 0x00]),
            &[
                ldr_x_unsigned(9, 27, 0),
                movz_x(10, 0x1234, 0),
                add_x(11, 9, 10),
                str_x_unsigned(11, 27, 0),
            ],
            "add_rax_imm32",
        );
        assert_prefix(
            translator.translate(0xB000, &[0x48, 0x81, 0xC0, 0x34, 0x12, 0x00, 0x00]),
            &[
                movz_x(9, 0x1234, 0),
                ldr_x_unsigned(10, 27, 0),
                add_x(11, 10, 9),
                str_x_unsigned(11, 27, 0),
            ],
            "add_rax_group1_imm32",
        );
        assert_prefix(
            translator.translate(0xB000, &[0x48, 0xC1, 0xE0, 0x03]),
            &[
                movz_x(9, 3, 0),
                ldr_x_unsigned(10, 27, 0),
                lsl_x(11, 10, 9),
                str_x_unsigned(11, 27, 0),
            ],
            "shl_rax_imm8",
        );
        assert_prefix(
            translator.translate(0xB000, &[0x48, 0xD3, 0xE0]),
            &[
                ldrb_unsigned(9, 27, 8),
                ldr_x_unsigned(10, 27, 0),
                lsl_x(11, 10, 9),
                str_x_unsigned(11, 27, 0),
            ],
            "shl_rax_cl",
        );
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0x63, 0xC1]),
            Some(words_to_le_bytes(&[
                ldr_w_unsigned(9, 27, 8),
                sxtw_x(10, 9),
                str_x_unsigned(10, 27, 0),
            ]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0x8D, 0x41, 0x08]),
            Some(words_to_le_bytes(&[
                ldr_x_unsigned(9, 27, 8),
                movz_x(10, 8, 0),
                add_x_imm(11, 9, 8),
                str_x_unsigned(11, 27, 0),
            ]))
        );
        let contains_word = |translated: &[u8], word: u32| {
            translated
                .windows(4)
                .any(|w| w == word.to_le_bytes().as_slice())
        };
        {
            let translated = translator
                .translate(0xB000, &[0x48, 0x01, 0xC8])
                .expect("add_rax_rcx");
            let prefix = words_to_le_bytes(&[0xF940_0769, 0xF940_036A, 0x8B09_014B, 0xF900_036B]);
            assert!(translated.starts_with(&prefix), "add_rax_rcx prefix");
            // adds x25, x10, x9 — implicit ALU flag write still present.
            assert!(contains_word(&translated, 0xAB09_0159), "add_rax_rcx adds");
        }
        {
            let translated = translator
                .translate(0xB000, &[0x48, 0x85, 0xC8])
                .expect("test_rax_rcx");
            let prefix = words_to_le_bytes(&[ldr_x_unsigned(9, 27, 8), ldr_x_unsigned(10, 27, 0)]);
            assert!(translated.starts_with(&prefix), "test_rax_rcx prefix");
            assert!(
                contains_word(&translated, and_x(25, 10, 9))
                    && contains_word(&translated, cmp_x(25, 31)),
                "test_rax_rcx logical flags"
            );
        }
        assert!(translator
            .translate(0xB000, &[0x48, 0x39, 0xC8, 0x74, 0x02])
            .is_some_and(|bytes| !bytes.is_empty()));
        // Standalone Jcc now restores NZCV from persistent RFLAGS first;
        // pin the stable branch tail only.
        let assert_tail = |translated: Option<Vec<u8>>, tail: &[u32], name: &str| {
            let translated = translated.expect(name);
            let tail = words_to_le_bytes(tail);
            assert!(
                translated.ends_with(&tail),
                "{name} tail; translated words: {:08x?}",
                translated
                    .chunks_exact(4)
                    .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
                    .collect::<Vec<_>>()
            );
        };
        assert_tail(
            translator.translate(0xB000, &[0x74, 0x02]),
            &[0x5400_0040, 0x1400_0001],
            "je_forward",
        );
        {
            // Self-loop: the b.eq back-branch offset spans the NZCV restore
            // prefix, so pin the structure instead of the exact offset.
            let translated = translator
                .translate(0xB000, &[0x74, 0xFE])
                .expect("je_self");
            assert!(
                translated.ends_with(&words_to_le_bytes(&[0x1400_0001])),
                "je_self fallthrough tail"
            );
            let n = translated.len();
            let beq = u32::from_le_bytes(translated[n - 8..n - 4].try_into().unwrap());
            assert_eq!(beq & 0xFF00_000F, 0x5400_0000, "je_self b.eq opcode");
            assert_ne!(beq & 0x0080_0000, 0, "je_self branches backwards");
        }
        assert_tail(
            translator.translate(0xB000, &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00]),
            &[0x5400_0040, 0x1400_0001],
            "je_near_forward",
        );
        {
            // CMP publishes persistent RFLAGS and the following JE restores
            // NZCV; pin the loads, the subs, and the branch tail.
            let translated = translator
                .translate(0xB000, &[0x48, 0x39, 0xC8, 0x74, 0x02])
                .expect("cmp_then_je");
            assert!(
                translated.starts_with(&words_to_le_bytes(&[0xF940_0769, 0xF940_036A])),
                "cmp_then_je loads"
            );
            assert!(contains_word(&translated, 0xEB09_015F), "cmp_then_je subs");
            assert!(
                translated.ends_with(&words_to_le_bytes(&[0x5400_0040, 0x1400_0001])),
                "cmp_then_je branch tail"
            );
        }
        {
            let translated = translator
                .translate(0xB000, &[0x48, 0x39, 0xC8])
                .expect("cmp_rax_rcx");
            assert!(
                translated.starts_with(&words_to_le_bytes(&[0xF940_0769, 0xF940_036A])),
                "cmp_rax_rcx loads"
            );
            assert!(contains_word(&translated, 0xEB09_015F), "cmp_rax_rcx subs");
        }
        // Block-entry flag readers (CMOVcc/SETcc) restore NZCV from the
        // persistent RFLAGS between the operand loads and the select body;
        // pin the loads as prefix and the select body as tail.
        {
            let translated = translator
                .translate(0xB000, &[0x48, 0x0F, 0x44, 0xC1])
                .expect("cmovz_rax_rcx");
            assert!(
                translated.starts_with(&words_to_le_bytes(&[
                    ldr_x_unsigned(9, 27, 0),
                    ldr_x_unsigned(10, 27, 8),
                ])),
                "cmovz_rax_rcx loads"
            );
            assert!(
                translated.ends_with(&words_to_le_bytes(&[
                    b_cond(prisma_ir::CondCode::Eq, 12),
                    mov_x(11, 9),
                    b(8),
                    mov_x(11, 10),
                    str_x_unsigned(11, 27, 0),
                ])),
                "cmovz_rax_rcx select tail"
            );
        }
        {
            let translated = translator
                .translate(0xB000, &[0x0F, 0x94, 0xC0])
                .expect("setz_al");
            assert!(
                translated.starts_with(&words_to_le_bytes(&[movz_x(9, 1, 0), movz_x(10, 0, 0),])),
                "setz_al constants"
            );
            assert!(
                translated.ends_with(&words_to_le_bytes(&[
                    b_cond(prisma_ir::CondCode::Eq, 12),
                    mov_x(11, 10),
                    b(8),
                    mov_x(11, 9),
                    strb_unsigned(11, 27, 0),
                ])),
                "setz_al select tail"
            );
        }
        assert_prefix(
            translator.translate(0xB000, &[0xF3, 0x48, 0x0F, 0xBD, 0xC1]),
            &[
                ldr_x_unsigned(9, 27, 8),
                clz_x(10, 9),
                str_x_unsigned(10, 27, 0),
                cmp_x(10, 31),
                cset_x(3, prisma_ir::CondCode::Eq),
                cmp_x(9, 31),
                cset_x(7, prisma_ir::CondCode::Eq),
                movz_x(19, 30, 0),
                lsl_x(3, 3, 19),
                movz_x(19, 29, 0),
                lsl_x(7, 7, 19),
                orr_x(3, 3, 7),
                msr_nzcv(3),
            ],
            "lzcnt_rax_rcx",
        );
        assert_prefix(
            translator.translate(0xB000, &[0xF3, 0x48, 0x0F, 0xBC, 0xC1]),
            &[
                ldr_x_unsigned(9, 27, 8),
                rbit_x(10, 9),
                clz_x(10, 10),
                str_x_unsigned(10, 27, 0),
                cmp_x(10, 31),
                cset_x(3, prisma_ir::CondCode::Eq),
                cmp_x(9, 31),
                cset_x(7, prisma_ir::CondCode::Eq),
                movz_x(19, 30, 0),
                lsl_x(3, 3, 19),
                movz_x(19, 29, 0),
                lsl_x(7, 7, 19),
                orr_x(3, 3, 7),
                msr_nzcv(3),
            ],
            "tzcnt_rax_rcx",
        );
        assert_prefix(
            translator.translate(0xB000, &[0xF3, 0x48, 0x0F, 0xB8, 0xC1]),
            &[
                ldr_x_unsigned(9, 27, 8),
                mov_x(10, 9),
                movz_x(19, 1, 0),
                lsr_x(3, 10, 19),
                movz_x(7, 0x5555, 0),
                movk_x(7, 0x5555, 16),
                movk_x(7, 0x5555, 32),
                movk_x(7, 0x5555, 48),
                and_x(3, 3, 7),
                sub_x(10, 10, 3),
                movz_x(19, 2, 0),
                lsr_x(3, 10, 19),
                movz_x(7, 0x3333, 0),
                movk_x(7, 0x3333, 16),
                movk_x(7, 0x3333, 32),
                movk_x(7, 0x3333, 48),
                and_x(10, 10, 7),
                and_x(3, 3, 7),
                add_x(10, 10, 3),
                movz_x(19, 4, 0),
                lsr_x(3, 10, 19),
                add_x(10, 10, 3),
                movz_x(7, 0x0f0f, 0),
                movk_x(7, 0x0f0f, 16),
                movk_x(7, 0x0f0f, 32),
                movk_x(7, 0x0f0f, 48),
                and_x(10, 10, 7),
                movz_x(21, 0x0101, 0),
                movk_x(21, 0x0101, 16),
                movk_x(21, 0x0101, 32),
                movk_x(21, 0x0101, 48),
                mul_x(10, 10, 21),
                movz_x(19, 56, 0),
                lsr_x(10, 10, 19),
                str_x_unsigned(10, 27, 0),
                cmp_x(9, 31),
                cset_x(3, prisma_ir::CondCode::Eq),
                movz_x(19, 30, 0),
                lsl_x(3, 3, 19),
                msr_nzcv(3),
            ],
            "popcnt_rax_rcx",
        );
        assert_eq!(
            translator.translate(0xB000, &[0x48, 0x0F, 0xC8]),
            Some(words_to_le_bytes(&[
                ldr_x_unsigned(9, 27, 0),
                rev_x(10, 9),
                str_x_unsigned(10, 27, 0),
            ]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x41, 0x0F, 0xC8]),
            Some(words_to_le_bytes(&[
                ldr_w_unsigned(9, 27, 64),
                rev_w(10, 9),
                str_w_unsigned(10, 27, 64),
                // 32-bit write zero-extends: clear the upper word of r8's slot.
                str_w_unsigned(31, 27, 68),
            ]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xF2, 0x48, 0x0F, 0x38, 0xF1, 0xC1]),
            Some(words_to_le_bytes(&[
                ldr_x_unsigned(9, 27, 0),
                ldr_x_unsigned(10, 27, 8),
                crc32cx(11, 9, 10),
                str_x_unsigned(11, 27, 0),
            ]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x0F, 0x38, 0xF0, 0x08]),
            Some(words_to_le_bytes(&[
                ldr_x_unsigned(9, 27, 0),
                // guest VA rebased to host through the backend address scratch.
                ldr_x_unsigned(4, 27, 840),
                add_x(4, 9, 4),
                ldr_w_unsigned(10, 4, 0),
                rev_w(11, 10),
                str_w_unsigned(11, 27, 8),
                // 32-bit write zero-extends: clear the upper word of rcx's slot.
                str_w_unsigned(31, 27, 12),
            ]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0x0F, 0x38, 0xF1, 0x08]),
            Some(words_to_le_bytes(&[
                ldr_x_unsigned(9, 27, 0),
                ldr_w_unsigned(10, 27, 8),
                rev_w(11, 10),
                ldr_x_unsigned(4, 27, 840),
                add_x(4, 9, 4),
                str_w_unsigned(11, 4, 0),
            ]))
        );
        // Narrow CMP now also publishes persistent RFLAGS; pin the operand
        // loads as prefix and the shifted-compare core as a contiguous
        // subsequence.
        let contains_seq = |translated: &[u8], words: &[u32]| {
            let seq = words_to_le_bytes(words);
            translated.windows(seq.len()).any(|w| w == seq.as_slice())
        };
        let assert_narrow_cmp =
            |translated: Option<Vec<u8>>, loads: &[u32], core: &[u32], name: &str| {
                let translated = translated.expect(name);
                assert!(
                    translated.starts_with(&words_to_le_bytes(loads)),
                    "{name} loads"
                );
                assert!(contains_seq(&translated, core), "{name} shifted compare");
            };
        assert_narrow_cmp(
            translator.translate(0xB000, &[0x83, 0xF8, 0x10]),
            &[0xD280_0209, 0xB940_036A],
            &[
                movz_x(19, 32, 0),
                lsl_x(3, 10, 19),
                lsl_x(7, 9, 19),
                cmp_x(3, 7),
            ],
            "cmp_eax_imm8",
        );
        assert_narrow_cmp(
            translator.translate(0xB000, &[0x66, 0x83, 0xF8, 0x10]),
            &[0xD280_0209, 0x7940_036A],
            &[
                movz_x(19, 48, 0),
                lsl_x(3, 10, 19),
                lsl_x(7, 9, 19),
                cmp_x(3, 7),
            ],
            "cmp_ax_imm8",
        );
        assert_narrow_cmp(
            translator.translate(0xB000, &[0x83, 0xFB, 0x10]),
            &[0xD280_0209, 0xB940_1B6A],
            &[
                movz_x(19, 32, 0),
                lsl_x(3, 10, 19),
                lsl_x(7, 9, 19),
                cmp_x(3, 7),
            ],
            "cmp_ebx_imm8",
        );
        assert_narrow_cmp(
            translator.translate(0xB000, &[0x66, 0x83, 0xFB, 0x10]),
            &[0xD280_0209, 0x7940_336A],
            &[
                movz_x(19, 48, 0),
                lsl_x(3, 10, 19),
                lsl_x(7, 9, 19),
                cmp_x(3, 7),
            ],
            "cmp_bx_imm8",
        );
        assert_eq!(
            translator.translate(0xB000, &[0xEB, 0x00]),
            Some(words_to_le_bytes(&[0x1400_0001]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xEB, 0xFE]),
            Some(words_to_le_bytes(&[0x1400_0000]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xEB, 0x02]),
            Some(words_to_le_bytes(&[0x1400_0001]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xE9, 0x00, 0x00, 0x00, 0x00]),
            Some(words_to_le_bytes(&[0x1400_0001]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xE9, 0x04, 0x00, 0x00, 0x00]),
            Some(words_to_le_bytes(&[0x1400_0001]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xE8, 0x00, 0x00, 0x00, 0x00]),
            Some(words_to_le_bytes(&[0x1400_0001]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xC3]),
            Some(words_to_le_bytes(&[0xD65F_03C0]))
        );
        assert_eq!(
            translator.translate(0xB000, &[0xC2, 0x10, 0x00]),
            Some(words_to_le_bytes(&[
                0xF940_1375,
                0x9100_62B5,
                0xF900_1375,
                0xD65F_03C0,
            ]))
        );
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
