// core/src/runtime/host_features.cpp — detection implementation.
//
// macOS uses sysctlbyname. Linux uses HWCAP. Windows/other hosts return
// the all-false default, which is safe: the legacy encodings we already
// emit work everywhere.

#include "prisma/host_features.hpp"

#include <cstddef>
#include <mutex>
#include <optional>
#include <string>

#if defined(__APPLE__)
#  include <sys/sysctl.h>
#endif

#if defined(__linux__)
#  include <sys/auxv.h>
#  if __has_include(<asm/hwcap.h>)
#    include <asm/hwcap.h>
#  endif
#endif

namespace prisma::runtime {

namespace {

[[maybe_unused]] bool sysctl_bool(const char* key) noexcept {
#if defined(__APPLE__)
    int      value = 0;
    std::size_t size  = sizeof(value);
    if (::sysctlbyname(key, &value, &size, nullptr, 0) == 0) {
        return value != 0;
    }
#else
    (void)key;
#endif
    return false;
}

HostFeatures detect() {
    HostFeatures f{};

#if defined(__APPLE__)
    // Apple publishes FEAT_* directly under hw.optional.arm. Not every
    // key exists on every macOS version — missing keys return 0, which
    // we treat as "unsupported".
    f.feat_lse      = sysctl_bool("hw.optional.arm.FEAT_LSE");
    f.feat_lse2     = sysctl_bool("hw.optional.arm.FEAT_LSE2");
    f.feat_lrcpc    = sysctl_bool("hw.optional.arm.FEAT_LRCPC");
    f.feat_lrcpc2   = sysctl_bool("hw.optional.arm.FEAT_LRCPC2");
    f.feat_flagm    = sysctl_bool("hw.optional.arm.FEAT_FlagM");
    f.feat_flagm2   = sysctl_bool("hw.optional.arm.FEAT_FlagM2");
    f.feat_dotprod  = sysctl_bool("hw.optional.arm.FEAT_DotProd");
    f.feat_crc32    = sysctl_bool("hw.optional.arm.FEAT_CRC32");
    f.feat_sha1     = sysctl_bool("hw.optional.arm.FEAT_SHA1");
    f.feat_sha256   = sysctl_bool("hw.optional.arm.FEAT_SHA256");
#elif defined(__linux__)
    const unsigned long hwcap  = ::getauxval(AT_HWCAP);
#    ifdef AT_HWCAP2
    const unsigned long hwcap2 = ::getauxval(AT_HWCAP2);
#    else
    const unsigned long hwcap2 = 0;
#    endif

#    ifdef HWCAP_ATOMICS
    f.feat_lse     = (hwcap & HWCAP_ATOMICS)  != 0;
#    endif
#    ifdef HWCAP_USCAT
    f.feat_lse2    = (hwcap & HWCAP_USCAT)    != 0;
#    endif
#    ifdef HWCAP_LRCPC
    f.feat_lrcpc   = (hwcap & HWCAP_LRCPC)    != 0;
#    endif
#    ifdef HWCAP_ILRCPC
    f.feat_lrcpc2  = (hwcap & HWCAP_ILRCPC)   != 0;
#    endif
#    ifdef HWCAP_FLAGM
    f.feat_flagm   = (hwcap & HWCAP_FLAGM)    != 0;
#    endif
#    ifdef HWCAP2_FLAGM2
    f.feat_flagm2  = (hwcap2 & HWCAP2_FLAGM2) != 0;
#    endif
#    ifdef HWCAP_ASIMDDP
    f.feat_dotprod = (hwcap & HWCAP_ASIMDDP)  != 0;
#    endif
#    ifdef HWCAP_CRC32
    f.feat_crc32   = (hwcap & HWCAP_CRC32)    != 0;
#    endif
#    ifdef HWCAP_SHA1
    f.feat_sha1    = (hwcap & HWCAP_SHA1)     != 0;
#    endif
#    ifdef HWCAP_SHA2
    f.feat_sha256  = (hwcap & HWCAP_SHA2)     != 0;
#    endif
    // Silence unused-var when the host kernel lacks the defines.
    (void)hwcap;
    (void)hwcap2;
#endif

    return f;
}

// Singleton holder so the first call triggers detection and later calls
// return the cached struct. The test override is a separate optional —
// when set, it shadows the detected values.
std::optional<HostFeatures>& override_slot() {
    static std::optional<HostFeatures> slot;
    return slot;
}

}  // namespace

const HostFeatures& host_features() {
    if (auto& slot = override_slot(); slot.has_value()) return *slot;
    static const HostFeatures cached = detect();
    return cached;
}

void override_host_features_for_test(HostFeatures f) noexcept {
    override_slot() = f;
}

void clear_host_features_override() noexcept {
    override_slot().reset();
}

}  // namespace prisma::runtime
