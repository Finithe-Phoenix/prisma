[CmdletBinding()]
param(
    [string]$OutputDirectory = "",
    [ValidateRange(1, 32)]
    [int]$Jobs = 4,
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$buildRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "android\app\build"))
$wineRoot = Join-Path $repoRoot "third_party\wine"
$dockerfile = Join-Path $repoRoot "docker\Dockerfile.wine-arm64"
$noPreloadReservePatch = Join-Path $repoRoot "third_party\wine-prisma\patches\0001-prisma-no-preload-reserve.patch"
$wineVersion = (Get-Content -LiteralPath (Join-Path $wineRoot "VERSION") -Raw).Trim() -replace '^Wine version ', ''
$wineCommit = (git -C $wineRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or -not $wineCommit) {
    throw "Unable to resolve the Wine submodule commit."
}

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $buildRoot "wine-arm64\$wineVersion"
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$buildPrefix = $buildRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $outputPath.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputDirectory must stay under $buildRoot"
}

function Get-ElfMachine {
    param([Parameter(Mandatory)][string]$Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 20 -or $bytes[0] -ne 0x7f -or $bytes[1] -ne 0x45 -or
        $bytes[2] -ne 0x4c -or $bytes[3] -ne 0x46 -or $bytes[5] -ne 1) {
        throw "Not a supported little-endian ELF file: $Path"
    }
    return [BitConverter]::ToUInt16($bytes, 18)
}

function Get-PeMachine {
    param([Parameter(Mandatory)][string]$Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "Not a PE file: $Path"
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 0 -or $peOffset + 6 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or $bytes[$peOffset + 3] -ne 0) {
        throw "Invalid PE signature: $Path"
    }
    return [BitConverter]::ToUInt16($bytes, $peOffset + 4)
}

function Test-PeArm64XHybrid {
    param([Parameter(Mandatory)][string]$Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        return $false
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 0 -or $peOffset + 24 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or $bytes[$peOffset + 3] -ne 0) {
        return $false
    }

    $sectionCount = [BitConverter]::ToUInt16($bytes, $peOffset + 6)
    $optionalHeaderSize = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
    $optionalHeader = $peOffset + 24
    $sectionTable = $optionalHeader + $optionalHeaderSize
    if ($optionalHeaderSize -lt 200 -or $sectionTable + (40 * $sectionCount) -gt $bytes.Length -or
        [BitConverter]::ToUInt16($bytes, $optionalHeader) -ne 0x20b -or
        [BitConverter]::ToUInt32($bytes, $optionalHeader + 108) -le 10) {
        return $false
    }

    $imageBase = [BitConverter]::ToUInt64($bytes, $optionalHeader + 24)
    $imageSize = [BitConverter]::ToUInt32($bytes, $optionalHeader + 56)
    $loadConfigRva = [BitConverter]::ToUInt32($bytes, $optionalHeader + 192)
    $loadConfigSize = [BitConverter]::ToUInt32($bytes, $optionalHeader + 196)
    if ($loadConfigRva -eq 0 -or $loadConfigSize -le 0xc8) {
        return $false
    }

    $loadConfigOffset = $null
    for ($sectionIndex = 0; $sectionIndex -lt $sectionCount; $sectionIndex++) {
        $section = $sectionTable + (40 * $sectionIndex)
        $virtualSize = [BitConverter]::ToUInt32($bytes, $section + 8)
        $virtualAddress = [BitConverter]::ToUInt32($bytes, $section + 12)
        $rawSize = [BitConverter]::ToUInt32($bytes, $section + 16)
        $rawOffset = [BitConverter]::ToUInt32($bytes, $section + 20)
        $mappedSize = [Math]::Max([uint64]$virtualSize, [uint64]$rawSize)
        if ([uint64]$loadConfigRva -ge [uint64]$virtualAddress -and
            ([uint64]$loadConfigRva - [uint64]$virtualAddress) -lt $mappedSize) {
            $candidate = [uint64]$rawOffset + ([uint64]$loadConfigRva - [uint64]$virtualAddress)
            if ($candidate + 0xd0 -gt [uint64]$bytes.Length) {
                return $false
            }
            $loadConfigOffset = [int64]$candidate
            break
        }
    }
    if ($null -eq $loadConfigOffset) {
        return $false
    }

    $declaredSize = [BitConverter]::ToUInt32($bytes, $loadConfigOffset)
    if ($declaredSize -le 0xc8) {
        return $false
    }
    $chpeMetadataPointer = [BitConverter]::ToUInt64($bytes, $loadConfigOffset + 0xc8)
    return $chpeMetadataPointer -gt $imageBase -and
        $chpeMetadataPointer -lt ($imageBase + [uint64]$imageSize)
}

