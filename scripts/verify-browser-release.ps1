#Requires -Version 5.1
<#
.SYNOPSIS
Fail-closed release gate for the Lightpanda WSL browser runtime.

.DESCRIPTION
Verifies every condition that must hold before a Windows NSIS release ships:

  1. The pinned Lightpanda 0.3.7 manifest is valid (exact URL + SHA-256, and the
     immutable rootfs URL + SHA-256, package list, and SPDX SBOM all present).
     This is delegated to scripts/verify-lightpanda-runtime-manifest.ps1 so the
     pinned constants live in exactly one reviewed place.
  2. AGPLv3 / upstream source reference metadata is present for Lightpanda.
  3. Desktop third-party notices exist and cover Lightpanda + the rootfs SBOM.
  4. No Playwright / Node / Chromium / Firefox browser engine is bundled.
  5. The NSIS installer exists and is below the 100 MiB size gate.
  6. The real-runtime integration log shows a green run with zero skips.

Exit code 0 = release gate passed; non-zero = failed (reasons on stderr).

.PARAMETER RepoRoot
Repository root. Defaults to the parent of the scripts/ directory.

.PARAMETER RuntimeLog
Path to the captured `browser_runtime_integration` test log that must show a
green, zero-skip run. Defaults to src-tauri/target/browser-runtime-integration-release.log.

.PARAMETER MaxNsisMib
Maximum allowed NSIS installer size in MiB (exclusive). Defaults to 100.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = "",
    [string]$RuntimeLog = "",
    [int]$MaxNsisMib = 100
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $RepoRoot = Split-Path -Parent $scriptDir
}
if ([string]::IsNullOrWhiteSpace($RuntimeLog)) {
    $RuntimeLog = Join-Path $RepoRoot "src-tauri/target/browser-runtime-integration-release.log"
}

$errors = New-Object System.Collections.Generic.List[string]
function Add-Error([string]$message) { $errors.Add($message) }

# ---------------------------------------------------------------------------
# 1. Pinned runtime manifest (exact Lightpanda URL/SHA + immutable rootfs
#    URL/SHA + rootfs package list + SPDX SBOM). Reuse the reviewed verifier.
# ---------------------------------------------------------------------------
$manifestVerifier = Join-Path $RepoRoot "scripts/verify-lightpanda-runtime-manifest.ps1"
if (-not (Test-Path $manifestVerifier)) {
    Add-Error "missing runtime manifest verifier at scripts/verify-lightpanda-runtime-manifest.ps1"
} else {
    & powershell -NoProfile -ExecutionPolicy Bypass -File $manifestVerifier -RepoRoot $RepoRoot *> $null
    if ($LASTEXITCODE -ne 0) {
        Add-Error "runtime manifest verification failed (Lightpanda/rootfs pin, SBOM, or package list)"
    }
}

# ---------------------------------------------------------------------------
# 2. Lightpanda AGPLv3 / upstream source reference.
# ---------------------------------------------------------------------------
$sourceRefPath = Join-Path $RepoRoot "third-party/lightpanda-source-reference.txt"
if (-not (Test-Path $sourceRefPath)) {
    Add-Error "missing Lightpanda source reference at third-party/lightpanda-source-reference.txt"
} else {
    $sourceRef = Get-Content -Raw -LiteralPath $sourceRefPath
    foreach ($token in @(
            "lightpanda-io/browser",
            "0.3.7",
            "AGPL",
            "unmodified",
            "895339b02205171a181dde743ae0068bb4564884076feac8482baca9c212aa5a"
        )) {
        if ($sourceRef -notmatch [regex]::Escape($token)) {
            Add-Error "lightpanda-source-reference.txt is missing required token: $token"
        }
    }
}

# ---------------------------------------------------------------------------
# 3. Desktop third-party notices covering Lightpanda + the rootfs SBOM.
# ---------------------------------------------------------------------------
$noticesPath = Join-Path $RepoRoot "THIRD_PARTY_NOTICES.md"
if (-not (Test-Path $noticesPath)) {
    Add-Error "missing THIRD_PARTY_NOTICES.md at the repository root"
} else {
    $notices = Get-Content -Raw -LiteralPath $noticesPath
    foreach ($token in @(
            "Lightpanda",
            "AGPL",
            "0.3.7",
            "Ubuntu",
            "SBOM"
        )) {
        if ($notices -notmatch [regex]::Escape($token)) {
            Add-Error "THIRD_PARTY_NOTICES.md is missing required token: $token"
        }
    }
}

# ---------------------------------------------------------------------------
# 4. No bundled Playwright / Node / Chromium / Firefox engine.
# ---------------------------------------------------------------------------
foreach ($dir in @(
        (Join-Path $RepoRoot "src-tauri/browser-runtime"),
        (Join-Path $RepoRoot ".cache/browser-runtime")
    )) {
    if (Test-Path $dir) {
        Add-Error "bundled browser runtime directory must not exist: $dir"
    }
}

