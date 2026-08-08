[CmdletBinding()]
param(
  [string]$OutputDirectory = "",
  [string]$ToolchainRoot = "",
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

$toolchainCandidates = [Collections.Generic.List[string]]::new()
if ($ToolchainRoot) {
  $toolchainCandidates.Add([IO.Path]::GetFullPath($ToolchainRoot))
}
if ($env:PRISMA_ARM64EC_TOOLCHAIN_ROOT) {
  $toolchainCandidates.Add([IO.Path]::GetFullPath($env:PRISMA_ARM64EC_TOOLCHAIN_ROOT))
}
$localToolchain = Join-Path $env:SystemDrive "PrismaToolchains\msvc-arm64ec\Contents"
if (Test-Path -LiteralPath $localToolchain -PathType Container) {
  $toolchainCandidates.Add($localToolchain)
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
  $installationPath = (& $vswhere -latest -products "*" -property installationPath).Trim()
  if ($LASTEXITCODE -eq 0 -and $installationPath) {
    $toolchainCandidates.Add($installationPath)
  }
}

$msvcRoot = $null
foreach ($candidate in $toolchainCandidates) {
  $msvcParent = if (Test-Path -LiteralPath (Join-Path $candidate "VC\Tools\MSVC") -PathType Container) {
    Join-Path $candidate "VC\Tools\MSVC"
  } elseif (Test-Path -LiteralPath (Join-Path $candidate "Contents\VC\Tools\MSVC") -PathType Container) {
    Join-Path $candidate "Contents\VC\Tools\MSVC"
  } else {
    $null
  }
  if (-not $msvcParent) {
    continue
  }
  $candidateRoot = Get-ChildItem -LiteralPath $msvcParent -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1
  if (-not $candidateRoot) {
    continue
  }
  $candidateLinker = Join-Path $candidateRoot.FullName "bin\Hostx64\arm64\link.exe"
  $candidateEcLibraries = Join-Path $candidateRoot.FullName "lib\arm64ec"
  $candidateArmLibraries = Join-Path $candidateRoot.FullName "lib\arm64"
  if ((Test-Path -LiteralPath $candidateLinker -PathType Leaf) -and
      (Test-Path -LiteralPath $candidateEcLibraries -PathType Container) -and
      (Test-Path -LiteralPath (Join-Path $candidateArmLibraries "msvcrt.lib") -PathType Leaf)) {
    $msvcRoot = $candidateRoot
    break
  }
}
if (-not $msvcRoot) {
  throw "Missing MSVC v143 C++ ARM64/ARM64EC linker and CRT libraries. Install Microsoft.VisualStudio.Component.VC.Tools.ARM64 or pass -ToolchainRoot with an official local layout extraction."
}

$arm64Linker = Join-Path $msvcRoot.FullName "bin\Hostx64\arm64\link.exe"
$arm64EcLibraries = Join-Path $msvcRoot.FullName "lib\arm64ec"
$arm64Libraries = Join-Path $msvcRoot.FullName "lib\arm64"
$linkerSignature = Get-AuthenticodeSignature -LiteralPath $arm64Linker
if ($linkerSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
    $linkerSignature.SignerCertificate.Subject -notmatch '^CN=Microsoft Corporation,') {
  throw "The ARM64 linker is not validly signed by Microsoft: $arm64Linker"
}
$sdkLibRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Lib"
$sdkVersion = Get-ChildItem -LiteralPath $sdkLibRoot -Directory |
  Where-Object {
    (Test-Path -LiteralPath (Join-Path $_.FullName "um\arm64") -PathType Container) -and
    (Test-Path -LiteralPath (Join-Path $_.FullName "ucrt\arm64") -PathType Container)
  } |
  Sort-Object Name -Descending |
  Select-Object -First 1
if (-not $sdkVersion) {
  throw "The Windows SDK ARM64 import libraries were not found."
}

$dumpbin = @(
  (Join-Path $msvcRoot.FullName "bin\Hostx64\arm64\dumpbin.exe")
  (Join-Path $msvcRoot.FullName "bin\Hostx64\x64\dumpbin.exe")
) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $dumpbin) {
  throw "dumpbin.exe is required for the ARM64EC PE audit."
}