function Assert-WineArtifact {
    param(
        [Parameter(Mandatory)][string]$Root,
        [switch]$RequireManifest
    )

    $wineBinary = Join-Path $Root "opt\prisma-wine\bin\wine"
    $xtaJit = Join-Path $Root "opt\prisma-wine\lib\wine\aarch64-windows\xtajit64.dll"
    if (-not (Test-Path -LiteralPath $wineBinary -PathType Leaf) -or
        -not (Test-Path -LiteralPath $xtaJit -PathType Leaf)) {
        throw "The artifact must contain bin/wine and aarch64-windows/xtajit64.dll."
    }
    if ((Get-ElfMachine -Path $wineBinary) -ne 183) {
        throw "The Wine loader is not an AArch64 ELF."
    }
    $peMachine = Get-PeMachine -Path $xtaJit
    $peMachineLabel = switch ($peMachine) {
        0xA641 { "0xA641 ARM64EC" }
        0xA64E { "0xA64E ARM64X" }
        0x8664 {
            if (-not (Test-PeArm64XHybrid -Path $xtaJit)) {
                throw "xtajit64.dll has a plain AMD64 header without valid ARM64X CHPE metadata."
            }
            "0x8664 ARM64X-hybrid"
        }
        default {
            throw ("xtajit64.dll is not ARM64EC/ARM64X (machine 0x{0:X4})." -f $peMachine)
        }
    }

    if ($RequireManifest) {
        $manifestPath = Join-Path $Root "prisma-wine-build.json"
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "Missing Prisma Wine artifact manifest."
        }
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if ($manifest.schema -ne "prisma-wine-arm64/v2" -or
            $manifest.wine_version -ne $wineVersion -or
            $manifest.wine_commit -ne $wineCommit -or
            $manifest.platform -ne "linux/arm64" -or
            $manifest.provider_kind -ne "wine-baseline-unsupported-simulation" -or
            $manifest.xtajit64_machine -ne $peMachineLabel -or
            $manifest.ready_for_x64_execution -ne $false) {
            throw "The Wine artifact manifest is incompatible with this build."
        }
        $wineHash = (Get-FileHash -LiteralPath $wineBinary -Algorithm SHA256).Hash.ToLowerInvariant()
        $xtaJitHash = (Get-FileHash -LiteralPath $xtaJit -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($manifest.wine_sha256 -ne $wineHash -or $manifest.xtajit64_sha256 -ne $xtaJitHash) {
            throw "The Wine artifact hash verification failed."
        }
    }

    return [pscustomobject]@{
        Wine = $wineBinary
        XtaJit = $xtaJit
        XtaJitMachine = $peMachineLabel
    }
}

