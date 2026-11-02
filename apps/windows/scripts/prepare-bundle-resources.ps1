# Stages what the installer carries so a fresh install can connect Google Calendar and Gmail on a PC
# with no Node.js (M3.4 Task 6):
#
#   . .\apps\windows\scripts\prepare-bundle-resources.ps1
#
# Produces apps\windows\src-tauri\bundle-resources\ (gitignored):
#   connectors\<name>\{dist, package.json, package-lock.json, node_modules}   the two Google connectors
#   node\{node.exe, LICENSE}                                                  a pinned Node.js
#   vulkan\{vulkan-1.dll, LICENSE.txt}                                         the Khronos Vulkan loader
#   llama\{whimbrel-llama-server.exe, *.dll, LICENSE, LICENSE-LLVM-OpenMP}     the local model helper
#
# The local model helper (W-1.3): llama.cpp's own Windows release at the tag the Mac builds from,
# run by core's api::local_model behind Ask with nothing set up. Only what llama-server needs comes
# out of the zip: the server and its DLLs, the CPU backends ggml picks between at run time, and the
# Vulkan backend (it uses the GPU driver's vulkan-1.dll, else ggml runs on the CPU). It sits in its
# own folder so its ggml DLLs stay apart from whisper's, and is renamed as on the Mac so the launch
# sweep never stops someone's own llama-server. TO UPDATE IT: change $LlamaTag and both hashes,
# from https://github.com/ggml-org/llama.cpp/releases/tag/<tag> (the zip's digest is listed there)
# and https://raw.githubusercontent.com/ggml-org/llama.cpp/<tag>/LICENSE, and match the Mac's
# scripts/bundle-llama-server.sh.
#
# The Vulkan loader (W-1.2): whisper runs on the GPU through Vulkan, which makes vulkan-1.dll a hard
# import of whimbrel-windows.exe. GPU drivers install it, but a PC without one (a VM, a cloud desktop,
# Windows Server, the basic display driver) would not launch at all. The installer puts LunarG's
# signed redistributable next to the exe; with no GPU driver behind it, whisper runs on the CPU.
# TO UPDATE IT: change $VulkanRtVersion and its three hashes together, from
# https://sdk.lunarg.com/sdk/download/<version>/windows/VulkanRT-X64-<version>-Components.zip.
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
$VulkanRtVersion = '1.4.363.0'
$VulkanRtZipSha256 = 'A25A927AA8B9F0371048F1861CF88AC3B9BC9B1FB332C42D897C8AB32695769A'
$VulkanDllSha256 = 'E1FCFC9489BEEFA6A6D9D11D6C7517F6A2B7F16E0D3FB8F4103BEE0210F314A3'
$VulkanLicenseSha256 = '404DEB98DABF3E54BF9070D59676B31D69A3DA2F9F6E56BDF09AD315C04C718C'
$LlamaTag = 'b11877'
$LlamaZipSha256 = '05EF0107993C7FCA52975B8692449B419EF041A7C72D0B6A1E98326436EFC4EB'
$LlamaLicenseSha256 = '94F29BBED6A22C35B992C5C6EBF0E7C92F13B836B90F36F461C9CF2F0F1D010D'
# What llama-server loads, found by running it from a folder holding only these (plus ggml-cpu-*.dll).
$LlamaFiles = @('llama-server-impl.dll', 'llama-common.dll', 'llama.dll', 'mtmd.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-vulkan.dll', 'libomp.dll', 'LICENSE-LLVM-OpenMP')

$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$out = Join-Path $repo 'apps\windows\src-tauri\bundle-resources'
$cache = if ($env:KESTREL_CACHE) { $env:KESTREL_CACHE } else { Join-Path $env:LOCALAPPDATA 'whimbrel-build-cache' }
New-Item -ItemType Directory -Force $cache | Out-Null

function Get-Verified([string]$url, [string]$name, [string]$sha256, [string]$version = $NodeVersion) {
    $path = Join-Path $cache "$version-$name"
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
$vulkanZip = Get-Verified "https://sdk.lunarg.com/sdk/download/$VulkanRtVersion/windows/VulkanRT-X64-$VulkanRtVersion-Components.zip" 'VulkanRT-Components.zip' $VulkanRtZipSha256 "vulkan-$VulkanRtVersion"
# The two files out of the verified zip, each checked again before it is staged.
$vulkanExtract = Join-Path $cache "vulkan-$VulkanRtVersion-extract"
Remove-Item $vulkanExtract -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive -Path $vulkanZip -DestinationPath $vulkanExtract
$vulkanRoot = Join-Path $vulkanExtract "VulkanRT-X64-$VulkanRtVersion-Components"
$vulkanDll = Join-Path $vulkanRoot 'x64\vulkan-1.dll'
$vulkanLicense = Join-Path $vulkanRoot 'VulkanRT-License.txt'
foreach ($f in @(@($vulkanDll, $VulkanDllSha256), @($vulkanLicense, $VulkanLicenseSha256))) {
    $actual = (Get-FileHash $f[0] -Algorithm SHA256).Hash
    if ($actual -ne $f[1]) { throw "$($f[0]) has SHA-256 $actual, not the pinned $($f[1]). Nothing was staged." }
}
$llamaZip = Get-Verified "https://github.com/ggml-org/llama.cpp/releases/download/$LlamaTag/llama-$LlamaTag-bin-win-vulkan-x64.zip" 'llama-win-vulkan-x64.zip' $LlamaZipSha256 "llama-$LlamaTag"
$llamaLicense = Get-Verified "https://raw.githubusercontent.com/ggml-org/llama.cpp/$LlamaTag/LICENSE" 'LICENSE' $LlamaLicenseSha256 "llama-$LlamaTag"
$llamaExtract = Join-Path $cache "llama-$LlamaTag-extract"
Remove-Item $llamaExtract -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive -Path $llamaZip -DestinationPath $llamaExtract

Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
& (Join-Path $PSScriptRoot 'prepare-connectors.ps1') -ForBundle

$nodeDir = Join-Path $out 'node'
New-Item -ItemType Directory -Force $nodeDir | Out-Null
Copy-Item $nodeExe (Join-Path $nodeDir 'node.exe') -Force
Copy-Item $nodeLicense (Join-Path $nodeDir 'LICENSE') -Force
$vulkanDir = Join-Path $out 'vulkan'
New-Item -ItemType Directory -Force $vulkanDir | Out-Null
Copy-Item $vulkanDll (Join-Path $vulkanDir 'vulkan-1.dll') -Force
Copy-Item $vulkanLicense (Join-Path $vulkanDir 'LICENSE.txt') -Force
$llamaDir = Join-Path $out 'llama'
New-Item -ItemType Directory -Force $llamaDir | Out-Null
Copy-Item (Join-Path $llamaExtract 'llama-server.exe') (Join-Path $llamaDir 'whimbrel-llama-server.exe') -Force
foreach ($f in $LlamaFiles) { Copy-Item (Join-Path $llamaExtract $f) $llamaDir -Force }
Copy-Item (Join-Path $llamaExtract 'ggml-cpu-*.dll') $llamaDir -Force
Copy-Item $llamaLicense (Join-Path $llamaDir 'LICENSE') -Force
Write-Host "staged Node $NodeVersion, the Vulkan loader $VulkanRtVersion, llama.cpp $LlamaTag and the connectors in $out"
