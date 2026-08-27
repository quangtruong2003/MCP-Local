#Requires -Version 5.1
<#
.SYNOPSIS
Verifies the pinned Lightpanda WSL runtime manifest and its companion artifacts.

.DESCRIPTION
Fail-closed verifier for src-tauri/resources/lightpanda-runtime-manifest.json.
Rejects placeholder values, floating/nightly versions, mutable rootfs URLs,
wrong Lightpanda digest/arch, and missing package list/SBOM artifacts.

Exit code 0 = manifest valid; non-zero = invalid (reasons printed to stderr).
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    # $PSScriptRoot is not populated in param defaults on Windows PowerShell 5.1.
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $RepoRoot = Split-Path -Parent $scriptDir
}

# Pinned, reviewed values. Changing these requires a security review.
$ExpectedLightpandaVersion = "0.3.7"
$ExpectedLightpandaArch = "x86_64-linux"
$ExpectedLightpandaUrl = "https://github.com/lightpanda-io/browser/releases/download/0.3.7/lightpanda-x86_64-linux"
$ExpectedLightpandaSha256 = "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a"
$ExpectedDistroImageVersion = "lwsl-ubuntu-noble-minbase-v1"
$ExpectedDistroArch = "amd64"

$errors = New-Object System.Collections.Generic.List[string]

function Add-Error([string]$message) {
    $errors.Add($message)
}

$manifestPath = Join-Path $RepoRoot "src-tauri/resources/lightpanda-runtime-manifest.json"
if (-not (Test-Path $manifestPath)) {
    Write-Error "FAIL: runtime manifest not found at $manifestPath"
    exit 1
}

try {
    $raw = Get-Content -Raw -LiteralPath $manifestPath
    $manifest = $raw | ConvertFrom-Json
} catch {
    Write-Error "FAIL: runtime manifest is not valid JSON: $_"
    exit 1
}

# Placeholder detection: any value containing angle brackets or common filler words.
$placeholderPattern = '(<[^>]*>|\bTODO\b|\bPLACEHOLDER\b|\bCHANGEME\b|\bexample\.com\b)'

function Test-ScalarField([string]$name, $expected) {
    $value = $manifest.PSObject.Properties[$name]
    if ($null -eq $value) {
        Add-Error "missing required field '$name'"
        return
    }
    $actual = [string]$value.Value
    if ($actual -match $placeholderPattern) {
        Add-Error "field '$name' contains a placeholder value: $actual"
        return
    }
    if ($null -ne $expected -and $actual -cne $expected) {
        Add-Error "field '$name' must be exactly '$expected' but was '$actual'"
    }
}

Test-ScalarField "schema_version" "1"
Test-ScalarField "runtime_family" "lightpanda-wsl"
Test-ScalarField "lightpanda_version" $ExpectedLightpandaVersion
Test-ScalarField "lightpanda_arch" $ExpectedLightpandaArch
Test-ScalarField "lightpanda_asset_url" $ExpectedLightpandaUrl
Test-ScalarField "lightpanda_sha256" $ExpectedLightpandaSha256
Test-ScalarField "distro_image_version" $ExpectedDistroImageVersion
Test-ScalarField "distro_arch" $ExpectedDistroArch

# Floating/nightly versions are never acceptable.
foreach ($name in @("lightpanda_version", "distro_image_version")) {
    $prop = $manifest.PSObject.Properties[$name]
    if ($null -ne $prop -and ([string]$prop.Value -match "nightly|latest|rolling")) {
        Add-Error "field '$name' must be pinned, got floating value: $($prop.Value)"
    }
}

# Lightpanda digest must be 64 lowercase hex chars.
$lpSha = $manifest.PSObject.Properties["lightpanda_sha256"]
if ($null -ne $lpSha -and [string]$lpSha.Value -notmatch "^[0-9a-f]{64}$") {
    Add-Error "lightpanda_sha256 must be 64 lowercase hex characters"
}

