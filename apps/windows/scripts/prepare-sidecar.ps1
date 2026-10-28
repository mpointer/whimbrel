# Builds the MCP server program and puts it where the Windows app looks for it (M3.4 Task 5).
#
#   . .\apps\windows\scripts\prepare-sidecar.ps1              # development: %LOCALAPPDATA%\Whimbrel\data\bin\
#   . .\apps\windows\scripts\prepare-sidecar.ps1 -ForBundle   # release: apps\windows\src-tauri\binaries\
#
# The installer carries it next to Whimbrel's own exe (Tauri "externalBin", named with the target
# triple at build time and without it once installed). A development build has no installer, so
# the app also looks in %LOCALAPPDATA%\Whimbrel\data\bin\. Re-run after pulling a change to core\mcp-server.
#
# Target directory: an existing CARGO_TARGET_DIR is used as-is (the CI and release jobs set a short,
# persistent one outside the checkout, so a build stays incremental on the runner); otherwise
# WHIMBREL_TARGET_DIR (see dev-env.ps1) with a -core suffix, else core\target. It never sets
# CARGO_TARGET_DIR for the machine.

param([switch]$ForBundle)

$ErrorActionPreference = 'Stop'

$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$core = Join-Path $repo 'core'
$triple = 'x86_64-pc-windows-msvc'

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR }
             elseif ($env:WHIMBREL_TARGET_DIR) { "$env:WHIMBREL_TARGET_DIR-core" }
             else { Join-Path $core 'target' }

Push-Location $core
try {
    # Static C runtime, as the app itself is built (apps/windows/src-tauri/.cargo/config.toml): a
    # sidecar linked to the dynamic one needs vcruntime140.dll, which a clean PC may not have.
    $savedFlags = $env:RUSTFLAGS
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    # --target-dir wins over the environment, and says which directory is meant.
    cargo build --release -p mcp-server --bin whimbrel-mcp-server --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    $env:RUSTFLAGS = $savedFlags
    Pop-Location
}

$built = Join-Path $targetDir 'release\whimbrel-mcp-server.exe'
if (-not (Test-Path $built)) { throw "the build did not produce $built" }

# Fail here, not on a customer's PC: the program must not need the Visual C++ runtime installed.
$bytes = [System.IO.File]::ReadAllBytes($built)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
if ($text -match '(?i)vcruntime140') { throw "$built needs vcruntime140.dll; it must be built with a static C runtime" }

if ($ForBundle) {
    $dest = Join-Path $repo 'apps\windows\src-tauri\binaries'
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item $built (Join-Path $dest "whimbrel-mcp-server-$triple.exe") -Force
    Write-Host "staged for the installer: $dest\whimbrel-mcp-server-$triple.exe"
} else {
    $dest = Join-Path $env:LOCALAPPDATA 'Whimbrel\data\bin'
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item $built (Join-Path $dest 'whimbrel-mcp-server.exe') -Force
    Write-Host "staged for development: $dest\whimbrel-mcp-server.exe"
}
