// core/src/cache/translation_cache.cpp — implementation.

#include "prisma/translation_cache.hpp"

#include "prisma/compress.hpp"

#include <algorithm>
#include <cstring>
#include <fstream>
#include <limits>
#include <vector>

namespace prisma::cache {

std::uint64_t fnv1a_64(std::span<const std::uint8_t> bytes) noexcept {
    constexpr std::uint64_t kFnvOffset = 0xcbf29ce484222325ULL;
    constexpr std::uint64_t kFnvPrime  = 0x100000001b3ULL;
    std::uint64_t h = kFnvOffset;
    for (std::uint8_t b : bytes) {
        h ^= static_cast<std::uint64_t>(b);
        h *= kFnvPrime;
    }
    return h;
}

// ---------------------------------------------------------------------------
// Internal helpers.
// ---------------------------------------------------------------------------

Key TranslationCache::pick_lru_key() const {
    // O(n) scan. Acceptable for caches up to a few thousand entries; a
    // true O(1) LRU (doubly-linked list + map iterator) is a later
    // refinement if profiling shows it.
    Key best{};
    std::uint64_t best_tick = std::numeric_limits<std::uint64_t>::max();
    for (const auto& [k, tick] : access_times_) {
        if (tick < best_tick) {
            best_tick = tick;
            best = k;
        }
    }
    return best;
}

void TranslationCache::erase_key(const Key& k) {
    auto it = entries_.find(k);
    if (it == entries_.end()) return;
    entries_.erase(it);
    auto it_addr = addr_to_hash_.find(k.guest_addr);
    if (it_addr != addr_to_hash_.end() && it_addr->second == k.content_hash) {
        addr_to_hash_.erase(it_addr);
    }
    access_times_.erase(k);
    hit_counts_.erase(k);
}

std::size_t TranslationCache::total_code_bytes() const noexcept {
    std::size_t total = 0;
    for (const auto& [_k, e] : entries_) total += e.code_bytes.size();
    return total;
}

void TranslationCache::maybe_evict() {
    // Loop until BOTH caps are satisfied (or either is disabled). An
    // insertion that itself exceeds max_bytes_ on its own will leave the
    // cache with just that one entry — the oldest-then-smallest spiral
    // is a concern only if caller inserts progressively larger entries;
    // real usage has bounded entry size.
    auto over_cap = [&]() {
        if (max_entries_ != 0 && entries_.size() > max_entries_) return true;
        if (max_bytes_   != 0 && total_code_bytes() > max_bytes_) return true;
        return false;
    };
    while (!entries_.empty() && over_cap()) {
        erase_key(pick_lru_key());
    }
}

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

LookupResult TranslationCache::lookup(
    std::uint64_t guest_addr,
    std::span<const std::uint8_t> guest_bytes) const {
    const auto it_addr = addr_to_hash_.find(guest_addr);
    if (it_addr == addr_to_hash_.end()) {
        return MissReason::UnknownAddress;
    }

    const std::uint64_t current_hash = fnv1a_64(guest_bytes);
    if (current_hash != it_addr->second) {
        return MissReason::StaleContent;
    }

    const Key k{guest_addr, current_hash};
    const auto it_entry = entries_.find(k);
    if (it_entry == entries_.end()) {
        // Defensive: the two maps agreed on a hash but the entry vanished.
        // Should not happen outside of a concurrent modification bug.
        return MissReason::StaleContent;
    }

    // Update LRU + hit count. `lookup` is const-facing; we treat access
    // tracking as internal bookkeeping. Alternative would be a mutable
    // member, which is more fragile across refactors.
    auto& self = *const_cast<TranslationCache*>(this);
    self.access_times_[k] = self.next_tick_++;
    self.hit_counts_[k] += 1;

    return &it_entry->second;
}

bool TranslationCache::insert(Key key, Entry entry) {
    const auto [it, inserted] = entries_.emplace(key, std::move(entry));
    if (!inserted) return false;
    addr_to_hash_[key.guest_addr] = key.content_hash;
    access_times_[key] = next_tick_++;
    hit_counts_[key] = 0;
    maybe_evict();
    return true;
}

void TranslationCache::upsert(Key key, Entry entry) {
    // Drop any prior entry for this guest_addr (possibly at a different hash).
    auto it_addr = addr_to_hash_.find(key.guest_addr);
    if (it_addr != addr_to_hash_.end() && it_addr->second != key.content_hash) {
        const Key old_key{key.guest_addr, it_addr->second};
        entries_.erase(old_key);
        access_times_.erase(old_key);
    }
    entries_[key] = std::move(entry);
    addr_to_hash_[key.guest_addr] = key.content_hash;
    access_times_[key] = next_tick_++;
    hit_counts_[key] = 0;
    maybe_evict();
}

std::optional<TranslationCache::EntryStats>
TranslationCache::stats_for(const Key& k) const {
    if (entries_.find(k) == entries_.end()) return std::nullopt;
    EntryStats s;
    if (auto it = hit_counts_.find(k); it != hit_counts_.end()) {
        s.hit_count = it->second;
    }
    if (auto it = access_times_.find(k); it != access_times_.end()) {
        s.last_used_tick = it->second;
    }
    return s;
}

void TranslationCache::reset_hit_counts() noexcept {
    for (auto& [_k, c] : hit_counts_) c = 0;
}

void TranslationCache::invalidate_page(std::uint64_t page_addr,
                                       std::size_t page_size) {
    // Collect addresses to drop first (cannot mutate while iterating).
    std::vector<std::uint64_t> to_drop;
    const std::uint64_t end = page_addr + static_cast<std::uint64_t>(page_size);
    for (const auto& [addr, hash] : addr_to_hash_) {
        if (addr >= page_addr && addr < end) {
            to_drop.push_back(addr);
        }
    }
    for (std::uint64_t addr : to_drop) {
        auto it_h = addr_to_hash_.find(addr);
        if (it_h != addr_to_hash_.end()) {
            const Key k{addr, it_h->second};
            entries_.erase(k);
            access_times_.erase(k);
            hit_counts_.erase(k);
            addr_to_hash_.erase(it_h);
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence. All integers are serialised little-endian. We assume a
// little-endian host (every ARM64 and x86_64 configuration we care about).
// ---------------------------------------------------------------------------

namespace {

using EntrySnapshot = std::vector<std::pair<Key, Entry>>;

void write_u32(std::ostream& os, std::uint32_t v) {
    unsigned char b[4];
    b[0] = static_cast<unsigned char>(v);
    b[1] = static_cast<unsigned char>(v >> 8);
    b[2] = static_cast<unsigned char>(v >> 16);
    b[3] = static_cast<unsigned char>(v >> 24);
    os.write(reinterpret_cast<const char*>(b), 4);
}

void write_u64(std::ostream& os, std::uint64_t v) {
    unsigned char b[8];
    for (int i = 0; i < 8; ++i) {
        b[static_cast<std::size_t>(i)] = static_cast<unsigned char>(v >> (8 * i));
    }
    os.write(reinterpret_cast<const char*>(b), 8);
}

bool read_u32(std::istream& is, std::uint32_t& v) {
    unsigned char b[4];
    if (!is.read(reinterpret_cast<char*>(b), 4)) return false;
    v = static_cast<std::uint32_t>(b[0])
      | (static_cast<std::uint32_t>(b[1]) << 8)
      | (static_cast<std::uint32_t>(b[2]) << 16)
      | (static_cast<std::uint32_t>(b[3]) << 24);
    return true;
}

bool read_u64(std::istream& is, std::uint64_t& v) {
    unsigned char b[8];
    if (!is.read(reinterpret_cast<char*>(b), 8)) return false;
    v = 0;
    for (int i = 0; i < 8; ++i) {
        v |= static_cast<std::uint64_t>(b[static_cast<std::size_t>(i)]) << (8 * i);
    }
    return true;
}

// Shared save path. Emits v2 format. Works on any range whose iterator
// value is a pair with `.first == Key` and `.second == Entry`.
template <typename Range>
std::optional<TranslationCache::IoError>
write_range_to_file(const std::filesystem::path& path,
                    std::size_t                  count,
                    const Range&                 entries,
                    bool                         compress_entries) {
    std::ofstream os(path, std::ios::binary | std::ios::trunc);
    if (!os) return TranslationCache::IoError::OpenFailed;

    const std::uint32_t flags =
        compress_entries ? TranslationCache::kFlagCompressed : 0u;

    write_u64(os, TranslationCache::kFileMagic);
    write_u32(os, TranslationCache::kFileVersion);
    write_u32(os, flags);
    write_u64(os, 0ull);                            // cpu fingerprint (future)
    write_u64(os, static_cast<std::uint64_t>(count));

    for (const auto& [key, entry] : entries) {
        write_u64(os, key.guest_addr);
        write_u64(os, key.content_hash);
        write_u64(os, static_cast<std::uint64_t>(entry.guest_size));

        const auto& raw = entry.code_bytes;
        std::vector<std::uint8_t> compressed;
        const std::uint8_t* payload_data = raw.data();
        std::size_t         payload_size = raw.size();
        if (compress_entries && !raw.empty()) {
            compressed = zstd_compress(
                std::span<const std::uint8_t>(raw.data(), raw.size()));
            if (compressed.empty()) return TranslationCache::IoError::WriteFailed;
            payload_data = compressed.data();
            payload_size = compressed.size();
        }

        write_u64(os, static_cast<std::uint64_t>(payload_size));         // stored_size
        write_u64(os, static_cast<std::uint64_t>(raw.size()));           // uncompressed_size
        if (payload_size > 0) {
            os.write(reinterpret_cast<const char*>(payload_data),
                     static_cast<std::streamsize>(payload_size));
        }
        if (!os) return TranslationCache::IoError::WriteFailed;
    }

    os.flush();
    if (!os) return TranslationCache::IoError::WriteFailed;
    return std::nullopt;
}

}  // namespace

std::optional<TranslationCache::IoError>
TranslationCache::save_to_file(const std::filesystem::path& path) const {
    return write_range_to_file(path, entries_.size(), entries_,
                               compress_on_save_);
}

void TranslationCache::save_to_file_async(std::filesystem::path path) {
    // Flush any in-flight save first — we only allow one at a time so
    // errors can be returned deterministically.
    (void)wait_for_async_save();

    // Snapshot entries by value. Taking the deep copy on the caller
    // thread means the worker never touches the live cache, and the
    // live cache is free to evolve immediately after this returns.
    EntrySnapshot snap;
    snap.reserve(entries_.size());
    for (const auto& kv : entries_) snap.push_back(kv);

    writer_error_.reset();
    writer_in_flight_.store(true, std::memory_order_release);
    // Snapshot the compression flag too — callers may flip it between
    // async calls but one save sees a single value.
    const bool compress = compress_on_save_;
    writer_ = std::thread(
        [this, path = std::move(path), snap = std::move(snap), compress]() mutable {
            auto err = write_range_to_file(path, snap.size(), snap, compress);
            writer_error_ = err;
            writer_in_flight_.store(false, std::memory_order_release);
        });
}

std::optional<TranslationCache::IoError>
TranslationCache::wait_for_async_save() {
    if (writer_.joinable()) writer_.join();
    auto err = writer_error_;
    writer_error_.reset();
    return err;
}

TranslationCache::~TranslationCache() {
    // Defensive: a user-forgotten async save shouldn't leak a thread
    // into the destructor's teardown. Block until any worker finishes.
    if (writer_.joinable()) writer_.join();
}

std::optional<TranslationCache::IoError>
TranslationCache::load_from_file(const std::filesystem::path& path) {
    std::ifstream is(path, std::ios::binary);
    if (!is) return IoError::OpenFailed;

    std::uint64_t magic = 0;
    if (!read_u64(is, magic))    return IoError::ReadFailed;
    if (magic != kFileMagic)     return IoError::BadMagic;

    std::uint32_t version = 0;
    std::uint32_t header_word = 0;  // v1: reserved ; v2: flags
    std::uint64_t cpu_fp = 0;
    std::uint64_t count = 0;
    if (!read_u32(is, version))      return IoError::ReadFailed;
    if (!read_u32(is, header_word))  return IoError::ReadFailed;
    if (!read_u64(is, cpu_fp))       return IoError::ReadFailed;
    if (!read_u64(is, count))        return IoError::ReadFailed;

    // v1 and v2 are both readable. Anything else is a hard stop.
    if (version != 1u && version != kFileVersion) {
        return IoError::UnsupportedVersion;
    }
    (void)cpu_fp;  // reserved for Fase 2.5 host-match check.

    const bool entries_compressed =
        (version >= 2u) && ((header_word & kFlagCompressed) != 0u);

    // Parse into a temporary so partial failure leaves *this untouched.
    std::unordered_map<Key, Entry, KeyHash> tmp_entries;
    std::unordered_map<std::uint64_t, std::uint64_t> tmp_addr;
    tmp_entries.reserve(static_cast<std::size_t>(count));
    tmp_addr.reserve(static_cast<std::size_t>(count));

    for (std::uint64_t i = 0; i < count; ++i) {
        Key key{};
        std::uint64_t guest_size  = 0;
        std::uint64_t stored_size = 0;
        std::uint64_t uncompr_size = 0;
        if (!read_u64(is, key.guest_addr))   return IoError::ReadFailed;
        if (!read_u64(is, key.content_hash)) return IoError::ReadFailed;
        if (!read_u64(is, guest_size))       return IoError::ReadFailed;
        if (!read_u64(is, stored_size))      return IoError::ReadFailed;

        if (version >= 2u) {
            if (!read_u64(is, uncompr_size)) return IoError::ReadFailed;
        } else {
            // v1: stored_size and uncompressed_size are the same thing.
            uncompr_size = stored_size;
        }

        std::vector<std::uint8_t> stored;
        if (stored_size > 0) {
            stored.resize(static_cast<std::size_t>(stored_size));
            if (!is.read(reinterpret_cast<char*>(stored.data()),
                         static_cast<std::streamsize>(stored_size))) {
                return IoError::Truncated;
            }
        }

        Entry e;
        e.guest_size         = static_cast<std::size_t>(guest_size);
        e.guest_content_hash = key.content_hash;
        if (entries_compressed) {
            auto decompressed = zstd_decompress(
                std::span<const std::uint8_t>(stored.data(), stored.size()));
            if (!decompressed) return IoError::Truncated;
            if (decompressed->size() != static_cast<std::size_t>(uncompr_size)) {
                return IoError::Truncated;
            }
            e.code_bytes = std::move(*decompressed);
        } else {
            e.code_bytes = std::move(stored);
        }

        tmp_entries.emplace(key, std::move(e));
        tmp_addr[key.guest_addr] = key.content_hash;
    }

    // Commit: atomic-ish replacement of internal state.
    entries_        = std::move(tmp_entries);
    addr_to_hash_   = std::move(tmp_addr);
    access_times_.clear();
    hit_counts_.clear();
    next_tick_      = 1;
    // Re-stamp access times so a loaded cache has a consistent LRU order.
    // Hit counts start at 0 — they're runtime-only telemetry.
    for (const auto& [k, _e] : entries_) {
        access_times_[k] = next_tick_++;
        hit_counts_[k]   = 0;
    }
    return std::nullopt;
}

}  // namespace prisma::cache
