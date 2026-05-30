// prisma/translation_cache.hpp — in-memory translation cache (Pillar 4 seed).
//
// This is the first step toward the distributed cache pilar: a local,
// process-lifetime cache keyed by guest code identity so that repeated
// executions of the same basic block skip decode + lower + emit.
//
// What's here now:
//   * Key = (guest_addr, content_hash). Content hash gives us SMC
//     detection: if the guest code at `guest_addr` changes (self-modify),
//     the hash changes and the cache miss forces retranslation.
//   * Value = a stored translation (opaque bytes + metadata) owned by the
//     cache, returned to the caller via a borrowed view.
//   * Page-level invalidate(page_addr): drops every entry whose
//     guest_addr falls inside the page. Used by the runtime when it
//     observes a write to executable memory.
//   * Persistent file format: save_to_file / load_from_file round-trips
//     the entire cache through a deterministic binary layout (F1-CA-003).
//     The on-disk format carries a version + reserved CPU fingerprint
//     so a future Fase 2.5 agent can reject caches built for a
//     different host.
//   * LRU eviction: set_max_entries(N) bounds the cache. Exceeding N
//     evicts the least-recently-used entry (tracked by an internal
//     access counter bumped on every lookup hit and insert). 0 = no
//     bound (default).
//
// Threading: NOT thread-safe. One cache per translation thread, or wrap
// externally. A lock-free redesign arrives with the P2P cache work.

#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_map>
#include <variant>
#include <vector>

namespace prisma::cache {

// FNV-1a 64-bit hash over a byte range. Good enough for cache keys; the
// P2P signed-cache protocol uses SHA-256 (see Pillar 4) but intra-process
// dedup is fine with FNV.
[[nodiscard]] std::uint64_t fnv1a_64(std::span<const std::uint8_t> bytes) noexcept;

// An entry in the cache. `code_bytes` is the translated host bytes (ARM64
// for us); `meta` carries opaque metadata the runtime may attach, such as
// guest size or entry fix-up offsets. For the Fase 0 MVP we only store
// code + guest_size.
struct Entry {
    std::vector<std::uint8_t> code_bytes;
    std::size_t guest_size{0};          // length of the guest region translated
    std::uint64_t guest_content_hash{0};
};

// Composite key identifying a translation. Equality requires BOTH the
// address and the content hash to match — this is what makes the cache
// SMC-safe by construction.
struct Key {
    std::uint64_t guest_addr{0};
    std::uint64_t content_hash{0};

    [[nodiscard]] constexpr bool operator==(const Key&) const noexcept = default;
};

// Reason a cache lookup missed. Informational only; callers usually act
// the same way (retranslate). Useful for telemetry under Pilar 1's
// classifier.
enum class MissReason {
    UnknownAddress,  // we have never seen this guest_addr.
    StaleContent,    // address known but content_hash differs (SMC).
};

// Result of a lookup: either a const pointer to the cached entry (owned
// by the cache; valid until the next write or invalidate), or a reason.
using LookupResult = std::variant<const Entry*, MissReason>;

class TranslationCache {
public:
    TranslationCache() = default;
    ~TranslationCache();

    // Cache is not copyable (the writer thread holds a reference to
    // internal state that does not survive a copy). Move is safe via
    // default — per-thread ownership transfer.
    TranslationCache(const TranslationCache&)            = delete;
    TranslationCache& operator=(const TranslationCache&) = delete;

    // Look up a translation for `guest_addr` with content hashed from
    // the `guest_bytes` slice. If the address is known but the hash
    // mismatches, returns MissReason::StaleContent so callers know the
    // guest code has been modified and that a retranslation is warranted.
    [[nodiscard]] LookupResult lookup(std::uint64_t guest_addr,
                                      std::span<const std::uint8_t> guest_bytes) const;

    // Insert or replace the translation for (guest_addr, hash). The cache
    // takes ownership of `entry`. Returns true if inserted, false if an
    // entry with the same key already exists (caller decided not to
    // replace — use upsert for that).
    bool insert(Key key, Entry entry);

    // Unconditional replace: removes any prior entry at the same guest
    // addr (regardless of hash) and installs the new one. This is what
    // the translator calls after a retranslation.
    void upsert(Key key, Entry entry);

    // Drop every entry whose guest_addr falls within [page_addr,
    // page_addr + page_size). The runtime calls this when it observes a
    // write to a page of guest executable memory.
    void invalidate_page(std::uint64_t page_addr, std::size_t page_size);

    [[nodiscard]] std::size_t size() const noexcept { return addr_to_hash_.size(); }

