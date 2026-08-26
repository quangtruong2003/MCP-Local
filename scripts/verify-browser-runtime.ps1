$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $repo 'src-tauri\browser-runtime'
$node = Join-Path $runtime 'node\node.exe'
$mcpCli = Join-Path $runtime 'node_modules\@playwright\mcp\cli.js'
$mcpPackage = Join-Path $runtime 'node_modules\@playwright\mcp\package.json'
$playwrightPackage = Join-Path $runtime 'node_modules\playwright\package.json'
$corePackage = Join-Path $runtime 'node_modules\playwright-core\package.json'
$browsers = Join-Path $runtime 'browsers'
$manifestPath = Join-Path $runtime 'runtime-manifest.json'

$required = @($node, $mcpCli, $mcpPackage, $playwrightPackage, $corePackage, $browsers, $manifestPath)
foreach ($path in $required) {
  if (-not (Test-Path -LiteralPath $path)) { throw "browser runtime missing required path: $path" }
}

$mcp = Get-Content -Raw -LiteralPath $mcpPackage | ConvertFrom-Json
$pw = Get-Content -Raw -LiteralPath $playwrightPackage | ConvertFrom-Json
$core = Get-Content -Raw -LiteralPath $corePackage | ConvertFrom-Json
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json

if ($mcp.version -ne '0.0.79') { throw "unexpected Playwright MCP version: $($mcp.version)" }
if ($manifest.node_version -ne '24.19.0') { throw "unexpected Node version in manifest: $($manifest.node_version)" }
if ($manifest.playwright_mcp_version -ne $mcp.version) { throw 'manifest MCP version mismatch' }
if ($manifest.playwright_version -ne $pw.version) { throw 'manifest Playwright version mismatch' }
if ($manifest.playwright_core_version -ne $core.version) { throw 'manifest Playwright Core version mismatch' }

$nodeVersion = (& $node --version).Trim().TrimStart('v')
if ($LASTEXITCODE -ne 0 -or $nodeVersion -ne '24.19.0') { throw "staged Node version mismatch: $nodeVersion" }

$browserNames = @($manifest.browser_entries | ForEach-Object { $_.name })
if (-not ($browserNames | Where-Object { $_ -match '^chromium-' })) { throw 'runtime manifest has no Chromium browser entry' }
if (-not ($browserNames | Where-Object { $_ -match '^firefox-' })) { throw 'runtime manifest has no Firefox browser entry' }
foreach ($entry in $manifest.browser_entries) {
  $entryPath = Join-Path $browsers $entry.name
  if (-not (Test-Path -LiteralPath $entryPath)) { throw "runtime manifest browser entry missing: $($entry.name)" }
}

& $node $mcpCli --help | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'staged Playwright MCP CLI failed' }

Write-Host "Browser runtime verified: Node $nodeVersion; MCP $($mcp.version); Playwright $($pw.version); browsers $($browserNames -join ', ')"
