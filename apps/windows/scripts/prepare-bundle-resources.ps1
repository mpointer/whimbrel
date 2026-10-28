# Stages what the installer carries so a fresh install can connect Google Calendar and Gmail on a PC
# with no Node.js (M3.4 Task 6):
#
#   . .\apps\windows\scripts\prepare-bundle-resources.ps1
#
# Produces apps\windows\src-tauri\bundle-resources\ (gitignored):
#   connectors\<name>\{dist, package.json, package-lock.json, node_modules}   the two Google connectors
#   node\{node.exe, LICENSE}                                                  a pinned Node.js
#
# Node is downloaded from nodejs.org and checked against the SHA-256 pinned below before it is used;
# a mismatch deletes the file and stops. The download is cached (KESTREL_CACHE, else
# %LOCALAPPDATA%\whimbrel-build-cache) so a build does not fetch 80 MB every time. Not the app's data
# folder: this PC may be a CI runner, and the app decides where its data is at launch.
#
# TO UPDATE NODE: change $NodeVersion and both hashes together. The hash for node.exe is on the
# release's SHASUMS256.txt under win-x64/node.exe; the LICENSE is the file of that name at the tag
# https://github.com/nodejs/node/blob/<version>/LICENSE. Re-run the connectors against the new Node.

$ErrorActionPreference = 'Stop'

$NodeVersion = 'v22.23.3'
$NodeExeSha256 = '9c9245166b4a8e182e0b797da9c20136117ff24368eaff1fec8343a123c8db0e'
$NodeLicenseSha256 = 'c738ae413cf561f174e34f6961f8ca458aae2369a73640dda6234c629b98bcc4'

$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$out = Join-Path $repo 'apps\windows\src-tauri\bundle-resources'
$cache = if ($env:KESTREL_CACHE) { $env:KESTREL_CACHE } else { Join-Path $env:LOCALAPPDATA 'whimbrel-build-cache' }
New-Item -ItemType Directory -Force $cache | Out-Null

function Get-Verified([string]$url, [string]$name, [string]$sha256) {
    $path = Join-Path $cache "$NodeVersion-$name"
    if (Test-Path $path) {
        if ((Get-FileHash $path -Algorithm SHA256).Hash -eq $sha256) { return $path }
        Remove-Item $path -Force   # a damaged cache entry: fetch it again
    }
    Write-Host "downloading $url"
    $saved = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'   # the progress bar makes Invoke-WebRequest very slow
    try {
        Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
    } finally {
        $ProgressPreference = $saved
    }
    $actual = (Get-FileHash $path -Algorithm SHA256).Hash
    if ($actual -ne $sha256) {
        Remove-Item $path -Force
        throw "$url has SHA-256 $actual, not the pinned $sha256. Nothing was staged."
    }
    return $path
}

# Verify before touching the output, so a bad download never leaves a half-staged folder.
$nodeExe = Get-Verified "https://nodejs.org/dist/$NodeVersion/win-x64/node.exe" 'node.exe' $NodeExeSha256
$nodeLicense = Get-Verified "https://raw.githubusercontent.com/nodejs/node/$NodeVersion/LICENSE" 'LICENSE' $NodeLicenseSha256

Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
& (Join-Path $PSScriptRoot 'prepare-connectors.ps1') -ForBundle

$nodeDir = Join-Path $out 'node'
New-Item -ItemType Directory -Force $nodeDir | Out-Null
Copy-Item $nodeExe (Join-Path $nodeDir 'node.exe') -Force
Copy-Item $nodeLicense (Join-Path $nodeDir 'LICENSE') -Force
Write-Host "staged Node $NodeVersion and the connectors in $out"
