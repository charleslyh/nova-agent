# Build release moray-cli and install to Tauri externalBin (moray-cli-<host-triple>.exe).
# See desktop/client/src-tauri/resources/binaries/README.md
$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root

$hostTriple = (rustc --print host-tuple).Trim()
$destDir = Join-Path $root "desktop/client/src-tauri/resources/binaries"
New-Item -ItemType Directory -Force -Path $destDir | Out-Null

if ($hostTriple -match "windows") {
    $src = Join-Path $root "target/release/moray-cli.exe"
    $dest = Join-Path $destDir "moray-cli-$hostTriple.exe"
} else {
    $src = Join-Path $root "target/release/moray-cli"
    $dest = Join-Path $destDir "moray-cli-$hostTriple"
}

cargo build -p moray-cli --release
Copy-Item -Force -Path $src -Destination $dest
Write-Host "Installed moray-cli sidecar -> $dest"
