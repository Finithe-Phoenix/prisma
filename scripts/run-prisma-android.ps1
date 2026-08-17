#!/usr/bin/env pwsh

[CmdletBinding()]
param(
    [string]$AvdName = "Prisma_Device",
    [string]$EmulatorPath = "C:\Users\daedg\AppData\Local\Android\Sdk\emulator\emulator.exe",
    [string]$AdbPath = "C:\Users\daedg\AppData\Local\Android\Sdk\platform-tools\adb.exe",
    [string]$ApkPath,
    [ValidateSet("software", "host", "auto", "lavapipe", "swiftshader", "swangle")]
    [string]$Gpu = "software",
    [switch]$NoInstall,
    [switch]$NoArm64Worker,
    [switch]$EnableAudio
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ApkPath)) {
    $ApkPath = Join-Path $PSScriptRoot "..\android\app\build\outputs\apk\debug\app-debug.apk"
}

$appComponent = "dev.prismaemu.app/.MainActivity"
$windowX = 100
$windowY = 100
$startedHere = $false
$initialQemuIds = @(
    Get-Process -Name "qemu-system-x86_64" -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty Id
)
$initialAndroidProcessIds = @(
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -in @("emulator", "qemu-system-x86_64") } |
        Select-Object -ExpandProperty Id
)

function Assert-FileExists {
    param(
        [Parameter(Mandatory)]
        [string]$LiteralPath,
        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $LiteralPath -PathType Leaf)) {
        throw "$Description not found: $LiteralPath"
    }
}

function Get-EmulatorSerials {
    $deviceLines = & $AdbPath devices 2>$null
    if ($LASTEXITCODE -ne 0) {
        return @()
    }

    return @(
        $deviceLines |
            ForEach-Object {
                if ($_ -match '^(emulator-\d+)\s+device$') {
                    $Matches[1]
                }
            }
    )
}

function Find-AvdSerial {
    foreach ($serial in Get-EmulatorSerials) {
        $reportedName = @(& $AdbPath -s $serial emu avd name 2>$null) |
            Select-Object -First 1
        if ($reportedName -and $reportedName.Trim() -eq $AvdName) {
            return $serial
        }
    }

    return $null
}

function Find-QemuProcess {
    param([switch]$OnlyNew)

    $processes = @(
        Get-Process -Name "qemu-system-x86_64" -ErrorAction SilentlyContinue |
            Sort-Object StartTime -Descending
    )

    if ($OnlyNew) {
        $processes = @($processes | Where-Object { $initialQemuIds -notcontains $_.Id })
    }

    $titledProcess = $processes |
        Where-Object { $_.MainWindowTitle -like "*$AvdName*" } |
        Select-Object -First 1
    if ($titledProcess) {
        return $titledProcess
    }

    $candidateIds = @(
        Get-CimInstance Win32_Process -Filter "Name = 'qemu-system-x86_64.exe'" |
            Where-Object { $_.CommandLine -and $_.CommandLine -match [regex]::Escape($AvdName) } |
            Sort-Object CreationDate -Descending |
            Select-Object -ExpandProperty ProcessId
    )

    foreach ($candidateId in $candidateIds) {
        if (-not $OnlyNew -or $initialQemuIds -notcontains $candidateId) {
            return Get-Process -Id $candidateId -ErrorAction SilentlyContinue
        }
    }

    return $processes | Select-Object -First 1
}

function Move-EmulatorWindow {
    param(
        [Parameter(Mandatory)]
        [System.Diagnostics.Process]$QemuProcess
    )

    $QemuProcess.Refresh()
    if ($QemuProcess.MainWindowHandle -eq 0) {
        throw "QEMU is running but its Qt window has not been created yet."
    }

    [void][PrismaEmulatorWindow]::ShowWindow($QemuProcess.MainWindowHandle, 9)
    $moved = [PrismaEmulatorWindow]::SetWindowPos(
        $QemuProcess.MainWindowHandle,
        [IntPtr]::Zero,
        $windowX,
        $windowY,
        0,
        0,
        0x0045
    )
    if (-not $moved) {
        throw "Windows refused to reposition the Android Emulator window."
    }

    [void][PrismaEmulatorWindow]::SetForegroundWindow($QemuProcess.MainWindowHandle)
}

