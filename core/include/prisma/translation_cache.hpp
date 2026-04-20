// prisma/translation_cache.hpp — in-memory translation cache (Pillar 4 seed).
//
// This is the first step toward the distributed cache pilar: a local,
// process-lifetime cache keyed by guest code identity so that repeated
// executions of the same basic block skip decode + lower + emit.
//
// What's here now (Fase 0):
//   * Key = (guest_addr, content_hash). Content hash gives us SMC
//     detection: if the guest code at `guest_addr` changes (self-modify),
//     the hash changes and the cache miss forces retranslation.
//   * Value = a stored translation (opaque bytes + metadata) owned by the
//     cache, returned to the caller via a borrowed view.
//   * Page-level invalidate(page_addr): drops every entry whose
//     guest_addr falls inside the page. Used by the runtime when it
//     observes a write to executable memory.
//   * No disk persistence. No LRU eviction yet — size grows unbounded
//     until invalidate() is called. Those land with RFC 0003 (tentative).
//
// Threading: NOT thread-safe. One cache per translation thread, or wrap
// externally. A lock-free redesign arrives with the P2P cache work.

#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>
#include <unordered_map>
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
};

}  // namespace prisma::cache
