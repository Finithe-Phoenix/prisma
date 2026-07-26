#include <catch2/catch_test_macros.hpp>
#include <windows.h>
#include <winternl.h>

// Forward declare the wow64cpu exports we want to test
extern "C" {
    __declspec(dllimport) NTSTATUS WINAPI BTCpuGetContext(HANDLE thread, HANDLE process, void *unknown, CONTEXT *context);
    __declspec(dllimport) void WINAPI BTCpuProcessInit(void);
    __declspec(dllimport) void WINAPI BTCpuThreadInit(void);
    __declspec(dllimport) void WINAPI BTCpuSimulate(void);
    __declspec(dllimport) NTSTATUS WINAPI BTCpuSetContext(HANDLE thread, HANDLE process, void *unknown, CONTEXT *context);
}

TEST_CASE("Wine wow64cpu STUB exports", "[wow64cpu][integration]") {
    SECTION("Initialization doesn't crash") {
        BTCpuProcessInit();
        BTCpuThreadInit();
        REQUIRE(true); // If we reach here, it didn't crash
    }

    SECTION("Context getters and setters return success") {
        CONTEXT ctx{};
        NTSTATUS res = BTCpuGetContext(nullptr, nullptr, nullptr, &ctx);
        REQUIRE(res == 0); // STATUS_SUCCESS

        res = BTCpuSetContext(nullptr, nullptr, nullptr, &ctx);
        REQUIRE(res == 0); // STATUS_SUCCESS
    }
}
