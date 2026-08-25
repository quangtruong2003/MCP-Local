$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$skillName = 'local-workspace-agent'
$skillRoot = Join-Path $root "plugin\skills\$skillName"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist '.agent-skill-stage'
$archive = Join-Path $dist "$skillName.zip"

if (-not (Test-Path (Join-Path $skillRoot 'SKILL.md'))) {
    throw "Skill source is missing: $skillRoot"
}

New-Item -ItemType Directory -Path $dist -Force | Out-Null
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $archive -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $stage -Force | Out-Null

try {
    Copy-Item $skillRoot -Destination (Join-Path $stage $skillName) -Recurse
    Compress-Archive -Path (Join-Path $stage $skillName) -DestinationPath $archive -CompressionLevel Optimal
} finally {
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output "Created $archive"
