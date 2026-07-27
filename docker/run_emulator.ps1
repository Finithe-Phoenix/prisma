#!/usr/bin/env pwsh

Write-Host "Building Android Emulator Docker image..."
docker build -t android-emulator -f docker/Dockerfile.android .

Write-Host "Stopping and removing existing container if it exists..."
docker stop android-emulator-container 2>
docker rm android-emulator-container 2>

Write-Host "Starting Android Emulator container..."
# Run the emulator in detached mode, mapping port 5555
docker run -d -p 5555:5555 --name android-emulator-container android-emulator

Write-Host "Container started."
Write-Host "Emulator is booting up. It may take a few minutes."
Write-Host "To connect, run: adb connect localhost:5555"