${previousLinker} = $env:CARGO_TARGET_ARM64EC_PC_WINDOWS_MSVC_LINKER
${previousPath} = $env:PATH
${previousLib} = $env:LIB
try {
  $env:CARGO_TARGET_ARM64EC_PC_WINDOWS_MSVC_LINKER = $arm64Linker
  $env:PATH = (Split-Path -Parent $arm64Linker) + [IO.Path]::PathSeparator + $env:PATH
  $env:LIB = @(
    $arm64EcLibraries
    $arm64Libraries
    (Join-Path $sdkVersion.FullName "um\arm64")
    (Join-Path $sdkVersion.FullName "ucrt\arm64")
  ) -join [IO.Path]::PathSeparator
  & cargo build --manifest-path $manifestPath -p prisma-xtajit64 --target $target --release --target-dir $targetDirectory
  if ($LASTEXITCODE -ne 0) {
    throw "The ARM64EC xtajit64 provider build failed."
  }
} finally {
  $env:CARGO_TARGET_ARM64EC_PC_WINDOWS_MSVC_LINKER = ${previousLinker}
  $env:PATH = ${previousPath}
  $env:LIB = ${previousLib}
}

$builtDll = Join-Path $targetDirectory "$target\release\prisma_xtajit64.dll"
if (-not (Test-Path -LiteralPath $builtDll -PathType Leaf)) {
  throw "Cargo did not produce the expected ARM64EC DLL: $builtDll"
}

$headers = (& $dumpbin /headers $builtDll | Out-String)
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not read the generated provider's PE headers."
}
$peMachine = if ($headers -match "(?im)^\s*A641 machine \(ARM64EC\)") {
  "0xA641 ARM64EC"
} elseif ($headers -match "(?im)^\s*A64E machine \(ARM64X\)") {
  "0xA64E ARM64X"
} elseif ($headers -match "(?im)^\s*8664 machine \(x64\) \(ARM64X\)") {
  "0x8664 ARM64X-hybrid"
} else {
  throw "The generated provider is neither ARM64EC nor an ARM64X hybrid PE."
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
$backupPath = $null
try {
  New-Item -ItemType Directory -Path $stagingPath -Force | Out-Null
  Copy-Item -LiteralPath $builtDll -Destination (Join-Path $stagingPath "xtajit64.dll")
  $artifact = Join-Path $stagingPath "xtajit64.dll"
  $manifest = [ordered]@{
    schema = "prisma-xtajit64/v1"
    target = $target
    pe_machine = $peMachine
    sha256 = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    simulation = "not-implemented"
    exports = $requiredExports
  }
  $manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $stagingPath "prisma-xtajit64.json") -Encoding utf8

  if (Test-Path -LiteralPath $outputPath) {
    if (-not $Force) {
      throw "Output already exists: $outputPath. Pass -Force to replace it."
    }
    $backupPath = [IO.Path]::GetFullPath((Join-Path $buildRoot ("prisma-xtajit64-backup-" + [Guid]::NewGuid().ToString("N"))))
    Move-Item -LiteralPath $outputPath -Destination $backupPath
  }
  New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
  try {
    Move-Item -LiteralPath $stagingPath -Destination $outputPath
    if ($backupPath -and (Test-Path -LiteralPath $backupPath)) {
      Remove-Item -LiteralPath $backupPath -Recurse -Force
      $backupPath = $null
    }
  } catch {
    if ($backupPath -and (Test-Path -LiteralPath $backupPath) -and -not (Test-Path -LiteralPath $outputPath)) {
      Move-Item -LiteralPath $backupPath -Destination $outputPath
      $backupPath = $null
    }
    throw
  }
  Write-Output "Built and verified ARM64EC xtajit64 provider: $outputPath"
} finally {
  if (Test-Path -LiteralPath $stagingPath) {
    Remove-Item -LiteralPath $stagingPath -Recurse -Force
  }
}
