# External code review for a Prisma diff, using the Codex (and, when available,
# Gemini) CLIs as a second set of eyes. Records verdicts under docs/REVIEWS-tmp/
# so they can be triaged before merge. See docs/CONTRIBUTING.md.
#
# Usage:
#   pwsh -File scripts/review-with-codex.ps1 [-DiffRange origin/main...HEAD] [-OutDir <dir>] [-WithGemini]
#
# Notes:
# - The diff is embedded inline in the prompt (avoids CLI sandbox file access).
# - The prompt file is written UTF-8 (PowerShell `>` would emit UTF-16).
# - Gemini is OFF by default: as of 2026-06-19 the individual-tier CLI returns
#   IneligibleTierError. Pass -WithGemini once it is re-authenticated.
# - Triage findings against the C++ reference and the byte/functional gates
#   before acting; reviewers produce occasional false positives (e.g. IR ref
#   numbering does NOT affect smoke_differential, which is functional-parity).
param(
    [string]$DiffRange = "origin/main...HEAD",
    [string]$OutDir = "",
    [switch]$WithGemini
)
$ErrorActionPreference = "Continue"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
if ([string]::IsNullOrEmpty($OutDir)) {
    $OutDir = Join-Path $env:TEMP "prisma-review"
}
New-Item -ItemType Directory -Force $OutDir | Out-Null

$checklist = @"
You are an expert reviewer of Prisma, a Rust x86-64 -> ARM64 dynamic binary
translator. Review the diff below. Focus ONLY on:
1. Correctness vs x86-64 semantics (operand direction, flag effects, sign/zero
   extension, edge cases: zero source, aliasing, carry).
2. Faithfulness to the C++ reference in core/src/decoder/x86_decoder.cpp — the
   Rust decoder must match it for the FFI differential.
3. IR-construction bugs. In THIS codebase: CmpFlags REQUIRES a result ref;
   Select reads NZCV by condition code and must follow its CmpFlags. The
   smoke_differential test compares FUNCTIONAL parity (both translate, same
   exit_kind, caching), NOT IR ref numbers, so ref-numbering differences are not
   a differential bug.
4. Missing test coverage, especially the aarch64-gated execution e2e.
Output a verdict: APPROVE or REQUEST-CHANGES, then a numbered list of concrete
findings (file:line + why + suggested fix). Flag uncertain items as QUESTION,
not blocker. Be concise; do not restate the diff.

=== DIFF ($DiffRange) ===
"@

Push-Location $repoRoot
try {
    $diff = git diff $DiffRange
} finally {
    Pop-Location
}
$prompt = $checklist + "`n" + ($diff -join "`n")
$promptPath = Join-Path $OutDir "prompt.txt"
[System.IO.File]::WriteAllText($promptPath, $prompt, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Prompt: $promptPath ($((Get-Item $promptPath).Length) bytes)"

Write-Host "=== codex (read-only sandbox) ==="
Get-Content $promptPath -Raw -Encoding UTF8 |
    codex exec --sandbox read-only - 2>&1 |
    Tee-Object (Join-Path $OutDir "codex-verdict.txt")

if ($WithGemini) {
    $env:GEMINI_CLI_TRUST_WORKSPACE = 'true'
    Write-Host "=== gemini ==="
    Get-Content $promptPath -Raw -Encoding UTF8 |
        gemini 2>&1 |
        Tee-Object (Join-Path $OutDir "gemini-verdict.txt")
}

Write-Host "=== Verdicts in $OutDir ==="
