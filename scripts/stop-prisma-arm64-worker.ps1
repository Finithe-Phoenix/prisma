#!/usr/bin/env pwsh

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$workerScript = Join-Path $repo "tools\arm64-worker\server.py"
$pidFile = Join-Path $repo "android\app\build\arm64-worker.pid"

if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) {
    Write-Host "No Prisma ARM64 worker PID file exists."
    return
}

$workerId = [int](Get-Content -LiteralPath $pidFile -Raw).Trim()
$process = Get-CimInstance Win32_Process -Filter "ProcessId = $workerId" -ErrorAction SilentlyContinue
if ($process -and $process.Name -eq "python.exe" -and $process.CommandLine.Contains(
    $workerScript,
    [System.StringComparison]::OrdinalIgnoreCase
)) {
    Stop-Process -Id $workerId
    Wait-Process -Id $workerId -Timeout 10 -ErrorAction SilentlyContinue
    Write-Host "Stopped Prisma ARM64 worker process $workerId."
} elseif ($process) {
    throw "PID $workerId does not belong to the Prisma ARM64 worker; refusing to stop it."
}

Remove-Item -LiteralPath $pidFile -Force
