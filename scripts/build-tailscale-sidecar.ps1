$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$sidecar = Join-Path $repo "sidecars\tailscale-funnel"
$out = Join-Path $repo "src-tauri\binaries\tailscale-funnel-sidecar-x86_64-pc-windows-msvc.exe"

Push-Location $sidecar
try {
    go test ./...
    if ($LASTEXITCODE -ne 0) { throw "go test failed" }
    go build -trimpath -ldflags="-s -w" -o $out .
    if ($LASTEXITCODE -ne 0) { throw "go build failed" }
} finally {
    Pop-Location
}

if (!(Test-Path $out)) { throw "Tailscale sidecar build output missing: $out" }
Write-Host "Built: $out"
