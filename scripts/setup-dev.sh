#!/usr/bin/env bash
# setup-dev.sh — One-time setup for Rust migration development
# Usage: bash scripts/setup-dev.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== Prisma Rust Migration — Dev Setup ==="

# 1. Check tools
echo "--- Checking tools ---"
command -v rustc >/dev/null 2>&1 && echo "  rustc: $(rustc --version)" || echo "  ERROR: rustc not found"
command -v cargo >/dev/null 2>&1 && echo "  cargo: $(cargo --version)" || echo "  ERROR: cargo not found"
command -v cmake >/dev/null 2>&1 && echo "  cmake: $(cmake --version | head -1)" || echo "  ERROR: cmake not found"

if command -v lake >/dev/null 2>&1; then
    echo "  lake (Lean): $(lake --version)"
else
    echo "  lake not found (optional, only for ir-spec work)"
fi

# 2. Verify workspace structure
echo "--- Checking workspace structure ---"
CRATES=("prisma-ir" "prisma-cache" "prisma-passes" "prisma-decoder" "prisma-runtime" "prisma-backend")
for crate in "${CRATES[@]}"; do
    if [ -d "$REPO_ROOT/shell/$crate/src" ]; then
        echo "  ✅ shell/$crate"
    else
        echo "  ❌ shell/$crate — missing src/"
    fi
done

# 3. Try cargo build (informational only)
echo "--- Attempting cargo build ---"
if cargo build --manifest-path "$REPO_ROOT/shell/Cargo.toml" 2>&1; then
    echo "  ✅ cargo build succeeded"
else
    echo "  ⚠️  cargo build had errors (expected for stub crates)"
fi

# 4. Try cmake build (C++ side must still work)
echo "--- Checking C++ build ---"
if [ ! -d "$REPO_ROOT/core/build" ]; then
    echo "  Creating core/build..."
    cmake -S "$REPO_ROOT/core" -B "$REPO_ROOT/core/build" -G Ninja -DCMAKE_BUILD_TYPE=Debug 2>&1
fi
if cmake --build "$REPO_ROOT/core/build" 2>&1; then
    echo "  ✅ C++ build succeeded"
else
    echo "  ⚠️  C++ build had errors (check core/ for compilation issues)"
fi

# 5. Environment setup hints
echo ""
echo "=== Next steps ==="
echo "  export PRISMA_CORE_LIB_DIR=\"$REPO_ROOT/core/build\""
echo "  cargo test --manifest-path $REPO_ROOT/shell/Cargo.toml --package prisma-ir"
echo "  cargo clippy --manifest-path $REPO_ROOT/shell/Cargo.toml -- -D warnings"
echo ""
echo "=== Dev setup complete ==="
