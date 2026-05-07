# Download ONNX Runtime 1.22.x for Windows x64
#
# ort 2.0.0-rc.10 requires ONNX Runtime with ORT_API_VERSION=22 (1.22.x).
# Windows ships an incompatible 1.10.0 in System32. This script downloads
# the correct DLL and places it next to the executable.

param(
    [string]$OutputDir = "",
    [string]$Version = "1.22.0"
)

$ErrorActionPreference = "Stop"

# Default: place next to the built executable
if (-not $OutputDir) {
    $OutputDir = Join-Path $PSScriptRoot "target\release"
    if (-not (Test-Path $OutputDir)) {
        $OutputDir = Join-Path $PSScriptRoot "target\debug"
    }
}

$filename = "onnxruntime-win-x64-$Version.zip"
$url = "https://github.com/microsoft/onnxruntime/releases/download/v$Version/$filename"
$downloadPath = Join-Path $env:TEMP $filename
$extractDir = Join-Path $env:TEMP "onnxruntime-$Version"

Write-Host "Downloading ONNX Runtime $Version from GitHub..."
Write-Host "  URL: $url"

# Download
Invoke-WebRequest -Uri $url -OutFile $downloadPath -UseBasicParsing

# Extract
if (Test-Path $extractDir) {
    Remove-Item -Recurse -Force $extractDir
}
Expand-Archive -Path $downloadPath -DestinationPath $extractDir

# Find and copy the DLL
$dllSource = Get-ChildItem -Path $extractDir -Filter "onnxruntime.dll" -Recurse | Select-Object -First 1

if (-not $dllSource) {
    Write-Error "onnxruntime.dll not found in extracted archive"
    exit 1
}

$dllDest = Join-Path $OutputDir "onnxruntime.dll"

# Create output directory if needed
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
}

Copy-Item -Path $dllSource.FullName -Destination $dllDest -Force
Write-Host "Installed: $dllDest"

# Also copy supporting DLLs (if any)
$libDir = Split-Path $dllSource.FullName
Get-ChildItem -Path $libDir -Filter "*.dll" | Where-Object { $_.Name -ne "onnxruntime.dll" } | ForEach-Object {
    $dest = Join-Path $OutputDir $_.Name
    Copy-Item -Path $_.FullName -Destination $dest -Force
    Write-Host "  Also copied: $($_.Name)"
}

# Verify version
$versionInfo = (Get-Item $dllDest).VersionInfo
Write-Host ""
Write-Host "ONNX Runtime DLL installed successfully."
Write-Host "  File version: $($versionInfo.FileVersion)"
Write-Host "  Product version: $($versionInfo.ProductVersion)"
Write-Host ""
Write-Host "Note: The app will load this DLL instead of the system one (1.10.0)."

# Cleanup
Remove-Item -Path $downloadPath -Force
Remove-Item -Path $extractDir -Recurse -Force
