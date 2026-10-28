# Fails if anything in the Windows shell's dependency graph enables libsql's `core`
# feature. That feature bundles libsql's own SQLite, which collides at link time with
# rusqlite's bundled SQLite (LNK2005 on MSVC) — docs/12-windows-app-design.md §3.
# Intended for the M3.5 `windows-app` CI job; runnable locally from any directory.

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '..\src-tauri\Cargo.toml'

# `cargo tree -e features -i libsql` lists every feature edge into libsql, including
# `libsql feature "core"` if anything turns it on.
$tree = cargo tree --manifest-path $manifest -e features -i libsql --target x86_64-pc-windows-msvc 2>&1
if ($LASTEXITCODE -ne 0) { $tree; throw "cargo tree failed" }

if ($tree -match 'libsql feature "core"') {
    $tree
    throw "libsql's `core` feature is enabled in apps/windows/src-tauri's graph (see docs/12 §3)"
}
Write-Host "OK: libsql `core` feature is not enabled."