New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null
$lockPath = Join-Path $buildRoot "prisma-wine-arm64.lock"
$lock = $null
$temporaryRoot = $null
$backupPath = $null
$backupMarkerPath = $null
$builderName = $null
try {
    $lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)

    Get-ChildItem -LiteralPath $buildRoot -Directory -Filter "prisma-wine-arm64-temp-*" |
        ForEach-Object {
            $marker = Join-Path $_.FullName ".prisma-wine-temp"
            if ((Test-Path -LiteralPath $marker -PathType Leaf) -and
                (Get-Content -LiteralPath $marker -Raw).Trim() -eq "prisma-wine-arm64-temp/v1") {
                Remove-Item -LiteralPath $_.FullName -Recurse -Force
            }
        }

    Get-ChildItem -LiteralPath $buildRoot -Directory |
        Where-Object { $_.Name -match '^wine-arm64-(context|staging)-(?<ownerPid>[0-9]+)$' } |
        ForEach-Object {
            $legacyPath = [IO.Path]::GetFullPath($_.FullName)
            $legacyPid = [int]$Matches.ownerPid
            $ownerAlive = $null -ne (Get-Process -Id $legacyPid -ErrorAction SilentlyContinue)
            if (-not $ownerAlive -and
                $legacyPath.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                Remove-Item -LiteralPath $legacyPath -Recurse -Force
            }
        }

    Get-ChildItem -LiteralPath $buildRoot -File -Filter "prisma-wine-arm64-backup-*.recovery.json" |
        ForEach-Object {
            $recoveryMarker = $_
            $recovery = Get-Content -LiteralPath $recoveryMarker.FullName -Raw | ConvertFrom-Json
            if ($recovery.schema -ne "prisma-wine-arm64-recovery/v1") {
                throw "Unknown Wine recovery marker: $($recoveryMarker.FullName)"
            }
            $recoveryOutput = [IO.Path]::GetFullPath([string]$recovery.output_path)
            $recoveryBackup = [IO.Path]::GetFullPath([string]$recovery.backup_path)
            if (-not $recoveryOutput.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                -not $recoveryBackup.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase) -or
                (Split-Path -Leaf $recoveryBackup) -notmatch '^prisma-wine-arm64-backup-[0-9a-f]{32}$') {
                throw "Unsafe Wine recovery marker: $($recoveryMarker.FullName)"
            }
            if (Test-Path -LiteralPath $recoveryBackup -PathType Container) {
                if (Test-Path -LiteralPath $recoveryOutput) {
                    $recoveryOutputIsValid = $false
                    try {
                        Assert-WineArtifact -Root $recoveryOutput -RequireManifest | Out-Null
                        $recoveryOutputIsValid = $true
                    }
                    catch {}
                    if ($recoveryOutputIsValid) {
                        Remove-Item -LiteralPath $recoveryBackup -Recurse -Force
                    }
                    else {
                        Remove-Item -LiteralPath $recoveryOutput -Recurse -Force
                        Move-Item -LiteralPath $recoveryBackup -Destination $recoveryOutput
                    }
                }
                else {
                    Move-Item -LiteralPath $recoveryBackup -Destination $recoveryOutput
                }
            }
            Remove-Item -LiteralPath $recoveryMarker.FullName -Force
        }

    $manifestPath = Join-Path $outputPath "prisma-wine-build.json"
    if ((Test-Path -LiteralPath $manifestPath) -and -not $Force) {
        Assert-WineArtifact -Root $outputPath -RequireManifest | Out-Null
        Write-Host "Wine ARM64 artifact already verified at $outputPath"
        exit 0
    }
    if ((Test-Path -LiteralPath $outputPath) -and -not $Force) {
        throw "An unverified or incompatible Wine artifact exists at $outputPath; pass -Force to replace it."
    }

    $temporaryRoot = [IO.Path]::GetFullPath((Join-Path $buildRoot ("prisma-wine-arm64-temp-" + [Guid]::NewGuid().ToString("N"))))
    if (-not $temporaryRoot.StartsWith($buildPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing temporary path outside the Android build directory: $temporaryRoot"
    }
    New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
    [IO.File]::WriteAllText((Join-Path $temporaryRoot ".prisma-wine-temp"), "prisma-wine-arm64-temp/v1`n")
    $contextPath = Join-Path $temporaryRoot "context"
    $stagingPath = Join-Path $temporaryRoot "staging"
    New-Item -ItemType Directory -Path $contextPath | Out-Null

    $archivePath = Join-Path $contextPath "wine-source.tar.gz"
    git -C $wineRoot archive --format=tar.gz --output=$archivePath HEAD
    if ($LASTEXITCODE -ne 0) {
        throw "git archive failed."
    }
    $textFiles = @(git -C $wineRoot grep -Il -e "." --)
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to enumerate Wine text files for CRLF normalization."
    }
    [IO.File]::WriteAllText(
        (Join-Path $contextPath "wine-text-files.txt"),
        ([string]::Join("`n", $textFiles) + "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    Copy-Item -LiteralPath $dockerfile -Destination (Join-Path $contextPath "Dockerfile")
    Copy-Item -LiteralPath $noPreloadReservePatch -Destination (Join-Path $contextPath "wine-prisma-no-preload-reserve.patch")

    $builderName = "prisma-wine-" + [Guid]::NewGuid().ToString("N")
    & docker buildx create --name $builderName --driver docker-container | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to create the isolated Prisma Wine BuildKit builder."
    }
    & docker buildx inspect --bootstrap $builderName | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to bootstrap the isolated Prisma Wine BuildKit builder."
    }

    $dockerArguments = @(
        "buildx", "build",
        "--builder", $builderName,
        "--platform", "linux/arm64",
        "--file", (Join-Path $contextPath "Dockerfile"),
        "--build-arg", "WINE_VERSION=$wineVersion",
        "--build-arg", "WINE_JOBS=$Jobs",
        "--output", "type=local,dest=$stagingPath",
        $contextPath
    )
    & docker @dockerArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Wine ARM64 Docker build failed."
    }

    $artifact = Assert-WineArtifact -Root $stagingPath
    $manifest = [ordered]@{
        schema = "prisma-wine-arm64/v2"
        wine_version = $wineVersion
        wine_commit = $wineCommit
        platform = "linux/arm64"
        wine_sha256 = (Get-FileHash -LiteralPath $artifact.Wine -Algorithm SHA256).Hash.ToLowerInvariant()
        xtajit64_sha256 = (Get-FileHash -LiteralPath $artifact.XtaJit -Algorithm SHA256).Hash.ToLowerInvariant()
        xtajit64_machine = $artifact.XtaJitMachine
        provider_kind = "wine-baseline-unsupported-simulation"
        ready_for_x64_execution = $false
    }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stagingPath "prisma-wine-build.json") -Encoding utf8
    Assert-WineArtifact -Root $stagingPath -RequireManifest | Out-Null

    if (Test-Path -LiteralPath $outputPath) {
        $backupId = [Guid]::NewGuid().ToString("N")
        $backupPath = [IO.Path]::GetFullPath((Join-Path $buildRoot "prisma-wine-arm64-backup-$backupId"))
        $backupMarkerPath = [IO.Path]::GetFullPath((Join-Path $buildRoot "prisma-wine-arm64-backup-$backupId.recovery.json"))
        [ordered]@{
            schema = "prisma-wine-arm64-recovery/v1"
            output_path = $outputPath
            backup_path = $backupPath
        } | ConvertTo-Json | Set-Content -LiteralPath $backupMarkerPath -Encoding utf8
        Move-Item -LiteralPath $outputPath -Destination $backupPath
    }
    try {
        New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
        Move-Item -LiteralPath $stagingPath -Destination $outputPath
        Assert-WineArtifact -Root $outputPath -RequireManifest | Out-Null
    }
    catch {
        if (Test-Path -LiteralPath $outputPath) {
            Remove-Item -LiteralPath $outputPath -Recurse -Force
        }
        if ($backupPath -and (Test-Path -LiteralPath $backupPath)) {
            Move-Item -LiteralPath $backupPath -Destination $outputPath
            $backupPath = $null
        }
        if ($backupMarkerPath -and (Test-Path -LiteralPath $backupMarkerPath)) {
            Remove-Item -LiteralPath $backupMarkerPath -Force
            $backupMarkerPath = $null
        }
        throw
    }
    if ($backupPath -and (Test-Path -LiteralPath $backupPath)) {
        Remove-Item -LiteralPath $backupPath -Recurse -Force
        $backupPath = $null
    }
    if ($backupMarkerPath -and (Test-Path -LiteralPath $backupMarkerPath)) {
        Remove-Item -LiteralPath $backupMarkerPath -Force
        $backupMarkerPath = $null
    }
    Write-Host "Wine ARM64 $wineVersion built and verified at $outputPath"
}
finally {
    if ($builderName) {
        & docker buildx rm --force $builderName 2>$null | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "Unable to remove isolated BuildKit builder $builderName."
        }
    }
    if ($temporaryRoot -and (Test-Path -LiteralPath $temporaryRoot)) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
    if ($lock) {
        $lock.Dispose()
    }
}
