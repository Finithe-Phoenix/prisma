param(
    [string]$Toolchain = "1.95.0-x86_64-pc-windows-msvc",
    [string]$Configuration = "Debug",
    [switch]$SkipFmt,
    [switch]$SkipClippy
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$shellDir = Join-Path $repoRoot "shell"
$coreLibDir = Join-Path $repoRoot "core\build\$Configuration"
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$cargo = Join-Path $cargoBin "cargo.exe"

if (-not (Test-Path $cargo)) {
    throw "cargo.exe not found at $cargo"
}

if (-not (Test-Path $coreLibDir)) {
    throw "C++ $Configuration library dir not found at $coreLibDir. Run: cmake --build core/build --config $Configuration --target prisma_core_tests"
}

$env:PATH = "$cargoBin;$coreLibDir;$env:PATH"
$env:PRISMA_CORE_LIB_DIR = (Resolve-Path $coreLibDir).Path

Push-Location $shellDir
try {
    if (-not $SkipFmt) {
        & $cargo "+$Toolchain" fmt --all --check
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    & $cargo "+$Toolchain" test --workspace
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if (-not $SkipClippy) {
        & $cargo "+$Toolchain" clippy --workspace --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}
finally {
    Pop-Location
}
