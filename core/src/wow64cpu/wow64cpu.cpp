#define WIN32_NO_STATUS
#include <windows.h>
#undef WIN32_NO_STATUS
#include <ntstatus.h>
#include <winternl.h>
#include "prisma/capi.h" // Our Prisma translator C API

typedef struct _SYSTEM_CPU_INFORMATION {
    USHORT ProcessorArchitecture;
    USHORT ProcessorLevel;
    USHORT ProcessorRevision;
    USHORT Reserved;
    ULONG ProcessorFeatureBits;
} SYSTEM_CPU_INFORMATION;

#ifdef _WIN32
#define WOW64_EXPORT __declspec(dllexport)
#else
#define WOW64_EXPORT __attribute__((visibility("default")))
#endif

// Wine wow64cpu interface exports

extern "C" {

WOW64_EXPORT NTSTATUS WINAPI BTCpuGetContext(HANDLE thread, HANDLE process, void *unknown, CONTEXT *context) {
    // Forward to Prisma context getter
    return STATUS_SUCCESS;
}

WOW64_EXPORT void WINAPI BTCpuProcessInit(void) {
    // Initialize Prisma instance for the process
}

WOW64_EXPORT NTSTATUS WINAPI BTCpuSetContext(HANDLE thread, HANDLE process, void *unknown, CONTEXT *context) {
    // Forward to Prisma context setter
    return STATUS_SUCCESS;
}

WOW64_EXPORT void WINAPI BTCpuThreadInit(void) {
    // Initialize Prisma thread state
}

WOW64_EXPORT void WINAPI BTCpuSimulate(void) {
    // The main execution loop!
    // This is called by Wine when it jumps from ARM64 back to x86_64 guest code.
    // It should invoke prisma_translator_run() or similar.
}

WOW64_EXPORT void WINAPI BTCpuFlushInstructionCache2(const void *addr, SIZE_T size) {
    // Flush Prisma translation cache for the given range
}

WOW64_EXPORT NTSTATUS WINAPI BTCpuNotifyMapViewOfSection(void *unknown1, void *unknown2, void *unknown3, SIZE_T unknown4, ULONG unknown5, ULONG unknown6) {
    return STATUS_SUCCESS;
}

// Optional exports
WOW64_EXPORT void WINAPI BTCpuNotifyMemoryAlloc(void *addr, SIZE_T size, ULONG type, ULONG prot, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuNotifyMemoryDirty(void *addr, SIZE_T size) {}
WOW64_EXPORT void WINAPI BTCpuNotifyMemoryFree(void *addr, SIZE_T size, ULONG type, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuNotifyMemoryProtect(void *addr, SIZE_T size, ULONG prot, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuNotifyProcessExecuteFlagsChange(ULONG flags) {}
WOW64_EXPORT void WINAPI BTCpuNotifyReadFile(HANDLE handle, void *addr, SIZE_T size, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuNotifyUnmapViewOfSection(void *addr, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuProcessTerm(HANDLE process, BOOL unknown, NTSTATUS status) {}
WOW64_EXPORT void WINAPI BTCpuThreadTerm(HANDLE thread, LONG exit_code) {}
WOW64_EXPORT void WINAPI BTCpuUpdateProcessorInformation(SYSTEM_CPU_INFORMATION *info) {}
WOW64_EXPORT NTSTATUS WINAPI BTCpuResetToConsistentState(EXCEPTION_POINTERS *ptrs) { return STATUS_SUCCESS; }

} // extern "C"
