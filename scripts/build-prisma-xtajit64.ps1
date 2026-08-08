[CmdletBinding()]
param(
  [string]$OutputDirectory = "",
  [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$buildRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "android\app\build"))
$manifestPath = Join-Path $repoRoot "shell\Cargo.toml"
$target = "arm64ec-pc-windows-msvc"
$targetDirectory = Join-Path $repoRoot "shell\target"

if (-not $OutputDirectory) {
  $OutputDirectory = Join-Path $buildRoot "prisma-xtajit64"
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$buildPrefix = $buildRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $outputPath.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase)) {
  throw "OutputDirectory must stay under $buildRoot"
}

$installedTargets = & rustup target list --installed
if ($LASTEXITCODE -ne 0 -or $installedTargets -notcontains $target) {
  throw "Missing Rust target '$target'. Install it with: rustup target add $target"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
  throw "Visual Studio Installer's vswhere.exe is required to locate ARM64EC tools."
}
$installationPath = (& $vswhere -latest -products "*" -property installationPath).Trim()
if ($LASTEXITCODE -ne 0 -or -not $installationPath) {
  throw "Visual Studio 2022 Build Tools were not found."
}
$msvcRoot = Get-ChildItem -LiteralPath (Join-Path $installationPath "VC\Tools\MSVC") -Directory |
  Sort-Object Name -Descending |
  Select-Object -First 1
if (-not $msvcRoot) {
  throw "Visual Studio MSVC tools were not found."
}
$arm64Linker = Join-Path $msvcRoot.FullName "bin\Hostx64\arm64\link.exe"
$arm64EcLibraries = Join-Path $msvcRoot.FullName "lib\arm64ec"
if (-not (Test-Path -LiteralPath $arm64Linker -PathType Leaf) -or
    -not (Test-Path -LiteralPath $arm64EcLibraries -PathType Container)) {
  throw "Missing MSVC v143 C++ ARM64/ARM64EC build tools (Visual Studio component Microsoft.VisualStudio.Component.VC.Tools.ARM64; ARM64EC is included in VS 2022 17.4+)."
}

$dumpbin = Join-Path $msvcRoot.FullName "bin\Hostx64\x64\dumpbin.exe"
if (-not (Test-Path -LiteralPath $dumpbin -PathType Leaf)) {
  throw "dumpbin.exe is required for the ARM64EC PE audit."
}

& cargo build --manifest-path $manifestPath -p prisma-xtajit64 --target $target --release --target-dir $targetDirectory
if ($LASTEXITCODE -ne 0) {
  throw "The ARM64EC xtajit64 provider build failed."
}

$builtDll = Join-Path $targetDirectory "$target\release\prisma_xtajit64.dll"
if (-not (Test-Path -LiteralPath $builtDll -PathType Leaf)) {
  throw "Cargo did not produce the expected ARM64EC DLL: $builtDll"
}

$headers = (& $dumpbin /headers $builtDll | Out-String)
if ($LASTEXITCODE -ne 0 -or $headers -notmatch "(?im)^\s*A641 machine \(ARM64EC\)") {
  throw "The generated provider is not an ARM64EC PE (machine 0xA641)."
}
$exportLines = & $dumpbin /exports $builtDll
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not read the generated provider's export table."
}
$requiredExports = @(
  "BTCpu64FlushInstructionCache",
  "BTCpu64IsProcessorFeaturePresent",
  "BTCpu64NotifyMemoryDirty",
  "BTCpu64NotifyReadFile",
  "BeginSimulation",
  "FlushInstructionCacheHeavy",
  "NotifyMapViewOfSection",
  "NotifyMemoryAlloc",
  "NotifyMemoryFree",
  "NotifyMemoryProtect",
  "NotifyUnmapViewOfSection",
  "ProcessInit",
  "ProcessTerm",
  "ResetToConsistentState",
  "ThreadInit",
  "ThreadTerm",
  "UpdateProcessorInformation",
  "ExitToX64",
  "DispatchJump",
  "RetToEntryThunk"
)
$actualExports = @(
  $exportLines | ForEach-Object {
    if ($_ -match '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)(?:\s+=.*)?$') {
      $Matches[1]
    }
  } | Sort-Object -Unique
)
$exportDelta = Compare-Object ($requiredExports | Sort-Object) $actualExports
if ($exportDelta) {
  $details = ($exportDelta | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join "; "
  throw "Generated provider export table differs from the Wine 11.14 contract: $details"
}

$stagingPath = [IO.Path]::GetFullPath((Join-Path $buildRoot "prisma-xtajit64-staging-$PID"))
try {
  New-Item -ItemType Directory -Path $stagingPath -Force | Out-Null
  Copy-Item -LiteralPath $builtDll -Destination (Join-Path $stagingPath "xtajit64.dll")
  $artifact = Join-Path $stagingPath "xtajit64.dll"
  $manifest = [ordered]@{
    schema = "prisma-xtajit64/v1"
    target = $target
    pe_machine = "0xA641"
    sha256 = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    simulation = "not-implemented"
    exports = $requiredExports
  }
  $manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $stagingPath "prisma-xtajit64.json") -Encoding utf8

  if (Test-Path -LiteralPath $outputPath) {
    if (-not $Force) {
      throw "Output already exists: $outputPath. Pass -Force to replace it."
    }
    Remove-Item -LiteralPath $outputPath -Recurse -Force
  }
  New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
  Move-Item -LiteralPath $stagingPath -Destination $outputPath
  Write-Output "Built and verified ARM64EC xtajit64 provider: $outputPath"
} finally {
  if (Test-Path -LiteralPath $stagingPath) {
    Remove-Item -LiteralPath $stagingPath -Recurse -Force
  }
}
