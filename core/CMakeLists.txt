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

# ---------------------------------------------------------------------------
# Library target: prisma_ir
# ---------------------------------------------------------------------------
#
# A static library holding the IR data structures and pure helpers. Passes,
# backend, cache, and runtime are future subtargets (Fase 1+).

add_library(prisma_ir STATIC
    src/ir/operation.cpp
    src/ir/pretty_print.cpp
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
    src/runtime/signal_handler.cpp
    src/runtime/dispatcher.cpp
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
# Translation cache (Pillar 4 seed — in-memory, SMC-aware)
# ---------------------------------------------------------------------------

add_library(prisma_cache STATIC
    src/cache/translation_cache.cpp
)

target_include_directories(prisma_cache
    PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(prisma_cache PRIVATE prisma_warnings)

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
        tests/test_signal_handler.cpp
        tests/test_translator.cpp
        tests/test_dispatcher.cpp
        tests/test_passes_advanced.cpp
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
