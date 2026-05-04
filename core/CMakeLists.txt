# core/CMakeLists.txt — Prisma DBT core engine
#
# Status: Fase 0 scaffolding. No DBT implementation yet; this sets up the
# build system, testing harness, and include layout that Fase 1 will fill in.
#
# vixl integration is deferred: vixl ships a SCons build, and writing a
# clean CMake wrapper deserves its own RFC (planned RFC 0002). Until then
# this CMakeLists builds IR data structures and tests only — no code
# emission.

cmake_minimum_required(VERSION 3.30)

project(prisma_core
    VERSION 0.0.1
    DESCRIPTION "Prisma DBT core engine (x86_64 → ARM64)"
    LANGUAGES CXX
)

# ---------------------------------------------------------------------------
# Compile settings
# ---------------------------------------------------------------------------

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Export compile_commands.json for clangd / clang-tidy.
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Treat warnings as errors in the core. We want a clean baseline.
if(NOT DEFINED PRISMA_WARNINGS_AS_ERRORS)
    set(PRISMA_WARNINGS_AS_ERRORS ON)
endif()

add_library(prisma_warnings INTERFACE)
target_compile_options(prisma_warnings INTERFACE
    -Wall -Wextra -Wpedantic
    -Wshadow -Wconversion -Wsign-conversion
    -Wold-style-cast -Wnon-virtual-dtor
    -Wcast-align -Wunused -Woverloaded-virtual
    -Wnull-dereference -Wdouble-promotion
)
if(PRISMA_WARNINGS_AS_ERRORS)
    target_compile_options(prisma_warnings INTERFACE -Werror)
endif()

# When sanitizers are enabled below, they attach to prisma_warnings so every
# library that already links prisma_warnings picks them up with zero extra
# wiring.

# ---------------------------------------------------------------------------
# Sanitizers (F1-TC-008).
#
# Orthogonal toggles. Combine ASan+UBSan safely; TSan is mutually exclusive
# with ASan (same runtime, different detectors). All three cost compile time
# and runtime, so we leave them off by default.
#
# Usage:
#   cmake -B build-asan  -DPRISMA_ENABLE_ASAN=ON  -DPRISMA_ENABLE_UBSAN=ON
#   cmake -B build-tsan  -DPRISMA_ENABLE_TSAN=ON
#   cmake -B build-msan  -DPRISMA_ENABLE_MSAN=ON  # clang only
# ---------------------------------------------------------------------------

option(PRISMA_ENABLE_ASAN  "Build with AddressSanitizer"              OFF)
option(PRISMA_ENABLE_UBSAN "Build with UndefinedBehaviorSanitizer"    OFF)
option(PRISMA_ENABLE_TSAN  "Build with ThreadSanitizer (excludes ASan)" OFF)
option(PRISMA_ENABLE_MSAN  "Build with MemorySanitizer (clang only)"  OFF)

add_library(prisma_sanitizers INTERFACE)

if(PRISMA_ENABLE_ASAN AND PRISMA_ENABLE_TSAN)
    message(FATAL_ERROR
        "PRISMA_ENABLE_ASAN and PRISMA_ENABLE_TSAN are mutually exclusive "
        "— they share a runtime.")
endif()

if(PRISMA_ENABLE_ASAN)
    target_compile_options(prisma_sanitizers INTERFACE
        -fsanitize=address -fno-omit-frame-pointer)
    target_link_options(prisma_sanitizers INTERFACE -fsanitize=address)
    message(STATUS "Prisma: AddressSanitizer enabled")
endif()

if(PRISMA_ENABLE_UBSAN)
    target_compile_options(prisma_sanitizers INTERFACE
        -fsanitize=undefined -fno-omit-frame-pointer
        -fno-sanitize-recover=undefined)
    target_link_options(prisma_sanitizers INTERFACE -fsanitize=undefined)
    message(STATUS "Prisma: UndefinedBehaviorSanitizer enabled")
endif()

if(PRISMA_ENABLE_TSAN)
    target_compile_options(prisma_sanitizers INTERFACE
        -fsanitize=thread -fno-omit-frame-pointer)
    target_link_options(prisma_sanitizers INTERFACE -fsanitize=thread)
    message(STATUS "Prisma: ThreadSanitizer enabled")
endif()

if(PRISMA_ENABLE_MSAN)
    if(NOT CMAKE_CXX_COMPILER_ID MATCHES "Clang")
        message(FATAL_ERROR "PRISMA_ENABLE_MSAN requires Clang")
    endif()
    target_compile_options(prisma_sanitizers INTERFACE
        -fsanitize=memory -fno-omit-frame-pointer
        -fsanitize-memory-track-origins=2)
    target_link_options(prisma_sanitizers INTERFACE -fsanitize=memory)
    message(STATUS "Prisma: MemorySanitizer enabled")
endif()

# Pipe sanitizers through the prisma_warnings INTERFACE target so all
# downstream libraries + tests pick them up transparently.
target_link_libraries(prisma_warnings INTERFACE prisma_sanitizers)

