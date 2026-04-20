# fuzz/CMakeLists.txt — opt-in fuzzer targets.
#
# Requires a Clang toolchain with libFuzzer. Apple Clang 17+ ships it.
# Enable with `-DPRISMA_ENABLE_FUZZERS=ON` from the top-level build.
# Disabled by default so the normal build / CI flow stays fast.

option(PRISMA_ENABLE_FUZZERS "Build libFuzzer drivers" OFF)

if(NOT PRISMA_ENABLE_FUZZERS)
    return()
endif()

# Sanity: libFuzzer is a Clang feature. We gate on the compiler id so
# a GCC build doesn't fail mid-link with a cryptic message.
if(NOT CMAKE_CXX_COMPILER_ID STREQUAL "Clang"
   AND NOT CMAKE_CXX_COMPILER_ID STREQUAL "AppleClang")
    message(WARNING "PRISMA_ENABLE_FUZZERS=ON but compiler is not Clang/AppleClang; "
                    "fuzzer targets will not be defined.")
    return()
endif()

# Probe: Apple Clang often ships without the libFuzzer runtime
# (libclang_rt.fuzzer_osx.a) in /Library/Developer/CommandLineTools.
# Test-link a tiny program; skip the targets if libFuzzer is missing
# so the main build stays green.
include(CheckCXXSourceCompiles)
set(CMAKE_REQUIRED_FLAGS "-fsanitize=fuzzer")
check_cxx_source_compiles("
    #include <cstddef>
    #include <cstdint>
    extern \"C\" int LLVMFuzzerTestOneInput(const uint8_t*, size_t) { return 0; }
    int main() { return 0; }
" PRISMA_HAVE_LIBFUZZER)
set(CMAKE_REQUIRED_FLAGS "")

if(NOT PRISMA_HAVE_LIBFUZZER)
    message(STATUS "libFuzzer runtime not available in this toolchain; "
                   "skipping fuzz targets. Install full LLVM (e.g. "
                   "`brew install llvm` on macOS) and set "
                   "CMAKE_CXX_COMPILER to its clang++ to enable them.")
    return()
endif()

set(PRISMA_FUZZ_FLAGS -fsanitize=fuzzer,address,undefined)

function(prisma_add_fuzzer TARGET)
    set(oneValueArgs DRIVER)
    set(multiValueArgs LIBS)
    cmake_parse_arguments(F "" "${oneValueArgs}" "${multiValueArgs}" ${ARGN})

    add_executable(${TARGET} ${F_DRIVER})
    target_compile_options(${TARGET} PRIVATE ${PRISMA_FUZZ_FLAGS})
    target_link_options(${TARGET}    PRIVATE ${PRISMA_FUZZ_FLAGS})
    target_link_libraries(${TARGET}  PRIVATE ${F_LIBS})
    # Fuzzers aren't built as part of `ctest`; they're an explicit
    # developer-driven target.
endfunction()

prisma_add_fuzzer(prisma_fuzz_decoder
    DRIVER decoder/fuzz_decoder.cpp
    LIBS   prisma_decoder prisma_ir
)
