// core/tests/test_guest_signal.cpp — F1-RT-011.

#include <atomic>
#include <catch2/catch_test_macros.hpp>

#include "prisma/guest_signal.hpp"

using namespace prisma::runtime;

TEST_CASE("guest_signal: with no handler, deliver returns LongjmpAbort") {
    clear_guest_signal_handler();
    REQUIRE_FALSE(has_guest_signal_handler());

    GuestSignal sig{
        GuestException::PageFault, 0xDEADBEEFu, 0x4000u, /*pf_kind=*/1u};
    REQUIRE(deliver_guest_signal(sig) == GuestSignalDisposition::LongjmpAbort);
}

TEST_CASE("guest_signal: registered handler observes the GuestSignal verbatim") {
    GuestException last_kind = GuestException::Other;
    std::uint64_t  last_pc   = 0;
    std::uint8_t   last_pf   = 0;

    install_guest_signal_handler(
        [&](const GuestSignal& s) -> GuestSignalDisposition {
            last_kind = s.kind;
            last_pc   = s.guest_pc;
            last_pf   = s.pf_kind;
            return GuestSignalDisposition::Resume;
        });
    REQUIRE(has_guest_signal_handler());

    GuestSignal sig{
        GuestException::PageFault, 0x1000u, 0x4000u, /*pf_kind=*/2u};
    REQUIRE(deliver_guest_signal(sig) == GuestSignalDisposition::Resume);
    REQUIRE(last_kind == GuestException::PageFault);
    REQUIRE(last_pc == 0x4000u);
    REQUIRE(last_pf == 2u);

    clear_guest_signal_handler();
}

TEST_CASE("guest_signal: handler can return LongjmpAbort to bail out") {
    install_guest_signal_handler(
        [](const GuestSignal&) -> GuestSignalDisposition {
            return GuestSignalDisposition::LongjmpAbort;
        });

    GuestSignal sig{GuestException::DivideError, 0u, 0x100u, 0u};
    REQUIRE(deliver_guest_signal(sig) == GuestSignalDisposition::LongjmpAbort);

    clear_guest_signal_handler();
}

TEST_CASE("guest_signal: clear_guest_signal_handler removes the previous one") {
    install_guest_signal_handler(
        [](const GuestSignal&) { return GuestSignalDisposition::Resume; });
    REQUIRE(has_guest_signal_handler());

    clear_guest_signal_handler();
    REQUIRE_FALSE(has_guest_signal_handler());

    GuestSignal sig{GuestException::InvalidOpcode, 0u, 0u, 0u};
    REQUIRE(deliver_guest_signal(sig) == GuestSignalDisposition::LongjmpAbort);
}

TEST_CASE("guest_signal: install replaces the prior handler") {
    std::atomic<int> counter{0};

    install_guest_signal_handler(
        [&](const GuestSignal&) {
            counter.fetch_add(1, std::memory_order_relaxed);
            return GuestSignalDisposition::Resume;
        });

    GuestSignal sig{GuestException::Other, 0u, 0u, 0u};
    (void)deliver_guest_signal(sig);
    REQUIRE(counter.load() == 1);

    install_guest_signal_handler(
        [&](const GuestSignal&) {
            counter.fetch_add(100, std::memory_order_relaxed);
            return GuestSignalDisposition::Resume;
        });
    (void)deliver_guest_signal(sig);
    REQUIRE(counter.load() == 101);

    clear_guest_signal_handler();
}
