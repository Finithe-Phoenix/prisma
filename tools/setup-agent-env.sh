#!/usr/bin/env bash
#
# tools/setup-agent-env.sh — onboard a fresh agent / machine to Prisma.
#
# Idempotent. Run from any directory; resolves repo root from this
# script's own location.
#
# Usage:
#   ./tools/setup-agent-env.sh                # full setup
#   ./tools/setup-agent-env.sh --no-skills    # skip Claude skills install
#   ./tools/setup-agent-env.sh --no-build     # skip CMake configure+build
#
# What this does:
#   1. Verifies host toolchain (cmake, ninja, clang/clang++, git, gh).
#   2. Optionally installs the obra/superpowers Claude skill set into
#      ~/.claude/skills/ — only runs if the user passed `--with-skills`
#      OR if a TTY is attached and the user confirms interactively.
#      The skills modify agent behaviour, so we never run silently.
#   3. Configures the core/build directory with sensible defaults.
#   4. Builds and runs the test suite (excl. signal_handler*).
#   5. Prints a "next steps" pointer to docs/HANDOFF.md.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

INSTALL_SKILLS=0   # opt-in
SKIP_BUILD=0
NO_TTY_PROMPT=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --with-skills)  INSTALL_SKILLS=1 ;;
        --no-skills)    INSTALL_SKILLS=0 ;;
        --no-build)     SKIP_BUILD=1 ;;
        --yes|-y)       NO_TTY_PROMPT=1 ;;
        --help|-h)
            grep '^#' "$0" | head -30 | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown flag: $1" >&2
            exit 2
            ;;
    esac
    shift
done

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!  %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31mxx  %s\033[0m\n' "$*"; exit 1; }

# --- 1. toolchain check -----------------------------------------------------

say "Checking host toolchain"

required=(git cmake ninja clang clang++)
missing=()
for tool in "${required[@]}"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        missing+=("$tool")
    fi
done
if [[ ${#missing[@]} -gt 0 ]]; then
    warn "Missing tools: ${missing[*]}"
    case "$(uname -s)" in
        Darwin) warn "Try: brew install cmake ninja llvm git gh" ;;
        Linux)  warn "Try: sudo apt install cmake ninja-build clang git gh" ;;
    esac
    fail "Install the missing tools and rerun"
fi

# Optional but recommended.
for tool in gh lake; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        warn "$tool not found — skipping its features (gh: PR ops; lake: Lean spec build)"
    fi
done

# Host arch sanity. E2E tests JIT-compile to ARM64; an x86_64 host
# falls back to the SUCCEED path.
host_arch="$(uname -m)"
case "$host_arch" in
    arm64|aarch64) say "Host arch: $host_arch (E2E tests will run real JIT)" ;;
    *)             warn "Host arch: $host_arch (E2E tests will skip with SUCCEED, code path still validated)" ;;
esac

# --- 2. Claude skills (opt-in) ---------------------------------------------

if [[ $INSTALL_SKILLS -eq 0 ]] && [[ -t 0 ]] && [[ $NO_TTY_PROMPT -eq 0 ]]; then
    echo
    say "Claude agent skills (obra/superpowers)"
    cat <<EOF

This will install ~14 skills into ~/.claude/skills/, including:
  - test-driven-development
  - using-git-worktrees
  - finishing-a-development-branch
  - systematic-debugging
  - subagent-driven-development
  - dispatching-parallel-agents
  - brainstorming, writing-plans, executing-plans
  - requesting-code-review, receiving-code-review
  - verification-before-completion, writing-skills
  - using-superpowers (master skill)

Skills modify how a Claude agent reasons about tasks. Source:
https://github.com/obra/superpowers (Apache 2.0).

EOF
    read -rp "Install these skills now? [y/N] " ans
    case "$ans" in y|Y|yes) INSTALL_SKILLS=1 ;; esac
fi

if [[ $INSTALL_SKILLS -eq 1 ]]; then
    say "Installing Claude skills from obra/superpowers"
    skills_dir="$HOME/.claude/skills"
    mkdir -p "$skills_dir"
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT
    git -C "$tmpdir" clone --depth 1 https://github.com/obra/superpowers.git
    if [[ -d "$tmpdir/superpowers/skills" ]]; then
        # rsync would be nicer; cp -R is portable.
        cp -R "$tmpdir/superpowers/skills/." "$skills_dir/"
        say "Installed $(ls "$skills_dir" | wc -l | tr -d ' ') skills into $skills_dir"
    else
        warn "Clone succeeded but skills/ directory missing — upstream layout may have changed"
    fi
fi

# --- 3. core/ build configure ----------------------------------------------

if [[ $SKIP_BUILD -eq 0 ]]; then
    say "Configuring core/build (Debug, Ninja)"
    if [[ ! -f core/build/build.ninja ]]; then
        cmake -S core -B core/build -G Ninja -DCMAKE_BUILD_TYPE=Debug
    else
        say "core/build already configured — skipping cmake configure"
    fi

    say "Building core (this is the long step)"
    cmake --build core/build

    say "Running tests (excl. flaky signal_handler*)"
    if core/build/prisma_core_tests --reporter compact "~signal_handler*"; then
        say "All tests passed"
    else
        fail "Tests failed — check core/build/prisma_core_tests output"
    fi
fi

# --- 4. orientation pointer ------------------------------------------------

cat <<EOF

\033[1;32m==>\033[0m Setup complete.

Next steps:
  1. Read CLAUDE.md (root) — code conventions, commit discipline.
  2. Read docs/HANDOFF.md — what the previous agent did and where to continue.
  3. Read docs/SESSION_TRACE.md — full 42-commit traceability of the last session.
  4. Read docs/COORDINATION.md — multi-agent claim protocol.
  5. Pick a hot spot from docs/HANDOFF.md section 5 (REP/string ops, Global CSE,
     LICM, VPTEST/VPBLENDV ymm, x87, return-stack).

To run the test suite again later:
  core/build/prisma_core_tests --reporter compact "~signal_handler*"

To enter sanitizer-instrumented build:
  cmake -S core -B /tmp/prisma-asan -DPRISMA_ENABLE_ASAN=ON \\
      -DPRISMA_ENABLE_UBSAN=ON -G Ninja
  cmake --build /tmp/prisma-asan
  /tmp/prisma-asan/prisma_core_tests --reporter compact "~signal_handler*"

EOF
