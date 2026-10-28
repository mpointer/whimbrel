# Stages the MCP connectors where the Windows app looks for them (M3.3 Task 7, M3.4 Task 6).
#
#   . .\apps\windows\scripts\prepare-connectors.ps1              # development: %LOCALAPPDATA%\Whimbrel\data\connectors\
#   . .\apps\windows\scripts\prepare-connectors.ps1 -ForBundle   # installer: apps\windows\src-tauri\bundle-resources\connectors\
#
# The app spawns `node <connectors>\<name>\dist\index.js` per call, and Node needs
# `node_modules` next to `dist`. This copies the committed `dist/` and installs the production
# dependencies. An installed Whimbrel carries the -ForBundle copy beside its executable (see
# prepare-bundle-resources.ps1, which also stages Node); a development build reads the other.
#
# Re-run it after pulling a change to mcp-servers/.

param([switch]$ForBundle)

$ErrorActionPreference = 'Stop'

$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$target = if ($ForBundle) { Join-Path $repo 'apps\windows\src-tauri\bundle-resources\connectors' }
          else { Join-Path $env:LOCALAPPDATA 'Whimbrel\data\connectors' }
# Calendar reads the day's meetings; Gmail sends the emails a routine drafts once you approve them.
# Outlook is deferred.
$connectors = @('google-calendar', 'gmail')

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "Node.js is not on PATH. Install Node LTS, or the app will report 'Node.js not found'."
}
Write-Host "node $(node --version)"

foreach ($name in $connectors) {
    $source = Join-Path $repo "mcp-servers\$name"
    if (-not (Test-Path (Join-Path $source 'dist\index.js'))) {
        throw "$name has no dist\index.js. Run 'npm install; npm run build' in $source first."
    }
    $destination = Join-Path $target $name
    New-Item -ItemType Directory -Force $destination | Out-Null

    Copy-Item (Join-Path $source 'dist') $destination -Recurse -Force
    Copy-Item (Join-Path $source 'package.json') $destination -Force
    Copy-Item (Join-Path $source 'package-lock.json') $destination -Force

    Push-Location $destination
    try {
        npm ci --omit=dev --silent
    } finally {
        Pop-Location
    }
    Write-Host "staged $name -> $destination"
}