    // Returns the number of distinct translations (address * hash pairs).
    [[nodiscard]] std::size_t entry_count() const noexcept { return entries_.size(); }

    // ------------------------------------------------------------------
    // Per-entry stats (F1-CA-007).
    //
    // `hit_count` is incremented on every successful lookup that returns
    // the entry; `last_used_tick` is the monotonic counter value at that
    // last use (same tick source as LRU). Stats are runtime-only and
    // are NOT persisted through save_to_file.
    struct EntryStats {
        std::uint64_t hit_count{0};
        std::uint64_t last_used_tick{0};
    };

    // Returns stats for `k`, or nullopt if no entry with that key exists.
    [[nodiscard]] std::optional<EntryStats> stats_for(const Key& k) const;

    // Reset all per-entry stats (hit counts zeroed; last-used ticks left
    // alone — they still drive LRU). Useful for tests and for resetting
    // telemetry at measurement boundaries.
    void reset_hit_counts() noexcept;

    // ------------------------------------------------------------------
    // LRU eviction (F1-CA-005 / F1-CA-006).
    //
    // Two independent caps can be set:
    //   * `max_entries` — hard ceiling on the count of cached entries.
    //   * `max_bytes`   — hard ceiling on the sum of `code_bytes.size()`
    //                     across all entries. Useful when a fixed code
    //                     buffer size drives the cache capacity.
    //
    // Either cap (or both) can be active. On insert / upsert the cache
    // evicts the least-recently-used entry until BOTH caps are
    // satisfied. A cap of 0 disables that dimension (the default).
    //
    // Lookup hits and insertions both bump an entry's "last used"
    // timestamp, so a cache with mixed entry sizes still picks the
    // oldest-touched entry first rather than the largest one.
    void set_max_entries(std::size_t n) noexcept { max_entries_ = n; }
    [[nodiscard]] std::size_t max_entries() const noexcept { return max_entries_; }

    // F1-CA-006: byte-budget bound. `n == 0` disables (default).
    void set_max_bytes(std::size_t n) noexcept { max_bytes_ = n; }
    [[nodiscard]] std::size_t max_bytes() const noexcept { return max_bytes_; }

    // Sum of `code_bytes.size()` across all live entries. Recomputed on
    // demand; cheap enough for caches at Prisma's current scale.
    [[nodiscard]] std::size_t total_code_bytes() const noexcept;

    // ------------------------------------------------------------------
    // Persistent on-disk format (F1-CA-003 / F1-CA-010 / RFC 0007+0008).
    //
    // Version 2 layout (all little-endian):
    //
    //   Header (32 bytes)
    //     u64 magic               = 0x50'52'53'4D'43'41'43'48  "PRSMCACH"
    //     u32 version             = 2
    //     u32 flags               bit 0 = entries_compressed
    //     u64 cpu_fingerprint     = 0 (reserved for Fase 2.5 host match)
    //     u64 entry_count
    //
    //   For each entry:
    //     u64 guest_addr
    //     u64 content_hash
    //     u64 guest_size
    //     u64 stored_size         (bytes on disk)
    //     u64 uncompressed_size   (raw code size; == stored_size when flag 0)
    //     u8  stored_bytes[stored_size]
    //
    // Version 1 (legacy, still readable):
    //
    //   Header (32 bytes)
    //     u64 magic ; u32 version=1 ; u32 reserved=0
    //     u64 cpu_fingerprint ; u64 entry_count
    //   For each entry:
    //     u64 guest_addr ; u64 content_hash ; u64 guest_size
    //     u64 code_size ; u8 code_bytes[code_size]
    //
    // The reader auto-detects the version and handles both shapes;
    // the writer always emits v2.

    static constexpr std::uint64_t kFileMagic         = 0x4843'4143'4D53'5250ULL;
    static constexpr std::uint32_t kFileVersion       = 2u;
    static constexpr std::uint32_t kFlagCompressed    = 1u << 0;

    enum class IoError {
        OpenFailed,        // fopen / ofstream failed.
        WriteFailed,       // any short / failed write.
        ReadFailed,        // short read mid-file.
        BadMagic,          // header magic mismatch.
        UnsupportedVersion,
        Truncated,         // entry_count claims more data than the file has.
    };

    // Serialise all entries to `path`. Returns std::nullopt on success
    // or an IoError on failure. The target file is overwritten.
    [[nodiscard]] std::optional<IoError>
    save_to_file(const std::filesystem::path& path) const;

