// core/src/runtime/guest_signal.cpp — F1-RT-011 implementation.
//
// The handler is held behind a function-local-static + mutex pair
// rather than a raw atomic<function> because std::function isn't
// cheap to atomically swap. Read-side (the trampoline) uses an
// atomic<bool> "has_handler" gate to avoid acquiring the mutex on
// every signal — the slow path only fires when the gate is true.

#include "prisma/guest_signal.hpp"

#include <atomic>
#include <mutex>

namespace prisma::runtime {

namespace {

std::mutex&                         handler_mutex() {
    static std::mutex m;
    return m;
}

GuestSignalHandler&                 handler_storage() {
    static GuestSignalHandler h;
    return h;
}

std::atomic<bool>&                  handler_present() {
    static std::atomic<bool> p{false};
    return p;
}

}  // namespace

void install_guest_signal_handler(GuestSignalHandler handler) {
    std::lock_guard<std::mutex> lk{handler_mutex()};
    handler_storage() = std::move(handler);
    handler_present().store(static_cast<bool>(handler_storage()),
                            std::memory_order_release);
}

void clear_guest_signal_handler() {
    std::lock_guard<std::mutex> lk{handler_mutex()};
    handler_storage() = nullptr;
    handler_present().store(false, std::memory_order_release);
}

bool has_guest_signal_handler() noexcept {
    return handler_present().load(std::memory_order_acquire);
}

GuestSignalDisposition deliver_guest_signal(const GuestSignal& sig) {
    if (!has_guest_signal_handler()) {
        return GuestSignalDisposition::LongjmpAbort;
    }
    GuestSignalHandler local;
    {
        std::lock_guard<std::mutex> lk{handler_mutex()};
        local = handler_storage();
    }
    if (!local) return GuestSignalDisposition::LongjmpAbort;
    return local(sig);
}

}  // namespace prisma::runtime
