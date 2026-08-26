$ErrorActionPreference = 'Stop'

$NodeVersion = '24.19.0'
$McpVersion = '0.0.79'
$repo = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $repo '.cache\browser-runtime'
$runtime = Join-Path $repo 'src-tauri\browser-runtime'
$stage = Join-Path $repo 'src-tauri\browser-runtime.stage'
$backup = Join-Path $repo 'src-tauri\browser-runtime.backup'
$rootModules = Join-Path $repo 'node_modules'

function Read-Package([string]$path) {
  if (-not (Test-Path -LiteralPath $path)) { throw "required npm package is missing: $path" }
  return Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
}

$mcpPackagePath = Join-Path $rootModules '@playwright\mcp\package.json'
$pwPackagePath = Join-Path $rootModules 'playwright\package.json'
$corePackagePath = Join-Path $rootModules 'playwright-core\package.json'
$mcp = Read-Package $mcpPackagePath
$pw = Read-Package $pwPackagePath
$core = Read-Package $corePackagePath

if ($mcp.version -ne $McpVersion) { throw "@playwright/mcp must be exactly $McpVersion, found $($mcp.version)" }
if ($mcp.dependencies.playwright -ne $pw.version) { throw "Playwright version does not match MCP dependency: $($pw.version)" }
if ($mcp.dependencies.'playwright-core' -ne $core.version) { throw "Playwright Core version does not match MCP dependency: $($core.version)" }
if ($pw.dependencies.'playwright-core' -ne $core.version) { throw 'Playwright and Playwright Core versions do not match' }

New-Item -ItemType Directory -Force -Path $cache | Out-Null
$zipName = "node-v$NodeVersion-win-x64.zip"
$zipPath = Join-Path $cache $zipName
$sumPath = Join-Path $cache 'SHASUMS256.txt'
$nodeBase = "https://nodejs.org/dist/v$NodeVersion"
if (-not (Test-Path -LiteralPath $zipPath)) {
  Write-Host "Downloading Node.js $NodeVersion runtime..."
  Invoke-WebRequest -UseBasicParsing -Uri "$nodeBase/$zipName" -OutFile $zipPath
}
if (-not (Test-Path -LiteralPath $sumPath)) {
  Invoke-WebRequest -UseBasicParsing -Uri "$nodeBase/SHASUMS256.txt" -OutFile $sumPath
}

$checksumLine = Get-Content -LiteralPath $sumPath | Where-Object { $_ -match "\s+$([regex]::Escape($zipName))$" } | Select-Object -First 1
if (-not $checksumLine) { throw "official Node checksum does not contain $zipName" }
$expectedHash = ($checksumLine -split '\s+')[0].ToUpperInvariant()
$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToUpperInvariant()
if ($actualHash -ne $expectedHash) { throw "Node archive checksum mismatch: expected $expectedHash, got $actualHash" }

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
New-Item -ItemType Directory -Force -Path $stage | Out-Null
$extract = Join-Path $stage '.node-extract'
Expand-Archive -LiteralPath $zipPath -DestinationPath $extract -Force
$nodeSource = Join-Path $extract "node-v$NodeVersion-win-x64"
if (-not (Test-Path -LiteralPath (Join-Path $nodeSource 'node.exe'))) { throw 'extracted Node runtime is incomplete' }
Move-Item -LiteralPath $nodeSource -Destination (Join-Path $stage 'node')
Remove-Item -Recurse -Force $extract

$stageModules = Join-Path $stage 'node_modules'
New-Item -ItemType Directory -Force -Path (Join-Path $stageModules '@playwright') | Out-Null
Copy-Item -Recurse -Force -LiteralPath (Join-Path $rootModules '@playwright\mcp') -Destination (Join-Path $stageModules '@playwright\mcp')
Copy-Item -Recurse -Force -LiteralPath (Join-Path $rootModules 'playwright') -Destination (Join-Path $stageModules 'playwright')
Copy-Item -Recurse -Force -LiteralPath (Join-Path $rootModules 'playwright-core') -Destination (Join-Path $stageModules 'playwright-core')