# ---------------------------------------------------------------------------
# Coverage instrumentation (F1-TC-005).
#
# Clang source-based coverage: pass `-fprofile-instr-generate
# -fcoverage-mapping` at compile + link time. After running the test
# binary, `llvm-profdata merge` + `llvm-cov show` produces per-file
# reports.
#
# Usage:
#   cmake -B build-cov -DPRISMA_ENABLE_COVERAGE=ON -DCMAKE_CXX_COMPILER=clang++
#   cmake --build build-cov
#   LLVM_PROFILE_FILE="prisma-%p.profraw" build-cov/prisma_core_tests
#   llvm-profdata merge -sparse prisma-*.profraw -o prisma.profdata
#   llvm-cov show build-cov/prisma_core_tests -instr-profile=prisma.profdata
# ---------------------------------------------------------------------------

option(PRISMA_ENABLE_COVERAGE "Build with clang source-based coverage" OFF)

add_library(prisma_coverage INTERFACE)
if(PRISMA_ENABLE_COVERAGE)
    if(NOT CMAKE_CXX_COMPILER_ID MATCHES "Clang")
        message(FATAL_ERROR
            "PRISMA_ENABLE_COVERAGE requires Clang (the llvm-cov ecosystem).")
    endif()
    target_compile_options(prisma_coverage INTERFACE
        -fprofile-instr-generate -fcoverage-mapping)
    target_link_options(prisma_coverage INTERFACE
        -fprofile-instr-generate -fcoverage-mapping)
    message(STATUS "Prisma: coverage instrumentation enabled")
endif()
target_link_libraries(prisma_warnings INTERFACE prisma_coverage)

# ---------------------------------------------------------------------------
# Library target: prisma_ir
# ---------------------------------------------------------------------------
#
# A static library holding the IR data structures and pure helpers. Passes,
# backend, cache, and runtime are future subtargets (Fase 1+).

add_library(prisma_ir STATIC
    src/ir/operation.cpp
    src/ir/pretty_print.cpp
    src/ir/validate.cpp
    src/ir/serialize.cpp
    src/ir/serialize_crc.cpp
    src/ir/cfg.cpp
    src/ir/dominators.cpp
    src/ir/profiler.cpp
)

target_include_directories(prisma_ir
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_ir PRIVATE prisma_warnings)

# ---------------------------------------------------------------------------
# Decoder library (x86_64 → Prisma IR)
# ---------------------------------------------------------------------------

add_library(prisma_decoder STATIC
    src/decoder/x86_decoder.cpp
)

target_include_directories(prisma_decoder
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_decoder
    PUBLIC  prisma_ir
    PRIVATE prisma_warnings
)

# ---------------------------------------------------------------------------
# Runtime library (JIT memory, signal handlers — growing incrementally)
# ---------------------------------------------------------------------------

add_library(prisma_runtime STATIC
    src/runtime/jit_memory.cpp
    src/runtime/jit_buffer_pool.cpp
    src/runtime/signal_handler.cpp
    src/runtime/dispatcher.cpp
    src/runtime/host_features.cpp
    src/runtime/smc_guard.cpp
    src/runtime/guest_signal.cpp
)

target_include_directories(prisma_runtime
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_runtime
    PUBLIC
        prisma_ir
        prisma_translator
    PRIVATE
        prisma_warnings
)

# ---------------------------------------------------------------------------
# IR optimization passes
# ---------------------------------------------------------------------------

add_library(prisma_passes STATIC
    src/passes/const_prop.cpp
    src/passes/dce.cpp
    src/passes/algebraic.cpp
    src/passes/cse.cpp
    src/passes/copy_prop.cpp
    src/passes/strength_reduce.cpp
    src/passes/branch_fold.cpp
    src/passes/flag_write_elim.cpp
    src/passes/redundant_load.cpp
    src/passes/dead_store.cpp
    src/passes/tail_call.cpp
    src/passes/peephole.cpp
    src/passes/pass_manager.cpp
)

target_include_directories(prisma_passes
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_passes
    PUBLIC  prisma_ir
    PRIVATE prisma_warnings
)

# ---------------------------------------------------------------------------
# zstd — BSD-3-Clause compression library (RFC 0008). Used by the
# translation cache for on-disk entry compression.
# ---------------------------------------------------------------------------

include(FetchContent)

set(ZSTD_BUILD_TESTS    OFF CACHE BOOL "")
set(ZSTD_BUILD_PROGRAMS OFF CACHE BOOL "")
set(ZSTD_BUILD_SHARED   OFF CACHE BOOL "")
set(ZSTD_BUILD_STATIC   ON  CACHE BOOL "")
set(ZSTD_LEGACY_SUPPORT OFF CACHE BOOL "")

