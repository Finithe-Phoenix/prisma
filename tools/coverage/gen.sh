#!/usr/bin/env bash
# F1-TC-006 — local coverage HTML generation.
#
# Builds prisma_core under clang source-based coverage, runs the test
# suite, and emits an HTML coverage report to tools/coverage/html/.
# The CI workflow (planned: .github/workflows/coverage.yml) wraps
# this and uploads to the prisma-emu.dev/coverage subdomain — that
# upload step is gated on F0-LG-003 (domain registration) and an
# active CI runner.
#
# Local usage:
#   tools/coverage/gen.sh                    # full build + report
#   tools/coverage/gen.sh --skip-build       # reuse existing build-cov
#   tools/coverage/gen.sh --filter PATTERN   # restrict tests run
#
# Hard requirements:
#   * clang++ on PATH (LLVM coverage is clang-only).
#   * llvm-profdata + llvm-cov on PATH.
# These ship with Apple Xcode CommandLineTools on macOS and with
# llvm/clang packages on Linux.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="${REPO_ROOT}/core/build-cov"
COV_DIR="${REPO_ROOT}/tools/coverage"
HTML_DIR="${COV_DIR}/html"
PROF_RAW="${COV_DIR}/prisma-%p.profraw"
PROF_DATA="${COV_DIR}/prisma.profdata"

SKIP_BUILD=0
TEST_FILTER=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build)  SKIP_BUILD=1; shift ;;
        --filter)      TEST_FILTER="$2"; shift 2 ;;
        --help|-h)
            sed -n '2,/^$/p' "$0" | sed 's/^# \?//'; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

# 0. Tool resolution. Apple silicon ships llvm-profdata / llvm-cov
# inside CommandLineTools but doesn't symlink them into /usr/bin —
# `xcrun -f` finds them. Fall back to PATH for Linux + brew installs.
resolve() {
    if command -v "$1" >/dev/null 2>&1; then
        command -v "$1"; return
    fi
    if command -v xcrun >/dev/null 2>&1; then
        local p
        p=$(xcrun -f "$1" 2>/dev/null) || true
        if [[ -n "$p" && -x "$p" ]]; then echo "$p"; return; fi
    fi
    echo "error: '$1' not on PATH or xcrun (install via xcode-select / brew install llvm)" >&2
    exit 2
}
LLVM_PROFDATA=$(resolve llvm-profdata)
LLVM_COV=$(resolve llvm-cov)
if ! command -v clang++ >/dev/null 2>&1; then
    echo "error: clang++ not on PATH" >&2; exit 2
fi

# 1. Configure + build under coverage instrumentation.
if [[ "$SKIP_BUILD" == 0 ]]; then
    rm -rf "$BUILD_DIR"
    cmake -S "${REPO_ROOT}/core" -B "$BUILD_DIR" -G Ninja \
          -DCMAKE_BUILD_TYPE=Debug \
          -DPRISMA_ENABLE_COVERAGE=ON \
          -DCMAKE_CXX_COMPILER=clang++
    cmake --build "$BUILD_DIR"
fi

# 2. Run the test suite under the coverage harness. The
#    LLVM_PROFILE_FILE template gives us one .profraw per process
#    (the suite is single-process today, but the template is forward-
#    compatible with the future multi-process fuzzer harness).
mkdir -p "$COV_DIR"
rm -f "$COV_DIR"/*.profraw
LLVM_PROFILE_FILE="$PROF_RAW" \
    "$BUILD_DIR/prisma_core_tests" \
    --reporter compact \
    ${TEST_FILTER:+"$TEST_FILTER"} \
    "~signal_handler*" \
    || true   # don't abort if the macOS signal_handler flake fires

# 3. Merge raw profiles + emit HTML report.
shopt -s nullglob
RAWS=( "$COV_DIR"/*.profraw )
if [[ ${#RAWS[@]} -eq 0 ]]; then
    echo "error: no .profraw files were produced; tests didn't run under " \
         "coverage (was the build instrumented?)" >&2
    exit 3
fi
"$LLVM_PROFDATA" merge -sparse "${RAWS[@]}" -o "$PROF_DATA"

rm -rf "$HTML_DIR"
mkdir -p "$HTML_DIR"
"$LLVM_COV" show \
    "$BUILD_DIR/prisma_core_tests" \
    -instr-profile="$PROF_DATA" \
    -format=html \
    -output-dir="$HTML_DIR" \
    -ignore-filename-regex='_deps/' \
    -ignore-filename-regex='/tests/' \
    >/dev/null

# 4. Print the headline number.
SUMMARY=$("$LLVM_COV" report \
    "$BUILD_DIR/prisma_core_tests" \
    -instr-profile="$PROF_DATA" \
    -ignore-filename-regex='_deps/' \
    -ignore-filename-regex='/tests/' \
    | tail -1)
echo "------------------------------------------------------------"
echo "Coverage report → $HTML_DIR/index.html"
echo "Summary line:    $SUMMARY"
echo "------------------------------------------------------------"