$confPath = Join-Path $RepoRoot "src-tauri/tauri.conf.json"
if (Test-Path $confPath) {
    $conf = Get-Content -Raw -LiteralPath $confPath
    if ($conf -match "browser-runtime") {
        Add-Error "tauri.conf.json still references browser-runtime resources"
    }
} else {
    Add-Error "missing src-tauri/tauri.conf.json"
}

$srcTauri = Join-Path $RepoRoot "src-tauri"
$suspicious = Get-ChildItem -Path $srcTauri -Recurse -Force -ErrorAction SilentlyContinue |
    Where-Object {
        $_.FullName -notmatch [regex]::Escape("\target\") -and (
            $_.Name -ieq "node.exe" -or
            $_.Name -imatch "^playwright" -or
            $_.Name -imatch "^chromium" -or
            $_.Name -imatch "^firefox" -or
            $_.Name -imatch "headless_shell"
        )
    }
foreach ($item in $suspicious) {
    Add-Error "bundled browser engine artifact must not exist: $($item.FullName)"
}

# ---------------------------------------------------------------------------
# 5. NSIS installer exists and is below the size gate.
# ---------------------------------------------------------------------------
$nsisCandidates = @()
# The cargo target dir is at the workspace root (target/) when a workspace is
# present, but may be src-tauri/target/ otherwise. Search both.
foreach ($nsisDir in @(
        (Join-Path $RepoRoot "target/release/bundle/nsis"),
        (Join-Path $RepoRoot "src-tauri/target/release/bundle/nsis")
    )) {
    if (Test-Path $nsisDir) {
        $nsisCandidates += @(Get-ChildItem -Path $nsisDir -Filter *.exe -ErrorAction SilentlyContinue)
    }
}
$nsisCandidates = @($nsisCandidates | Sort-Object LastWriteTime -Descending)
if ($nsisCandidates.Count -eq 0) {
    Add-Error "no NSIS installer found under target/release/bundle/nsis (run 'npm run tauri build' first)"
} else {
    $nsis = $nsisCandidates[0]
    $maxBytes = [int64]$MaxNsisMib * 1024 * 1024
    $sizeMib = [math]::Round($nsis.Length / 1MB, 2)
    if ($nsis.Length -ge $maxBytes) {
        Add-Error ("NSIS installer {0} is {1} MiB; must be below {2} MiB" -f $nsis.Name, $sizeMib, $MaxNsisMib)
    } else {
        Write-Host ("OK: NSIS installer {0} is {1} MiB (< {2} MiB)" -f $nsis.Name, $sizeMib, $MaxNsisMib)
    }
}

# ---------------------------------------------------------------------------
# 6. Real-runtime integration log: green run, zero skips.
# ---------------------------------------------------------------------------
if (-not (Test-Path $RuntimeLog)) {
    Add-Error "missing real-runtime integration log at $RuntimeLog (capture it with LW_REQUIRE_LIGHTPANDA_RUNTIME_TESTS=1)"
} else {
    $log = Get-Content -Raw -LiteralPath $RuntimeLog
    if ($log -match "SKIP \[") {
        Add-Error "real-runtime integration log contains a skipped runtime test (SKIP [...])"
    }
    if ($log -notmatch "test result: ok") {
        Add-Error "real-runtime integration log has no 'test result: ok' summary"
    }
    $failedMatch = [regex]::Match($log, "test result: ok\. \d+ passed; (\d+) failed")
    if ($failedMatch.Success -and [int]$failedMatch.Groups[1].Value -ne 0) {
        Add-Error "real-runtime integration log reports $($failedMatch.Groups[1].Value) failed test(s)"
    }
    $passedMatch = [regex]::Match($log, "test result: ok\. (\d+) passed")
    if ($passedMatch.Success) {
        $passed = [int]$passedMatch.Groups[1].Value
        if ($passed -lt 19) {
            Add-Error "real-runtime integration log ran only $passed test(s); expected at least 19"
        }
    } else {
        Add-Error "real-runtime integration log does not report a passed-test count"
    }
}

# ---------------------------------------------------------------------------
# Result
# ---------------------------------------------------------------------------
if ($errors.Count -gt 0) {
    foreach ($e in $errors) {
        Write-Host "FAIL: $e" -ForegroundColor Red
    }
    Write-Error "browser release gate failed with $($errors.Count) error(s)"
    exit 1
}

Write-Host "OK: browser release gate passed (lightpanda pinned, notices present, no bundled engine, NSIS < $MaxNsisMib MiB, runtime log zero-skip)"
exit 0
