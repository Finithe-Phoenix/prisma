// core/src/cache/translation_cache.cpp — implementation.

#include "prisma/translation_cache.hpp"

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
    return &it_entry->second;
}

bool TranslationCache::insert(Key key, Entry entry) {
    const auto [it, inserted] = entries_.emplace(key, std::move(entry));
    if (!inserted) return false;
    addr_to_hash_[key.guest_addr] = key.content_hash;
    return true;
}

void TranslationCache::upsert(Key key, Entry entry) {
    // Drop any prior entry for this guest_addr (possibly at a different hash).
    auto it_addr = addr_to_hash_.find(key.guest_addr);
    if (it_addr != addr_to_hash_.end() && it_addr->second != key.content_hash) {
        entries_.erase(Key{key.guest_addr, it_addr->second});
    }
    entries_[key] = std::move(entry);
    addr_to_hash_[key.guest_addr] = key.content_hash;
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
            entries_.erase(Key{addr, it_h->second});
            addr_to_hash_.erase(it_h);
        }
    }
}

}  // namespace prisma::cache
