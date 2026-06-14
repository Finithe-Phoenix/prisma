#!/usr/bin/env bash
# diff-test.sh — Differential testing between C++ and Rust
# Usage: bash scripts/diff-test.sh [--update] [--crate prisma-cache|prisma-passes|...]
set -euo pipefail

UPDATE=false
CRATE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --update) UPDATE=true; shift ;;
        --crate) CRATE="$2"; shift 2 ;;
        --help)
            echo "Usage: $0 [--update] [--crate prisma-*]"
            echo "  --update: regenerate reference outputs from C++"
            echo "  --crate: limit tests to specific crate"
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -d "$REPO_ROOT/core/build" ]; then
    echo "ERROR: C++ build not found. Run: cmake -S core -B core/build -G Ninja && cmake --build core/build"
    exit 1
fi

if [ ! -f "$REPO_ROOT/shell/Cargo.toml" ]; then
    echo "ERROR: No shell/Cargo.toml found"
    exit 1
fi

# Reference: generate C++ outputs for differential tests
generate_cpp_reference() {
    local crate="$1"
    echo "=== Generating C++ reference for $crate ==="
    case "$crate" in
        prisma-cache)
            "$REPO_ROOT/core/build/prisma_core_tests" "[cache]" --reporter compact
            ;;
        prisma-passes)
            "$REPO_ROOT/core/build/prisma_core_tests" "[pass]" --reporter compact
            ;;
        prisma-decoder)
            "$REPO_ROOT/core/build/prisma_core_tests" "[decoder]" --reporter compact
            ;;
        *)
            echo "Unknown crate: $crate"
            ;;
    esac
}

# Differential run: C++ and Rust side by side
diff_test() {
    local crate="$1"
    echo "=== Running differential tests for $crate ==="
    
    if [ "$UPDATE" = true ]; then
        generate_cpp_reference "$crate"
    fi
    
    cargo test --manifest-path "$REPO_ROOT/shell/Cargo.toml" --package "$crate" -- differential 2>&1 || true
}

# Main
if [ -n "$CRATE" ]; then
    diff_test "$CRATE"
else
    # Run all differential tests
    for crate in prisma-cache prisma-passes prisma-decoder; do
        if [ -d "$REPO_ROOT/shell/$crate/src" ]; then
            diff_test "$crate"
        fi
    done
fi

echo ""
echo "=== Done ==="
echo "If tests failed, check docs/DIFFERENTIAL_TESTING.md for debug instructions."
