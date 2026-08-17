#!/usr/bin/env pwsh

[CmdletBinding()]
param(
    [int]$Port = 8765,
    [switch]$Rebuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$workerScript = Join-Path $repo "tools\arm64-worker\server.py"
$buildDirectory = Join-Path $repo "android\app\build"
$pidFile = Join-Path $buildDirectory "arm64-worker.pid"
$stdoutLog = Join-Path $buildDirectory "arm64-worker.stdout.log"
$stderrLog = Join-Path $buildDirectory "arm64-worker.stderr.log"
$image = "prisma-arm64-cross"
$volume = "prisma-arm64-target"
$probe = "/target/aarch64-unknown-linux-gnu/debug/prisma-arm64-probe"

function Test-Worker {
    try {
        $response = Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/health" -TimeoutSec 2
        return $response.StatusCode -eq 200 -and $response.Content -eq "ok"
    } catch {
        return $false
    }
}

function Save-WorkerPid {
    $process = Get-CimInstance Win32_Process |
        Where-Object {
            $_.Name -eq "python.exe" -and
            $_.CommandLine -and
            $_.CommandLine.Contains($workerScript, [System.StringComparison]::OrdinalIgnoreCase)
        } |
        Select-Object -First 1
    if ($process) {
        New-Item -ItemType Directory -Path $buildDirectory -Force | Out-Null
        Set-Content -LiteralPath $pidFile -Value $process.ProcessId
    }
}

if (Test-Worker) {
    Save-WorkerPid
    Write-Host "Prisma ARM64 worker is already listening on port $Port."
    return
}

if (-not (Get-Command docker.exe -ErrorAction SilentlyContinue)) {
    throw "Docker Desktop is required for the emulated ARM64 execution worker."
}
if (-not (Test-Path -LiteralPath $workerScript -PathType Leaf)) {
    throw "ARM64 worker script not found: $workerScript"
}

docker info *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Docker Desktop is installed but its engine is not running."
}

$imageExists = docker image inspect $image 2>$null
if ($LASTEXITCODE -ne 0 -or $Rebuild) {
    docker build -f (Join-Path $repo "docker\Dockerfile.arm64-test") -t $image $repo
    if ($LASTEXITCODE -ne 0) { throw "Failed to build $image." }
}

docker volume inspect $volume *> $null
if ($LASTEXITCODE -ne 0) {
    docker volume create $volume | Out-Null
}

docker run --rm --platform linux/arm64 -v "${volume}:/target:ro" debian:bookworm-slim $probe *> $null
$probeExists = $LASTEXITCODE -eq 0
if (-not $probeExists -or $Rebuild) {
    $container = "prisma-arm64-cross-build"
    docker rm -f $container *> $null
    docker run --name $container `
        --mount "type=bind,src=$repo,dst=/workspace" `
        --mount "type=volume,src=$volume,dst=/target" `
        $image `
        bash -lc "CARGO_TARGET_DIR=/target cargo build --manifest-path shell/Cargo.toml -p prisma-android --bin prisma-arm64-probe --target aarch64-unknown-linux-gnu"
    $buildExit = $LASTEXITCODE
    docker rm $container *> $null
    if ($buildExit -ne 0) { throw "Failed to build the ARM64 Prisma probe." }
}

New-Item -ItemType Directory -Path $buildDirectory -Force | Out-Null
$launcher = Get-Command py.exe -ErrorAction Stop
Start-Process `
    -FilePath $launcher.Source `
    -ArgumentList @("-3.12", "`"$workerScript`"") `
    -WorkingDirectory $repo `
    -WindowStyle Hidden `
    -RedirectStandardOutput $stdoutLog `
    -RedirectStandardError $stderrLog | Out-Null

$deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
    Start-Sleep -Milliseconds 250
    if (Test-Worker) { break }
} while ([DateTime]::UtcNow -lt $deadline)
if (-not (Test-Worker)) {
    throw "ARM64 worker did not become healthy. See $stderrLog"
}

Save-WorkerPid

Write-Host "Prisma ARM64 worker is ready on http://127.0.0.1:$Port."
