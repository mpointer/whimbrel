# Dot-source before building apps/windows/src-tauri outside a Developer PowerShell:
#
#   . .\apps\windows\scripts\dev-env.ps1
#
# whisper-rs-sys needs CMake (whisper.cpp) and libclang (bindgen). Both ship inside the
# VS Build Tools "Desktop development with C++" workload but are not on PATH in a plain
# shell. This finds them through vswhere instead of hardcoding a machine's install path.
# CI's windows-latest image already has both on PATH and does not need this.

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw "vswhere.exe not found; install VS Build Tools with the C++ workload" }

$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw "No VS install with the MSVC x64 toolset found" }

$cmakeBin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
$llvmBin  = Join-Path $vs 'VC\Tools\Llvm\x64\bin'

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    if (-not (Test-Path (Join-Path $cmakeBin 'cmake.exe'))) { throw "cmake.exe not found under $vs; add the 'C++ CMake tools for Windows' component" }
    $env:PATH = "$cmakeBin;$env:PATH"
}
if (-not $env:LIBCLANG_PATH) {
    if (-not (Test-Path (Join-Path $llvmBin 'libclang.dll'))) { throw "libclang.dll not found under $vs; add the 'C++ Clang tools for Windows' component" }
    $env:LIBCLANG_PATH = $llvmBin
}

# Whisper's Vulkan build (W-1.2) needs the Vulkan SDK: `winget install KhronosGroup.VulkanSDK`. Its
# installer sets VULKAN_SDK machine-wide, which a shell (or the CI runner's service) started before the
# install doesn't have yet. Its shader build also nests about 236 characters under the target dir, so
# keep the target dir under about 22 characters (E:\kestrel-win-target fits; see below).
if (-not $env:VULKAN_SDK) { $env:VULKAN_SDK = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine') }
if (-not $env:VULKAN_SDK -or -not (Test-Path (Join-Path $env:VULKAN_SDK 'Bin\glslc.exe'))) {
    throw "The Vulkan SDK isn't installed: winget install KhronosGroup.VulkanSDK"
}
$env:PATH = "$(Join-Path $env:VULKAN_SDK 'Bin');$env:PATH"

$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $cargoBin 'cargo.exe'))) {
    $env:PATH = "$cargoBin;$env:PATH"
}

# whisper.cpp's CMake try-compile runs under MSBuild, whose FileTracker fails with
# FTK1011 once a .tlog path passes MAX_PATH (260). Under a deep checkout (a git worktree,
# say) `target\release\build\whisper-rs-sys-<hash>\out\build\CMakeFiles\CMakeScratch\...`
# crosses it. A short target dir outside the repo keeps every build path well under.
#
# A debug and a release build together run past 100 GB (the sherpa-onnx library alone is ~1 GB per
# build-setting change), so the folder can be moved to a drive with room: set WHIMBREL_TARGET_DIR (for
# example `setx WHIMBREL_TARGET_DIR E:\kestrel-win-target`). It is deliberately NOT CARGO_TARGET_DIR:
# the self-hosted CI runner runs as the same user and its jobs need CARGO_TARGET_DIR unset
# (install-actions-runner.ps1), so a persistent CARGO_TARGET_DIR would leak into them. The user-level
# value is read directly so a shell that started before `setx` still gets it.
if (-not $env:CARGO_TARGET_DIR) {
    $target = $env:WHIMBREL_TARGET_DIR
    if (-not $target) { $target = [Environment]::GetEnvironmentVariable('WHIMBREL_TARGET_DIR', 'User') }
    if (-not $target) { $target = Join-Path $env:LOCALAPPDATA 'kestrel-win-target' }
    $env:CARGO_TARGET_DIR = $target
}

Write-Host "cmake:         $((Get-Command cmake).Source)"
Write-Host "LIBCLANG_PATH: $env:LIBCLANG_PATH"
Write-Host "VULKAN_SDK:    $env:VULKAN_SDK"
Write-Host "cargo:         $((Get-Command cargo -ErrorAction SilentlyContinue).Source)"
