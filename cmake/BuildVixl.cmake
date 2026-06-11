# cmake/BuildVixl.cmake — build vixl from a FetchContent-fetched source tree.
#
# vixl (https://github.com/Linaro/vixl, BSD-3-Clause) ships a SCons build only.
# Rather than invoke SCons from CMake, we build the aarch64 subset ourselves by
# listing the required translation units. This is fully documented here so the
# wrapper is auditable end-to-end.
#
# Usage (in core/CMakeLists.txt):
#
#     include(FetchContent)
#     FetchContent_Declare(vixl_src GIT_REPOSITORY ... GIT_TAG ...)
#     FetchContent_MakeAvailable(vixl_src)
#     include(${CMAKE_SOURCE_DIR}/../cmake/BuildVixl.cmake)
#     prisma_add_vixl(vixl SOURCE_DIR ${vixl_src_SOURCE_DIR})
#     target_link_libraries(my_target PRIVATE vixl)

function(prisma_add_vixl TARGET_NAME)
    set(options)
    set(oneValueArgs SOURCE_DIR)
    set(multiValueArgs)
    cmake_parse_arguments(ARG "${options}" "${oneValueArgs}" "${multiValueArgs}" ${ARGN})

    if(NOT ARG_SOURCE_DIR)
        message(FATAL_ERROR "prisma_add_vixl requires SOURCE_DIR")
    endif()

    set(VIXL_ROOT ${ARG_SOURCE_DIR})

    # aarch64-only sources. Simulator / logic / debugger excluded — we run
    # generated code natively on ARM64 hosts, and on x86_64 we verify bytes
    # structurally rather than simulate execution. Simulator can be re-enabled
    # via a future PRISMA_VIXL_WITH_SIMULATOR option.
    set(VIXL_SOURCES
        ${VIXL_ROOT}/src/code-buffer-vixl.cc
        ${VIXL_ROOT}/src/compiler-intrinsics-vixl.cc
        ${VIXL_ROOT}/src/cpu-features.cc
        ${VIXL_ROOT}/src/utils-vixl.cc
        ${VIXL_ROOT}/src/aarch64/assembler-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/assembler-sve-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/cpu-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/cpu-features-auditor-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/decoder-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/disasm-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/instructions-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/macro-assembler-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/macro-assembler-sve-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/operands-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/pointer-auth-aarch64.cc
        ${VIXL_ROOT}/src/aarch64/registers-aarch64.cc
    )

    add_library(${TARGET_NAME} STATIC ${VIXL_SOURCES})

    # SYSTEM include dirs: clang/gcc treat these as third-party, so header
    # warnings that would otherwise trip consumers' -Werror are silenced.
    target_include_directories(${TARGET_NAME}
        SYSTEM PUBLIC
            ${VIXL_ROOT}/src
            ${VIXL_ROOT}/src/aarch64
    )

    # AArch64 target on; simulator off (see rationale above).
    # VIXL_CODE_BUFFER_MALLOC chosen over MMAP: we manage JIT-safe memory at
    # a higher layer (Prisma's translation cache), so vixl only needs a
    # plain allocator for the intermediate buffer.
    target_compile_definitions(${TARGET_NAME} PUBLIC
        VIXL_INCLUDE_TARGET_A64
        VIXL_CODE_BUFFER_MALLOC
    )

    # vixl source requires C++17 as of 2026. Match the project's C++20
    # baseline by setting at least C++17.
    set_property(TARGET ${TARGET_NAME} PROPERTY CXX_STANDARD 17)
    set_property(TARGET ${TARGET_NAME} PROPERTY CXX_STANDARD_REQUIRED ON)

    # On MSVC, __cplusplus defaults to 199711L unless /Zc:__cplusplus is
    # passed. vixl checks `__cplusplus >= 201703L` to require C++17, so we
    # must enable the MSVC conformance mode flag here.
    target_compile_options(${TARGET_NAME} PRIVATE
        $<$<CXX_COMPILER_ID:MSVC>:/Zc:__cplusplus>
    )

    # vixl's own code style triggers warnings that our core would treat as
    # errors. Fully neutralise warnings for vixl translation units — this is
    # third-party code, we audit by version-pinning not by lint.
    # -w disables all warnings in clang/gcc; /W0 does the same on MSVC.
    target_compile_options(${TARGET_NAME} PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang,AppleClang>:-w>
        $<$<CXX_COMPILER_ID:MSVC>:/W0>
    )

    # Consumers expect to include headers as `<aarch64/macro-assembler-aarch64.h>`.
    # vixl's layout supports that via `src/aarch64/` being an include dir.

    set_property(TARGET ${TARGET_NAME} PROPERTY POSITION_INDEPENDENT_CODE ON)

    # Lightweight sanity message at configure time.
    message(STATUS "vixl: built from ${VIXL_ROOT} (A64 target, no simulator)")
endfunction()