# Distro image URL must be an immutable published release asset, not a
# mutable/latest link and not a placeholder.
$distroUrlProp = $manifest.PSObject.Properties["distro_image_url"]
if ($null -eq $distroUrlProp) {
    Add-Error "missing required field 'distro_image_url'"
} else {
    $distroUrl = [string]$distroUrlProp.Value
    if ($distroUrl -match $placeholderPattern) {
        Add-Error "distro_image_url contains a placeholder value: $distroUrl"
    } elseif ($distroUrl -match "nightly|/latest|latest/download|rolling") {
        Add-Error "distro_image_url must be immutable, got mutable URL: $distroUrl"
    } elseif ($distroUrl -notmatch "^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/releases/download/[^/]+/[^/]+$") {
        Add-Error "distro_image_url must be a pinned GitHub release asset URL (https://github.com/<owner>/<repo>/releases/download/<tag>/<file>), got: $distroUrl"
    } else {
        # The asset must be attached to the immutable runtime tag
        # "runtime-<distro_image_version>" so URL and version cannot drift.
        $tag = ($distroUrl -split "/releases/download/")[1].Split("/")[0]
        $expectedVersion = $manifest.PSObject.Properties["distro_image_version"]
        if ($null -ne $expectedVersion -and $tag -cne "runtime-$([string]$expectedVersion.Value)") {
            Add-Error "distro_image_url release tag '$tag' must equal 'runtime-$([string]$expectedVersion.Value)'"
        }
    }
}

# Distro image digest must be 64 lowercase hex chars.
$distroShaProp = $manifest.PSObject.Properties["distro_image_sha256"]
if ($null -eq $distroShaProp) {
    Add-Error "missing required field 'distro_image_sha256'"
} else {
    $distroSha = [string]$distroShaProp.Value
    if ($distroSha -match $placeholderPattern) {
        Add-Error "distro_image_sha256 contains a placeholder value: $distroSha"
    } elseif ($distroSha -notmatch "^[0-9a-f]{64}$") {
        Add-Error "distro_image_sha256 must be 64 lowercase hex characters, got: $distroSha"
    }
}

# Companion artifacts: package manifest + SPDX SBOM must exist and be non-trivial.
$packagesPath = Join-Path $RepoRoot "third-party/lightpanda-rootfs-packages.txt"
if (-not (Test-Path $packagesPath)) {
    Add-Error "missing rootfs package list at third-party/lightpanda-rootfs-packages.txt"
} elseif ((Get-Item $packagesPath).Length -lt 16) {
    Add-Error "rootfs package list is empty or trivially small"
}

$sbomPath = Join-Path $RepoRoot "third-party/lightpanda-rootfs-sbom.spdx.json"
if (-not (Test-Path $sbomPath)) {
    Add-Error "missing rootfs SBOM at third-party/lightpanda-rootfs-sbom.spdx.json"
} else {
    try {
        $sbom = Get-Content -Raw -LiteralPath $sbomPath | ConvertFrom-Json
        if (-not $sbom.PSObject.Properties["spdxVersion"]) {
            Add-Error "rootfs SBOM is missing 'spdxVersion'"
        }
        $packages = $sbom.PSObject.Properties["packages"]
        if ($null -eq $packages -or @($packages.Value).Count -eq 0) {
            Add-Error "rootfs SBOM lists no packages"
        }
    } catch {
        Add-Error "rootfs SBOM is not valid JSON: $_"
    }
}

if ($errors.Count -gt 0) {
    foreach ($e in $errors) {
        Write-Host "FAIL: $e" -ForegroundColor Red
    }
    Write-Error "lightpanda runtime manifest verification failed with $($errors.Count) error(s)"
    exit 1
}

Write-Host "OK: lightpanda runtime manifest verified (lightpanda $ExpectedLightpandaVersion, rootfs $ExpectedDistroImageVersion)"
exit 0
