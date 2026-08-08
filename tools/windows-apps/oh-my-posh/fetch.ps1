[CmdletBinding()]
param(
  [string]$Destination = (Join-Path $PSScriptRoot "artifacts\oh-my-posh.exe"),
  [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$manifestPath = Join-Path $PSScriptRoot "fixture.lock.json"
$fixture = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
$destinationDirectory = Split-Path -Parent $destinationPath
$expectedTag = "v$($fixture.version)"
$expectedSourceUrl = "https://github.com/JanDeDobbeleer/oh-my-posh/releases/download/$expectedTag/$($fixture.filename)"

if ($fixture.tag -ne $expectedTag -or $fixture.source_url -ne $expectedSourceUrl) {
  throw "fixture.lock.json must use the exact versioned Oh My Posh release URL."
}
if ($fixture.source_url -match "(?i)(^|/)latest(/|$)") {
  throw "Refusing to acquire an Oh My Posh artifact through a latest URL."
}
if ($fixture.sha256 -notmatch "^[0-9a-f]{64}$" -or [long]$fixture.size -le 0) {
  throw "fixture.lock.json has an invalid size or SHA-256."
}

function Test-LockedArtifact {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return $false
  }

  $item = Get-Item -LiteralPath $Path
  if ($item.Length -ne [long]$fixture.size) {
    return $false
  }

  $actualHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  return $actualHash -eq $fixture.sha256
}

if ((Test-LockedArtifact -Path $destinationPath) -and -not $Force) {
  Write-Output "Verified cached Oh My Posh $($fixture.version): $destinationPath"
  exit 0
}

if ((Test-Path -LiteralPath $destinationPath) -and -not $Force) {
  throw "The existing artifact does not match fixture.lock.json. Re-run with -Force to replace it."
}

New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null
$temporaryPath = Join-Path $destinationDirectory ".oh-my-posh.$PID.$([guid]::NewGuid().ToString('N')).download"

try {
  Invoke-WebRequest -UseBasicParsing -Uri $fixture.source_url -OutFile $temporaryPath

  if (-not (Test-LockedArtifact -Path $temporaryPath)) {
    throw "Downloaded artifact size or SHA-256 does not match fixture.lock.json."
  }

  Move-Item -LiteralPath $temporaryPath -Destination $destinationPath -Force
  Write-Output "Fetched and verified Oh My Posh $($fixture.version): $destinationPath"
} finally {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Force
  }
}
