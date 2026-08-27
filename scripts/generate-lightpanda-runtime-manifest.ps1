#Requires -Version 5.1
<#
.SYNOPSIS
Generates src-tauri/resources/lightpanda-runtime-manifest.json atomically.

.DESCRIPTION
The Lightpanda URL/SHA-256 are fixed to the reviewed 0.3.7 values. The rootfs
URL and SHA-256 must come from the published immutable runtime release
(workflow lightpanda-runtime.yml). Refuses placeholder or mutable inputs.
Writes to a temp file first, then moves atomically.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DistroImageUrl,

    [Parameter(Mandatory = $true)]
    [string]$DistroImageSha256,

    [string]$RepoRoot = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $RepoRoot = Split-Path -Parent $scriptDir
}

# Pinned, reviewed values. Changing these requires a security review.
$LightpandaVersion = "0.3.7"
$LightpandaArch = "x86_64-linux"
$LightpandaAssetUrl = "https://github.com/lightpanda-io/browser/releases/download/0.3.7/lightpanda-x86_64-linux"
$LightpandaSha256 = "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a"
$DistroImageVersion = "lwsl-ubuntu-noble-minbase-v1"
$DistroArch = "amd64"

function Stop-Bad([string]$message) {
    Write-Error $message
    exit 1
}

if ($DistroImageUrl -match '(<[^>]*>|\bTODO\b|\bPLACEHOLDER\b|nightly|/latest|latest/download)') {
    Stop-Bad "DistroImageUrl must be an immutable published URL, got: $DistroImageUrl"
}
if ($DistroImageUrl -notmatch '^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/releases/download/[^/]+/[^/]+$') {
    Stop-Bad "DistroImageUrl must be a pinned GitHub release asset URL, got: $DistroImageUrl"
}
$tag = ($DistroImageUrl -split '/releases/download/')[1].Split('/')[0]
if ($tag -cne "runtime-$DistroImageVersion") {
    Stop-Bad "DistroImageUrl release tag '$tag' must be 'runtime-$DistroImageVersion'"
}
if ($DistroImageSha256 -notmatch '^[0-9a-f]{64}$') {
    Stop-Bad "DistroImageSha256 must be 64 lowercase hex characters, got: $DistroImageSha256"
}

$manifest = [ordered]@{
    schema_version      = 1
    runtime_family      = "lightpanda-wsl"
    lightpanda_version  = $LightpandaVersion
    lightpanda_arch     = $LightpandaArch
    lightpanda_asset_url = $LightpandaAssetUrl
    lightpanda_sha256   = $LightpandaSha256
    distro_image_version = $DistroImageVersion
    distro_arch         = $DistroArch
    distro_image_url    = $DistroImageUrl
    distro_image_sha256 = $DistroImageSha256
}

$json = $manifest | ConvertTo-Json -Depth 4
# Normalize to 2-space indentation for stable diffs (ConvertTo-Json uses 4).
$lines = $json -split "`r?`n"
$out = New-Object System.Collections.Generic.List[string]
foreach ($line in $lines) {
    if ($line -match '^(\s+)') {
        $indent = [math]::Floor($Matches[1].Length / 4)
        $out.Add((New-Object string ' ', ($indent * 2)) + $line.TrimStart())
    } else {
        $out.Add($line)
    }
}
$content = ($out -join "`n") + "`n"

$resourcesDir = Join-Path $RepoRoot "src-tauri/resources"
if (-not (Test-Path $resourcesDir)) { New-Item -ItemType Directory -Path $resourcesDir | Out-Null }
$target = Join-Path $resourcesDir "lightpanda-runtime-manifest.json"
$tmp = Join-Path $resourcesDir ("lightpanda-runtime-manifest.json.tmp-" + [guid]::NewGuid().ToString("N"))
# UTF-8 without BOM: serde_json rejects a leading BOM.
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($tmp, $content, $utf8NoBom)
Move-Item -Force -LiteralPath $tmp -Destination $target

Write-Host "Wrote $target"
Write-Host "  lightpanda: $LightpandaVersion ($LightpandaSha256)"
Write-Host "  rootfs:     $DistroImageVersion ($DistroImageSha256)"
Write-Host "  url:        $DistroImageUrl"