FetchContent_Declare(
    zstd
    GIT_REPOSITORY https://github.com/facebook/zstd.git
    GIT_TAG        v1.5.7
    SOURCE_SUBDIR  build/cmake
    GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(zstd)

# ---------------------------------------------------------------------------
# Translation cache (Pillar 4 seed — in-memory, SMC-aware)
# ---------------------------------------------------------------------------

add_library(prisma_cache STATIC
    src/cache/translation_cache.cpp
    src/cache/sha256.cpp
    src/cache/compress.cpp
)

target_include_directories(prisma_cache
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
            ${zstd_SOURCE_DIR}/lib
)

target_link_libraries(prisma_cache
    PRIVATE prisma_warnings
            libzstd_static)

# ---------------------------------------------------------------------------
# Translator — top-level facade that combines decoder + passes + lowerer
# + cache + runtime into one public API.
# ---------------------------------------------------------------------------

add_library(prisma_translator STATIC
    src/translator/translator.cpp
)

target_include_directories(prisma_translator
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_translator
    PUBLIC
        prisma_ir
        prisma_decoder
        prisma_passes
        prisma_emitter
        prisma_runtime
        prisma_cache
    PRIVATE
        prisma_warnings
)

# ---------------------------------------------------------------------------
# vixl — ARM64 assembler library (BSD-3-Clause). See RFC 0002.
# ---------------------------------------------------------------------------

include(FetchContent)

FetchContent_Declare(
    vixl_src
    GIT_REPOSITORY https://github.com/Linaro/vixl.git
    GIT_TAG        59921852720418d0c27d443167af0df387f78f4e  # HEAD of main, 2026-04
    # GIT_SHALLOW TRUE disabled: specific-commit fetches need full clone.
)
FetchContent_MakeAvailable(vixl_src)

include(${CMAKE_CURRENT_SOURCE_DIR}/../cmake/BuildVixl.cmake)
prisma_add_vixl(prisma_vixl SOURCE_DIR ${vixl_src_SOURCE_DIR})

# ---------------------------------------------------------------------------
# Emitter library (uses vixl)
# ---------------------------------------------------------------------------

add_library(prisma_emitter STATIC
    src/backend/emitter.cpp
    src/backend/lowering.cpp
    src/backend/abi.cpp
)

target_include_directories(prisma_emitter
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_emitter
    PUBLIC  prisma_ir prisma_vixl
    PRIVATE prisma_warnings
)

# ---------------------------------------------------------------------------
# Testing
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Optional subprojects
# ---------------------------------------------------------------------------

# Fuzzers live under ../fuzz/ and build only when explicitly enabled.
# Their CMakeLists gates on Clang; a non-Clang build just skips.
if(EXISTS ${CMAKE_CURRENT_SOURCE_DIR}/../fuzz/CMakeLists.txt)
    add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../fuzz
                     ${CMAKE_CURRENT_BINARY_DIR}/fuzz)
endif()

option(PRISMA_BUILD_TESTS "Build Prisma core tests" ON)

if(PRISMA_BUILD_TESTS)
    include(FetchContent)

    FetchContent_Declare(
        Catch2
        GIT_REPOSITORY https://github.com/catchorg/Catch2.git
        GIT_TAG        v3.10.0  # pinned; update deliberately
        GIT_SHALLOW    TRUE
    )
    FetchContent_MakeAvailable(Catch2)
    list(APPEND CMAKE_MODULE_PATH ${Catch2_SOURCE_DIR}/extras)

    enable_testing()
    include(Catch)

    add_executable(prisma_core_tests
        tests/test_ir.cpp
        tests/test_arm64_encoding.cpp
        tests/test_emitter.cpp
        tests/test_decoder.cpp
        tests/test_e2e.cpp
        tests/test_jit_execution.cpp
        tests/test_passes.cpp
        tests/test_translation_cache.cpp
        tests/test_lowering.cpp
        tests/test_abi.cpp
        tests/test_cfg.cpp
        tests/test_dominators.cpp
        tests/test_profiler.cpp
        tests/test_jit_buffer_pool.cpp
        tests/test_peephole.cpp
        tests/test_guest_signal.cpp
        tests/test_cpu_state.cpp
        tests/test_property.cpp
        tests/test_signal_handler.cpp
        tests/test_smc_guard.cpp
        tests/test_translator.cpp
        tests/test_dispatcher.cpp
        tests/test_passes_advanced.cpp
        tests/test_passes_more.cpp
        tests/test_passes_mem.cpp
        tests/test_ir_validate.cpp
        tests/test_ir_serialize.cpp
        tests/test_host_features.cpp
        tests/test_cache_stress.cpp
        tests/test_sha256.cpp
        tests/test_benchmarks.cpp
    )
    target_link_libraries(prisma_core_tests
        PRIVATE
            prisma_ir
            prisma_decoder
            prisma_emitter
            prisma_runtime
            prisma_passes
            prisma_cache
            prisma_translator
            prisma_warnings
            Catch2::Catch2WithMain
    )
    catch_discover_tests(prisma_core_tests)
endif()
