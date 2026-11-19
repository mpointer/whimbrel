# Dot-source before building apps/windows/src-tauri for Windows on Arm (aarch64-pc-windows-msvc)
# on an x64 PC:
#
#   . .\apps\windows\scripts\arm64-env.ps1
#   cargo build --release --target aarch64-pc-windows-msvc
#
# Needs: `rustup target add aarch64-pc-windows-msvc`, and VS Build Tools with "MSVC ARM64 build
# tools" (Microsoft.VisualStudio.Component.VC.Tools.ARM64) and "C++ Clang tools for Windows".
#
# What it sets, each found the hard way (2026-11-13):
# - The x64-to-Arm64 cross compiler environment (vcvarsall x64_arm64). Without it CMake picks
#   whichever Visual Studio has no Arm64 tools.
# - clang on PATH: ring's Arm64 assembly needs it.
# - clang-cl for C and C++: ggml (whisper.cpp) refuses MSVC on Arm ("MSVC is not supported for
#   ARM, use clang"). /EHsc, because clang-cl leaves C++ exceptions off and esaxx and gguf use them.
# - Ninja instead of the Visual Studio generator, and whisper.cpp on the static C runtime (/MT)
#   in every configuration: the rest of the build is +crt-static (.cargo/config.toml), and
#   whisper-rs-sys builds RelWithDebInfo, whose default is /MD (LNK2038 otherwise).
#
# The Arm64 build has no Vulkan whisper and no diarization yet (src-tauri/Cargo.toml).

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw "vswhere.exe not found; install VS Build Tools with the C++ workload" }
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
if (-not $vs) { throw "No VS install with the MSVC Arm64 build tools: add Microsoft.VisualStudio.Component.VC.Tools.ARM64" }

# The cross compiler's environment, copied out of a cmd shell.
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvarsall.bat'
cmd /c "`"$vcvars`" x64_arm64 >nul && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
}

$llvmBin = Join-Path $vs 'VC\Tools\Llvm\x64\bin'
$cmakeBin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
$ninjaBin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja'
foreach ($f in @((Join-Path $llvmBin 'clang-cl.exe'), (Join-Path $cmakeBin 'cmake.exe'), (Join-Path $ninjaBin 'ninja.exe'))) {
    if (-not (Test-Path $f)) { throw "$f not found; add the 'C++ Clang tools' and 'C++ CMake tools' components" }
}
$env:PATH = "$llvmBin;$cmakeBin;$ninjaBin;$env:PATH"
$env:LIBCLANG_PATH = $llvmBin

$env:CC_aarch64_pc_windows_msvc = 'clang-cl'
$env:CXX_aarch64_pc_windows_msvc = 'clang-cl'
$env:CXXFLAGS_aarch64_pc_windows_msvc = '/EHsc'
$env:CMAKE_GENERATOR = 'Ninja'
foreach ($v in 'CMAKE_GENERATOR_INSTANCE', 'CMAKE_GENERATOR_PLATFORM', 'CMAKE_GENERATOR_TOOLSET') { Remove-Item "env:$v" -ErrorAction SilentlyContinue }
$env:CMAKE_MSVC_RUNTIME_LIBRARY = 'MultiThreaded'
$env:CMAKE_C_FLAGS_RELWITHDEBINFO = '/MT /Zi /O2 /Ob1 /DNDEBUG'
$env:CMAKE_CXX_FLAGS_RELWITHDEBINFO = '/MT /Zi /O2 /Ob1 /DNDEBUG /utf-8'

# SQLCipher (X4, issue #816) builds vendored OpenSSL, which needs Strawberry Perl and NASM. The
# environment copy above keeps PERL and PATH, so this only checks they are there: without them
# the Arm64 build fails deep inside openssl-src, long after this script said it was ready.
$perl = if ($env:PERL -and (Test-Path $env:PERL)) { $env:PERL } else { (Get-Command perl -ErrorAction SilentlyContinue).Source }
if (-not $perl) { throw "No Perl for OpenSSL (SQLCipher): install Strawberry Perl and set PERL to its perl.exe" }
& $perl -e 'require Locale::Maketext::Simple' 2>$null
if ($LASTEXITCODE -ne 0) { throw "$perl can't build OpenSSL (Git's Perl can't): set PERL to Strawberry Perl's perl.exe" }
$env:PERL = $perl
if (-not (Get-Command nasm -ErrorAction SilentlyContinue)) { throw "nasm is not on PATH; OpenSSL (SQLCipher) needs it" }

Write-Host "Arm64 cross build environment set from $vs"