    // ------------------------------------------------------------------
    // F1-CA-008 — compaction.
    //
    // The cache is keyed by `(guest_addr, content_hash)`. SMC at
    // `guest_addr` produces a fresh entry with a new content_hash;
    // the old entry stays around until evicted by LRU. `compact()`
    // walks the entries and drops every (guest_addr, content_hash)
    // pair whose content_hash isn't the most recent for that
    // guest_addr (i.e. every superseded SMC entry). Plus, when two
    // entries cover identical guest ranges with the same content,
    // only the freshest survives.
    //
    // Returns the number of entries evicted. Cheap: O(N) over the
    // entry table.
    std::size_t compact();

    // F1-CA-009: offload serialisation to a worker thread.
    //
    // Snapshots the current entries synchronously (deep copy), then
    // spawns a std::thread that writes the snapshot to `path` at the
    // disk's pace. Returns immediately.
    //
    // Threading contract:
    //   * Only one async save may be in flight per cache instance. A
    //     second call joins the in-flight worker before starting a new
    //     snapshot.
    //   * The snapshot is taken at call time — mutations to the live
    //     cache after the call do not appear in the saved file.
    //   * `wait_for_async_save()` must be called before destroying the
    //     cache (the destructor also joins defensively).
    void save_to_file_async(std::filesystem::path path);

    // F1-CA-010: set whether `save_to_file` / `save_to_file_async`
    // zstd-compress each entry's `code_bytes`. Default false. The
    // compression bit is recorded in the v2 file header; readers
    // handle both compressed and uncompressed files automatically.
    void set_compress_on_save(bool on) noexcept { compress_on_save_ = on; }
    [[nodiscard]] bool compress_on_save() const noexcept { return compress_on_save_; }

    // Block until any in-flight async save finishes. Returns the error
    // from that save (if any), or nullopt when no save was pending or
    // the save succeeded.
    [[nodiscard]] std::optional<IoError> wait_for_async_save();

    // Replace the current cache with the contents of `path`. On error
    // the cache is left unchanged; the method returns the IoError. The
    // access counter and LRU state are reset; on-disk caches are assumed
    // to carry no temporal information.
    [[nodiscard]] std::optional<IoError>
    load_from_file(const std::filesystem::path& path);

private:
    // Primary store: full key → entry. Keyed by the composite so SMC
    // doesn't lose the old entry until it's explicitly evicted.
    struct KeyHash {
        std::size_t operator()(const Key& k) const noexcept {
            return static_cast<std::size_t>(k.guest_addr ^ (k.content_hash * 0x9E3779B97F4A7C15ULL));
        }
    };
    std::unordered_map<Key, Entry, KeyHash> entries_;

    // Secondary index: guest_addr → latest content_hash we have seen.
    // Lets lookup() tell the difference between "never seen" and "stale".
    std::unordered_map<std::uint64_t, std::uint64_t> addr_to_hash_;

    // LRU tracking. access_times_[key] is the monotonic counter value at
    // this entry's last touch. 0 = never recorded (should not happen
    // after insert). next_tick_ increments on every use.
    std::unordered_map<Key, std::uint64_t, KeyHash> access_times_;
    std::uint64_t next_tick_{1};

    // Hit-count tracking for F1-CA-007. Bumped on every successful
    // lookup; zeroed by reset_hit_counts. Insert / upsert initialise the
    // count to zero.
    std::unordered_map<Key, std::uint64_t, KeyHash> hit_counts_;

    // 0 = unlimited (default). Both caps are checked in maybe_evict.
    std::size_t max_entries_{0};
    std::size_t max_bytes_{0};

    // Async save state (F1-CA-009). `writer_` runs a single worker
    // thread; `writer_error_` stores the result of the last completed
    // save (reset each time a new save is spawned).
    std::thread                    writer_;
    std::optional<IoError>         writer_error_;
    std::atomic<bool>              writer_in_flight_{false};

    // F1-CA-010: opt-in zstd compression on save. Read at save time
    // and snapshotted into the worker lambda, so toggling mid-save
    // doesn't race.
    bool                           compress_on_save_{false};

    // Pick the LRU key (smallest access_times_ value) from the current
    // map. Undefined result on empty cache; callers must guard.
    [[nodiscard]] Key pick_lru_key() const;

    // Erase a specific Key from every internal structure. Idempotent on
    // keys that are absent.
    void erase_key(const Key& k);

    // After adding a new entry, evict until we are back under max_entries_.
    // Called from insert / upsert.
    void maybe_evict();
};

}  // namespace prisma::cache
