[CmdletBinding()]
param(
  [string]$OutputDirectory = "",
  [string]$ToolchainRoot = "",
  [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-PeSectionForRva {
  param(
    [byte[]]$Bytes,
    [int]$PeOffset,
    [uint32]$Rva
  )

  $sectionCount = [BitConverter]::ToUInt16($Bytes, $PeOffset + 6)
  $optionalHeaderSize = [BitConverter]::ToUInt16($Bytes, $PeOffset + 20)
  $sectionTable = $PeOffset + 24 + $optionalHeaderSize
  for ($index = 0; $index -lt $sectionCount; $index++) {
    $header = $sectionTable + (40 * $index)
    $nameLength = 0
    while ($nameLength -lt 8 -and $Bytes[$header + $nameLength] -ne 0) {
      $nameLength++
    }
    $name = [Text.Encoding]::ASCII.GetString($Bytes, $header, $nameLength)
    $virtualSize = [BitConverter]::ToUInt32($Bytes, $header + 8)
    $virtualAddress = [BitConverter]::ToUInt32($Bytes, $header + 12)
    $rawSize = [BitConverter]::ToUInt32($Bytes, $header + 16)
    $rawAddress = [BitConverter]::ToUInt32($Bytes, $header + 20)
    $span = [Math]::Max([uint64]$virtualSize, [uint64]$rawSize)
    if ([uint64]$Rva -ge [uint64]$virtualAddress -and
        [uint64]$Rva -lt ([uint64]$virtualAddress + $span)) {
      return [pscustomobject]@{
        Name = $name
        FileOffset = [int]([uint64]$rawAddress + ([uint64]$Rva - [uint64]$virtualAddress))
        Characteristics = [BitConverter]::ToUInt32($Bytes, $header + 36)
      }
    }
  }
  throw ("RVA 0x{0:X8} does not belong to a PE section." -f $Rva)
}

function Get-AsciiZ {
  param([byte[]]$Bytes, [int]$Offset)

  $end = $Offset
  while ($end -lt $Bytes.Length -and $Bytes[$end] -ne 0) {
    $end++
  }
  if ($end -eq $Bytes.Length) {
    throw "Unterminated ASCII string in PE export directory."
  }
  return [Text.Encoding]::ASCII.GetString($Bytes, $Offset, $end - $Offset)
}

function Set-NativeTransitionExports {
  param([string]$Path, [string[]]$Names)

  $bytes = [IO.File]::ReadAllBytes($Path)
  $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
  $optionalHeader = $peOffset + 24
  if ([BitConverter]::ToUInt16($bytes, $optionalHeader) -ne 0x20b) {
    throw "ARM64EC provider must use a PE32+ optional header."
  }
  $exportRva = [BitConverter]::ToUInt32($bytes, $optionalHeader + 112)
  $exportSection = Get-PeSectionForRva $bytes $peOffset $exportRva
  $exportOffset = $exportSection.FileOffset
  $functionCount = [BitConverter]::ToUInt32($bytes, $exportOffset + 20)
  $nameCount = [BitConverter]::ToUInt32($bytes, $exportOffset + 24)
  $functionsRva = [BitConverter]::ToUInt32($bytes, $exportOffset + 28)
  $namesRva = [BitConverter]::ToUInt32($bytes, $exportOffset + 32)
  $ordinalsRva = [BitConverter]::ToUInt32($bytes, $exportOffset + 36)
  $functionsOffset = (Get-PeSectionForRva $bytes $peOffset $functionsRva).FileOffset
  $namesOffset = (Get-PeSectionForRva $bytes $peOffset $namesRva).FileOffset
  $ordinalsOffset = (Get-PeSectionForRva $bytes $peOffset $ordinalsRva).FileOffset
  $pending = [Collections.Generic.HashSet[string]]::new(
    $Names,
    [StringComparer]::Ordinal
  )
  $entryThunkPrefix = [byte[]](0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x55, 0x5d, 0xe9)

  for ($index = 0; $index -lt $nameCount; $index++) {
    $nameRva = [BitConverter]::ToUInt32($bytes, $namesOffset + (4 * $index))
    $nameOffset = (Get-PeSectionForRva $bytes $peOffset $nameRva).FileOffset
    $name = Get-AsciiZ $bytes $nameOffset
    if (-not $pending.Contains($name)) {
      continue
    }
    $ordinal = [BitConverter]::ToUInt16($bytes, $ordinalsOffset + (2 * $index))
    if ($ordinal -ge $functionCount) {
      throw "Export '$name' has an invalid function ordinal."
    }
    $functionSlot = $functionsOffset + (4 * $ordinal)
    $thunkRva = [BitConverter]::ToUInt32($bytes, $functionSlot)
    $thunkSection = Get-PeSectionForRva $bytes $peOffset $thunkRva
    if ($thunkSection.Name -ne ".hexpthk") {
      if (($thunkSection.Characteristics -band 0x20000000) -eq 0) {
        throw "Export '$name' does not point at executable native code."
      }
      [void]$pending.Remove($name)
      continue
    }
    for ($byteIndex = 0; $byteIndex -lt $entryThunkPrefix.Length; $byteIndex++) {
      if ($bytes[$thunkSection.FileOffset + $byteIndex] -ne $entryThunkPrefix[$byteIndex]) {
        throw "Export '$name' does not use the canonical ARM64EC entry thunk."
      }
    }
    $relativeTarget = [BitConverter]::ToInt32($bytes, $thunkSection.FileOffset + 10)
    $nativeTarget = [int64]$thunkRva + 14 + [int64]$relativeTarget
    if ($nativeTarget -lt 0 -or $nativeTarget -gt [uint32]::MaxValue) {
      throw "Export '$name' resolved outside the PE RVA range."
    }
    $nativeSection = Get-PeSectionForRva $bytes $peOffset ([uint32]$nativeTarget)
    if (($nativeSection.Characteristics -band 0x20000000) -eq 0 -or
        $nativeSection.Name -eq ".hexpthk") {
      throw "Export '$name' did not resolve to native executable ARM64 code."
    }
    [BitConverter]::GetBytes([uint32]$nativeTarget).CopyTo($bytes, $functionSlot)
    [void]$pending.Remove($name)
  }
  if ($pending.Count -ne 0) {
    throw "Missing native transition exports: $($pending -join ', ')"
  }
  [IO.File]::WriteAllBytes($Path, $bytes)
}

function Set-NativeEntrypoint {
  param([string]$Path)

  $bytes = [IO.File]::ReadAllBytes($Path)
  $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
  $optionalHeader = $peOffset + 24
  $entrypointSlot = $optionalHeader + 16
  $entrypointRva = [BitConverter]::ToUInt32($bytes, $entrypointSlot)
  $entrypointSection = Get-PeSectionForRva $bytes $peOffset $entrypointRva
  if ($entrypointSection.Name -ne ".hexpthk") {
    if (($entrypointSection.Characteristics -band 0x20000000) -eq 0) {
      throw "ARM64EC cdylib entrypoint does not point at executable native code."
    }
    return
  }
  $entryThunkPrefix = [byte[]](0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x55, 0x5d, 0xe9)
  for ($byteIndex = 0; $byteIndex -lt $entryThunkPrefix.Length; $byteIndex++) {
    if ($bytes[$entrypointSection.FileOffset + $byteIndex] -ne $entryThunkPrefix[$byteIndex]) {
      throw "ARM64EC cdylib entrypoint does not use the canonical entry thunk."
    }
  }
  $relativeTarget = [BitConverter]::ToInt32($bytes, $entrypointSection.FileOffset + 10)
  $nativeTarget = [int64]$entrypointRva + 14 + [int64]$relativeTarget
  if ($nativeTarget -lt 0 -or $nativeTarget -gt [uint32]::MaxValue) {
    throw "ARM64EC cdylib entrypoint resolved outside the PE RVA range."
  }
  $nativeSection = Get-PeSectionForRva $bytes $peOffset ([uint32]$nativeTarget)
  if (($nativeSection.Characteristics -band 0x20000000) -eq 0 -or
      $nativeSection.Name -eq ".hexpthk") {
    throw "ARM64EC cdylib entrypoint did not resolve to native executable code."
  }
  [BitConverter]::GetBytes([uint32]$nativeTarget).CopyTo($bytes, $entrypointSlot)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

function Disable-PeTlsCallbacks {
  param([string]$Path)

  $bytes = [IO.File]::ReadAllBytes($Path)
  $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
  $optionalHeader = $peOffset + 24
  if ([BitConverter]::ToUInt16($bytes, $optionalHeader) -ne 0x20b) {
    throw "ARM64EC provider must use a PE32+ optional header."
  }
  # PE32+ data directories start at +112; directory 9 describes TLS. Wine
  # invokes generated CRT callbacks while xtajit64 is still in its special
  # early-load path, before the initial thread owns that module's TLS block.
  # Keep the directory, template and index intact so Rust TLS works after Wine
  # allocates the block; suppress only the callback array pointer.
  $tlsDataDirectory = $optionalHeader + 112 + (9 * 8)
  $tlsRva = [BitConverter]::ToUInt32($bytes, $tlsDataDirectory)
  $tlsSize = [BitConverter]::ToUInt32($bytes, $tlsDataDirectory + 4)
  if ($tlsRva -eq 0 -or $tlsSize -lt 40) {
    throw "ARM64EC provider is missing its PE32+ TLS directory."
  }
  $tlsOffset = (Get-PeSectionForRva $bytes $peOffset $tlsRva).FileOffset
  [Array]::Clear($bytes, $tlsOffset + 24, 8)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$buildRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "android\app\build"))
$manifestPath = Join-Path $repoRoot "shell\Cargo.toml"
$target = "arm64ec-pc-windows-msvc"
$targetDirectory = Join-Path $repoRoot "shell\target"
$builtDll = Join-Path $targetDirectory "$target\release\prisma_xtajit64.dll"

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
${previousInclude} = $env:INCLUDE
try {
  $env:CARGO_TARGET_ARM64EC_PC_WINDOWS_MSVC_LINKER = $arm64Linker
  $env:PATH = (Split-Path -Parent $arm64Linker) + [IO.Path]::PathSeparator + $env:PATH
  $env:LIB = @(
    $arm64EcLibraries
    $arm64Libraries
    (Join-Path $sdkVersion.FullName "um\arm64")
    (Join-Path $sdkVersion.FullName "ucrt\arm64")
  ) -join [IO.Path]::PathSeparator
  $sdkIncludeRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Include\$($sdkVersion.Name)"
  $env:INCLUDE = @(
    (Join-Path $msvcRoot.FullName "include")
    (Join-Path $sdkIncludeRoot "ucrt")
    (Join-Path $sdkIncludeRoot "shared")
    (Join-Path $sdkIncludeRoot "um")
    (Join-Path $sdkIncludeRoot "winrt")
    (Join-Path $sdkIncludeRoot "cppwinrt")
  ) -join [IO.Path]::PathSeparator
  # PE post-processing must never become Cargo's cached linker output. Clean
  # only this owned package so Cargo relinks a pristine TLS directory instead
  # of restoring a previously patched DLL from `target/release/deps`.
  & cargo clean --manifest-path $manifestPath -p prisma-xtajit64 --target $target --target-dir $targetDirectory
  if ($LASTEXITCODE -ne 0) {
    throw "Could not invalidate the cached ARM64EC provider artifacts."
  }
  & cargo build --manifest-path $manifestPath -p prisma-xtajit64 --target $target --release --locked --jobs 1 --target-dir $targetDirectory
  if ($LASTEXITCODE -ne 0) {
    throw "The ARM64EC xtajit64 provider build failed."
  }
} finally {
  $env:CARGO_TARGET_ARM64EC_PC_WINDOWS_MSVC_LINKER = ${previousLinker}
  $env:PATH = ${previousPath}
  $env:LIB = ${previousLib}
  $env:INCLUDE = ${previousInclude}
}

if (-not (Test-Path -LiteralPath $builtDll -PathType Leaf)) {
  throw "Cargo did not produce the expected ARM64EC DLL: $builtDll"
}

$requiredProviderExports = @(
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
$rawTransitionExports = @("ExitToX64", "DispatchJump", "RetToEntryThunk")
# Wine redirects normal provider callbacks through the image's ARM64EC
# metadata. Only these three transition pointers are consumed raw and must be
# rewritten from their x64 entry thunks to canonical native destinations.
Set-NativeTransitionExports -Path $builtDll -Names $rawTransitionExports
# Rust's DLL entrypoint thunk is outside LLVM's ARM64X redirection table. Wine
# calls the PE entrypoint before ProcessInit, so publish its native destination.
Set-NativeEntrypoint -Path $builtDll
# Wine loads xtajit64 before allocating the initial thread's PE TLS block.
# Suppress only generated CRT callbacks; preserve the TLS directory itself for
# std dependencies such as TranslationCache's randomized hash state.
Disable-PeTlsCallbacks -Path $builtDll

$peBytes = [IO.File]::ReadAllBytes($builtDll)
if ($peBytes.Length -lt 64 -or $peBytes[0] -ne 0x4d -or $peBytes[1] -ne 0x5a) {
  throw "The generated provider is not a PE file."
}
$peOffset = [BitConverter]::ToInt32($peBytes, 0x3c)
if ($peOffset -lt 0 -or $peOffset + 6 -gt $peBytes.Length -or
    $peBytes[$peOffset] -ne 0x50 -or $peBytes[$peOffset + 1] -ne 0x45 -or
    $peBytes[$peOffset + 2] -ne 0 -or $peBytes[$peOffset + 3] -ne 0) {
  throw "The generated provider has an invalid PE signature."
}
$optionalHeader = $peOffset + 24
$tlsDataDirectory = $optionalHeader + 112 + (9 * 8)
$tlsRva = [BitConverter]::ToUInt32($peBytes, $tlsDataDirectory)
$tlsSize = [BitConverter]::ToUInt32($peBytes, $tlsDataDirectory + 4)
if ($tlsRva -eq 0 -or $tlsSize -lt 40) {
  throw "Post-processed provider lost its PE TLS directory."
}
$tlsOffset = (Get-PeSectionForRva $peBytes $peOffset $tlsRva).FileOffset
$tlsCallbacksVa = [BitConverter]::ToUInt64($peBytes, $tlsOffset + 24)
if ($tlsCallbacksVa -ne 0) {
  throw ("Post-processed provider still has TLS callbacks at VA 0x{0:X16}." -f $tlsCallbacksVa)
}
$machineCode = [BitConverter]::ToUInt16($peBytes, $peOffset + 4)
$peMachine = switch ($machineCode) {
  0xA641 { "0xA641 ARM64EC" }
  0xA64E { "0xA64E ARM64X" }
  0x8664 { "0x8664 ARM64X-hybrid" }
  default { throw ("The generated provider has unsupported PE machine 0x{0:X4}." -f $machineCode) }
}

$headers = (& $dumpbin /headers $builtDll | Out-String)
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not read the generated provider's PE headers."
}
if ($headers -notmatch "(?im)^\s*(A641 machine \(ARM64EC\)|A64E machine \(ARM64X\)|8664 machine \(x64\) \(ARM64X\))") {
  throw "dumpbin did not recognize the generated provider as ARM64EC/ARM64X."
}
$exportLines = & $dumpbin /exports $builtDll
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not read the generated provider's export table."
}
$requiredExports = $requiredProviderExports
$actualExports = @(
  $exportLines | ForEach-Object {
    if ($_ -match '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)(?:\s+=.*)?$') {
      $Matches[1]
    }
  } | Sort-Object -CaseSensitive -Unique
)
$exportDelta = Compare-Object ($requiredExports | Sort-Object -CaseSensitive) $actualExports -CaseSensitive
if ($exportDelta) {
  $details = ($exportDelta | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join "; "
  throw "Generated provider export table differs from the Wine 11.14 contract: $details"
}

$requiredImports = @(
  "API-MS-WIN-CORE-MEMORY-L1-1-6.DLL",
  "API-MS-WIN-CORE-SYNCH-L1-2-0.DLL",
  "BCRYPTPRIMITIVES.DLL",
  "KERNEL32.DLL",
  "NTDLL.DLL",
  "VCRUNTIME140.DLL"
) | Sort-Object
$dependentLines = & $dumpbin /dependents $builtDll
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not read the generated provider's dependencies."
}
$actualImports = @(
  $dependentLines | ForEach-Object {
    if ($_ -match '^\s+([A-Za-z0-9][A-Za-z0-9._-]*\.dll)\s*$') {
      $Matches[1].ToUpperInvariant()
    }
  } | Sort-Object -Unique
)
$importDelta = Compare-Object $requiredImports $actualImports
if ($importDelta) {
  $details = ($importDelta | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join "; "
  throw "Generated provider import table differs from the audited runtime contract: $details"
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
    simulation = "implemented-awaiting-f3-wn-019"
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