function Wait-Until {
    param(
        [Parameter(Mandatory)]
        [scriptblock]$Condition,
        [Parameter(Mandatory)]
        [int]$TimeoutSeconds,
        [Parameter(Mandatory)]
        [string]$FailureMessage
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $result = & $Condition
        if ($result) {
            return $result
        }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)

    throw $FailureMessage
}

Assert-FileExists -LiteralPath $EmulatorPath -Description "Android Emulator"
Assert-FileExists -LiteralPath $AdbPath -Description "ADB"
if (-not $NoInstall) {
    Assert-FileExists -LiteralPath $ApkPath -Description "Prisma debug APK"
    $ApkPath = (Resolve-Path -LiteralPath $ApkPath).Path
}
if (-not $NoArm64Worker) {
    & (Join-Path $PSScriptRoot "start-prisma-arm64-worker.ps1")
}

if (-not ("PrismaEmulatorWindow" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class PrismaEmulatorWindow
{
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(
        IntPtr hWnd,
        IntPtr hWndInsertAfter,
        int x,
        int y,
        int cx,
        int cy,
        uint flags);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
}

try {
    $serial = Find-AvdSerial
    $qemuProcess = $null

    if ($serial) {
        Write-Host "Reusing $AvdName on $serial."
        $qemuProcess = Wait-Until `
            -Condition { Find-QemuProcess } `
            -TimeoutSeconds 30 `
            -FailureMessage "Could not locate the QEMU process for $AvdName."
    } else {
        $arguments = @(
            "-avd", $AvdName,
            "-gpu", $Gpu,
            "-feature", "-Vulkan",
            "-no-snapshot"
        )
        if (-not $EnableAudio) {
            $arguments += "-no-audio"
        }

        Write-Host "Starting $AvdName with the $Gpu graphics backend."
        Start-Process -FilePath $EmulatorPath -ArgumentList $arguments | Out-Null
        $startedHere = $true

        $qemuProcess = Wait-Until `
            -Condition {
                $candidate = Find-QemuProcess -OnlyNew
                if ($candidate) {
                    $candidate.Refresh()
                    if ($candidate.MainWindowHandle -ne 0) {
                        $candidate
                    }
                }
            } `
            -TimeoutSeconds 90 `
            -FailureMessage "Android Emulator did not create a Qt window within 90 seconds."
    }

    Move-EmulatorWindow -QemuProcess $qemuProcess

    $serial = Wait-Until `
        -Condition { Find-AvdSerial } `
        -TimeoutSeconds 120 `
        -FailureMessage "$AvdName did not register with ADB within 120 seconds."

    Write-Host "Waiting for Android to finish booting on $serial."
    Wait-Until `
        -Condition {
            $bootCompleted = @(& $AdbPath -s $serial shell getprop sys.boot_completed 2>$null) |
                Select-Object -First 1
            $bootCompleted -and $bootCompleted.Trim() -eq "1"
        } `
        -TimeoutSeconds 180 `
        -FailureMessage "Android did not finish booting within 180 seconds." | Out-Null

    if (-not $NoInstall) {
        Write-Host "Installing $ApkPath."
        $installOutput = & $AdbPath -s $serial install -r -t $ApkPath 2>&1
        if ($LASTEXITCODE -ne 0 -or $installOutput -notcontains "Success") {
            throw "APK installation failed:`n$($installOutput -join [Environment]::NewLine)"
        }
    }

    & $AdbPath -s $serial shell am force-stop dev.prismaemu.app | Out-Null
    $launchOutput = & $AdbPath -s $serial shell am start -W -n $appComponent 2>&1
    if ($LASTEXITCODE -ne 0 -or $launchOutput -match "Error") {
        throw "Prisma failed to launch:`n$($launchOutput -join [Environment]::NewLine)"
    }

    Move-EmulatorWindow -QemuProcess $qemuProcess
    Write-Host "Prisma is ready on $serial. The emulator window is at ($windowX,$windowY)."
} catch {
    if ($startedHere) {
        $serial = Find-AvdSerial
        if ($serial) {
            & $AdbPath -s $serial emu kill 2>$null | Out-Null
        }

        Get-Process -Name "qemu-system-x86_64", "emulator" -ErrorAction SilentlyContinue |
            Where-Object { $initialAndroidProcessIds -notcontains $_.Id } |
            Stop-Process -Force -ErrorAction SilentlyContinue
    }

    throw
}
