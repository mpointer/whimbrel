# Installs a self-hosted GitHub Actions runner for mpointer/kestrel on this PC, so
# rust-tests.yml's `windows` job (runs-on: [self-hosted, Windows, X64, kestrel-windows])
# runs here instead of on paid windows-latest.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File apps\windows\scripts\install-actions-runner.ps1
#
# What it does:
#   1. Downloads actions-runner-win-x64 2.337.0 and checks its published SHA-256.
#   2. Extracts it to C:\ar (a short path: whisper.cpp's MSBuild step fails past 260 chars).
#   3. Registers it under this PC's name with the label "kestrel-windows", using a
#      one-hour registration token from `gh` (never printed). It runs as YOUR account, not
#      as a Windows service, so jobs see your rustup and VS Build Tools.
#   4. Sets the jobs' environment in C:\ar\.env, which the runner loads at start (it
#      ignored .path in practice):
#      - PATH: machine + user PATH, expanded, plus CMake and cargo, so rustup,
#        cmake and cargo are found (actions-rs/toolchain otherwise tries to install
#        rustup and fails on Windows);
#      - LIBCLANG_PATH (whisper-rs-sys's bindgen);
#      - core.fsmonitor=false through GIT_CONFIG_COUNT/KEY/VALUE. Git for Windows' system
#        gitconfig turns fsmonitor on, and its detached daemon held actions/checkout's
#        temporary HOME open, failing checkout with EBUSY.
#   5. Registers a scheduled task that starts the runner, hidden, when you sign in, and
#      starts it now.
# Workflows on this runner must not let Swatinem/rust-cache prune CARGO_HOME\bin: its
# save step (pushes to main) "cleans cargo/bin" down to cargo-installed binaries and
# deletes rustup.exe. Use `cache-bin: false` (or no rust-cache) on self-hosted jobs.
#
# Undo: Unregister-ScheduledTask 'GitHub Actions runner (whimbrel)'; then in C:\ar run
#   .\config.cmd remove --token <token from: gh api -X POST repos/mpointer/kestrel/actions/runners/remove-token --jq .token>
# and delete C:\ar.

$ErrorActionPreference = 'Stop'
$ver = '2.337.0'
$sha = '1150692afa94e71f872017e254ea55b6eece1eece3fe7e3a6d4c93d0a1b85cfc'
$gh = 'C:\Program Files\GitHub CLI\gh.exe'
$task = 'GitHub Actions runner (whimbrel)'

New-Item -ItemType Directory -Force C:\ar | Out-Null
$zip = "C:\ar\actions-runner-win-x64-$ver.zip"
Invoke-WebRequest -Uri "https://github.com/actions/runner/releases/download/v$ver/actions-runner-win-x64-$ver.zip" -OutFile $zip
$got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($got -ne $sha) { throw "checksum mismatch: $got" }
Write-Host 'checksum OK'
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::ExtractToDirectory($zip, 'C:\ar')

. (Join-Path $PSScriptRoot 'dev-env.ps1') | Out-Null
$libclang = $env:LIBCLANG_PATH
$cmakeBin = Split-Path -Parent (Get-Command cmake).Source
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$env:CARGO_TARGET_DIR = $null   # jobs build in their own checkout; rust-cache expects core/target

$token = & $gh api -X POST repos/mpointer/kestrel/actions/runners/registration-token --jq .token
Push-Location C:\ar
.\config.cmd --unattended --url https://github.com/mpointer/kestrel --token $token --name $env:COMPUTERNAME --labels kestrel-windows --work _work --replace
$token = $null
Pop-Location

# The runner gets its own Rust toolchain (C:\ar\.cargo, C:\ar\.rustup). actions-rs/toolchain
# and Swatinem/rust-cache modify CARGO_HOME/RUSTUP_HOME, and on the Mac runner a job deleted
# the user's own ~/.cargo/bin/rustup. Nothing a job runs may touch %USERPROFILE%\.cargo.
$cargoHome = 'C:\ar\.cargo'; $rustupHome = 'C:\ar\.rustup'
$userCargo = Join-Path $env:USERPROFILE '.cargo'
$init = 'C:\ar\rustup-init.exe'
$initUrl = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
Invoke-WebRequest $initUrl -OutFile $init -UseBasicParsing
# Saved to a file and read as text: Invoke-WebRequest's .Content for this URL is bytes.
Invoke-WebRequest "$initUrl.sha256" -OutFile "$init.sha256" -UseBasicParsing
$initSha = ((Get-Content "$init.sha256" -Raw).Trim() -split '\s+')[0].ToLower()
if ($initSha -notmatch '^[0-9a-f]{64}$') { throw "unexpected rustup-init.sha256 content: $initSha" }
if ((Get-FileHash $init -Algorithm SHA256).Hash.ToLower() -ne $initSha) { throw 'rustup-init checksum mismatch' }
$env:CARGO_HOME = $cargoHome; $env:RUSTUP_HOME = $rustupHome
& $init -y --no-modify-path --default-toolchain stable --profile minimal
# Nothing a job runs may replace rustup.exe: the proxies in C:\ar\.cargo\bin are
# symlinks to it, so a missing rustup.exe breaks cargo and rustc too.
& "$cargoHome\bin\rustup.exe" set auto-self-update disable
$env:CARGO_HOME = $null; $env:RUSTUP_HOME = $null

$parts = @([Environment]::GetEnvironmentVariable('Path', 'Machine'), [Environment]::GetEnvironmentVariable('Path', 'User')) -join ';' -split ';' |
    Where-Object { $_ } | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_) } |
    Where-Object { -not $_.StartsWith($userCargo, [StringComparison]::OrdinalIgnoreCase) }
$jobPath = (@("$cargoHome\bin") + @($parts) + $cmakeBin | Select-Object -Unique) -join ';'
Add-Content -Path C:\ar\.env -Value @(
    "CARGO_HOME=$cargoHome",
    "RUSTUP_HOME=$rustupHome",
    "PATH=$jobPath",
    "LIBCLANG_PATH=$libclang",
    'GIT_CONFIG_COUNT=1',
    'GIT_CONFIG_KEY_0=core.fsmonitor',
    'GIT_CONFIG_VALUE_0=false'
)

$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-NoProfile -WindowStyle Hidden -Command "Set-Location C:\ar; & .\run.cmd"'
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Settings $settings -Description 'Self-hosted runner for mpointer/kestrel (C:\ar). Starts at sign-in under this account.' | Out-Null
Start-ScheduledTask -TaskName $task
Start-Sleep -Seconds 15

Write-Host '--- the runner as GitHub sees it ---'
& $gh api repos/mpointer/kestrel/actions/runners --jq '.runners[] | {name, status, labels: [.labels[].name]}'
