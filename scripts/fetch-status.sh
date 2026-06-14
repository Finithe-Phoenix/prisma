#!/usr/bin/env bash
# fetch-status.sh — Quick status snapshot for multi-agent coordination
# Usage: bash scripts/fetch-status.sh
set -euo pipefail

echo "=== GIT STATUS ==="
git status --short

echo ""
echo "=== RECENT COMMITS (top 10) ==="
git log --oneline -10

echo ""
echo "=== UNTRACKED FILES ==="
git ls-files --others --exclude-standard

echo ""
echo "=== BRANCH ==="
git branch --show-current

echo ""
echo "=== DIFF STAT (working tree) ==="
git diff --stat

echo ""
echo "=== STASH ==="
git stash list

echo ""
echo "=== BACKLOG CLAIMS ==="
if [ -f docs/BACKLOG.md ]; then
    grep -n '\[~' docs/BACKLOG.md || echo "(none)"
fi

echo ""
echo "=== WORK_QUEUE ==="
if [ -f WORK_QUEUE.md ]; then
    grep -E '^## |\[ \]|\[x\]' WORK_QUEUE.md || echo "(no entries or missing)"
fi

echo ""
echo "=== LEAN SORRY BUDGET ==="
if [ -f ir-spec/.sorry-budget ]; then
    cat ir-spec/.sorry-budget
fi

echo ""
echo "=== CARGO TEST (shell) ==="
if command -v cargo &> /dev/null; then
    cargo test --manifest-path shell/Cargo.toml 2>&1 | tail -5 || true
else
    echo "cargo not available"
fi

echo ""
echo "=== CMAKE BUILD ==="
if [ -d core/build ]; then
    ls core/build/prisma_core_tests 2>/dev/null && echo "Build exists"
    echo "Core tests: $(core/build/prisma_core_tests --list-tests 2>/dev/null | wc -l) tests" || true
else
    echo "No cmake build directory"
fi