$node = Join-Path $stage 'node\node.exe'
$deviceScript = "const fs=require('fs'); const {devices}=require('./node_modules/playwright'); fs.writeFileSync('device-descriptors.json', JSON.stringify(devices));"
Push-Location $stage
try {
  & $node -e $deviceScript
  if ($LASTEXITCODE -ne 0) { throw "Playwright device descriptor export failed with exit code $LASTEXITCODE" }
} finally {
  Pop-Location
}
if (-not (Test-Path -LiteralPath (Join-Path $stage 'device-descriptors.json'))) { throw 'staged Playwright device descriptors are missing' }

$licenses = Join-Path $stage 'licenses'
New-Item -ItemType Directory -Force -Path $licenses | Out-Null
$licenseSources = @(
  @{ Source = Join-Path $stage 'node\LICENSE'; Name = 'NODE-LICENSE.txt' },
  @{ Source = Join-Path $stageModules '@playwright\mcp\LICENSE'; Name = 'PLAYWRIGHT-MCP-LICENSE.txt' },
  @{ Source = Join-Path $stageModules 'playwright\LICENSE'; Name = 'PLAYWRIGHT-LICENSE.txt' },
  @{ Source = Join-Path $stageModules 'playwright-core\LICENSE'; Name = 'PLAYWRIGHT-CORE-LICENSE.txt' }
)
foreach ($item in $licenseSources) {
  if (Test-Path -LiteralPath $item.Source) { Copy-Item -Force -LiteralPath $item.Source -Destination (Join-Path $licenses $item.Name) }
}

$browsers = Join-Path $stage 'browsers'
$browserCache = Join-Path $cache ("playwright-browsers-" + ($pw.version -replace '[^A-Za-z0-9._-]', '_'))
New-Item -ItemType Directory -Force -Path $browserCache | Out-Null
if (-not (Get-ChildItem -LiteralPath $browserCache -Directory -ErrorAction SilentlyContinue | Select-Object -First 1)) {
  $existingBrowsers = Join-Path $runtime 'browsers'
  if (Test-Path -LiteralPath $existingBrowsers) {
    Copy-Item -Recurse -Force -Path (Join-Path $existingBrowsers '*') -Destination $browserCache
  }
}
$node = Join-Path $stage 'node\node.exe'
$playwrightCli = Join-Path $stageModules 'playwright\cli.js'
$previousBrowsersPath = $env:PLAYWRIGHT_BROWSERS_PATH
try {
  $env:PLAYWRIGHT_BROWSERS_PATH = $browserCache
  Write-Host "Ensuring locked Playwright browsers ($($pw.version)) in build cache..."
  & $node $playwrightCli install chromium firefox
  if ($LASTEXITCODE -ne 0) { throw "Playwright browser install failed with exit code $LASTEXITCODE" }
} finally {
  $env:PLAYWRIGHT_BROWSERS_PATH = $previousBrowsersPath
}
Copy-Item -Recurse -Force -LiteralPath $browserCache -Destination $browsers

$browserEntries = @(Get-ChildItem -LiteralPath $browsers -Directory | Sort-Object Name | ForEach-Object {
  [ordered]@{ name = $_.Name }
})
if (-not ($browserEntries.name | Where-Object { $_ -match '^chromium-' })) { throw 'staged browser install did not produce Chromium' }
if (-not ($browserEntries.name | Where-Object { $_ -match '^firefox-' })) { throw 'staged browser install did not produce Firefox' }

$manifest = [ordered]@{
  node_version = $NodeVersion
  playwright_mcp_version = $mcp.version
  playwright_version = $pw.version
  playwright_core_version = $core.version
  platform = 'windows-x64'
  browser_entries = $browserEntries
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'runtime-manifest.json')

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $backup
$hadRuntime = Test-Path -LiteralPath $runtime
if ($hadRuntime) { Move-Item -LiteralPath $runtime -Destination $backup }
try {
  Move-Item -LiteralPath $stage -Destination $runtime
  & (Join-Path $PSScriptRoot 'verify-browser-runtime.ps1')
  if ($LASTEXITCODE -ne 0) { throw 'browser runtime verification failed after staging' }
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $backup
} catch {
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $runtime
  if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $runtime }
  throw
}

Write-Host "Browser runtime staged: Node $NodeVersion; MCP $($mcp.version); Playwright $($pw.version)"
