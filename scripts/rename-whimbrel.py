#!/usr/bin/env python3
"""Mechanical rename of the product from Kestrel to Whimbrel (rename design step 4b,
docs/superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md).

Run from the repository root on a clean tree:

    python3 scripts/rename-whimbrel.py          # apply
    python3 scripts/rename-whimbrel.py --check  # fail if a kept identifier changed or a
                                                # renameable token survives

The script is the reviewable artefact: the rules below say what changes and, more
importantly, what must not. Everything that keys user data or names a deployed
resource is on the keep list and is restored verbatim after the pass.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.environ.get("RENAME_ROOT") or Path(__file__).resolve().parent.parent).resolve()

# Directories never touched (history, dependencies, build output).
SKIP_DIRS = {
    ".git", "node_modules", "target", "dist", "build", "DerivedData", ".wrangler",
    "docs/superpowers",          # plans and specs are the historical record
    "docs/release-notes",        # shipped release notes say what shipped
    "docs/release-evidence",
    "docs/refactor-package",     # a third-party assessment, quoted as delivered
}
# Files never touched by content replacement (historical narration stays).
SKIP_FILES = {
    "docs/05-roadmap.md",
    "website/site/data/releases.json",  # shipped release titles, quoted as shipped
    "CHANGELOG.md",                     # past releases, quoted as shipped
    "scripts/rename-whimbrel.py",       # this file names the old identifiers on purpose
    "apps/windows/src-tauri/nsis/hooks.nsh",  # finds and removes the Kestrel install by name
    "apps/windows/src-tauri/src/paths.rs",    # moves the Windows data folder from its old name
}
BINARY_SUFFIXES = {
    ".png", ".jpg", ".jpeg", ".gif", ".ico", ".icns", ".webp", ".mp4", ".webm", ".mov",
    ".pdf", ".woff", ".woff2", ".ttf", ".otf", ".sqlite", ".db", ".bin", ".zip", ".caf",
    ".wav", ".onnx", ".safetensors",
}

# Strings that must survive byte for byte. Longer, more specific entries first.
KEEP = [
    # Identifiers that key user data (decision 1 of the design).
    "com.mpointer.kestrel",                       # Mac bundle id, Tauri identifier, UserDefaults,
                                                  # Keychain and Credential Manager services,
                                                  # LaunchAgent label, OSLog subsystems
    'previousFolderName = "Kestrel"',            # the Mac data folder's old name, searched for once
    "`Application Support/Kestrel/`, and before", # SpikePaths doc comment naming that move
    "(Kestrel, then littlebird)",
    "testMovesKestrelFolderToWhimbrel",           # SpikePathsTests for that move
    "testKestrelPreferredOverLittlebird",
    "LOCALAPPDATA%\\Kestrel",                     # the Windows data folder's old name, in docs naming that move
    "kestrel.sqlite",                             # the Windows database file, which keeps its name
    "KESTREL_CACHE",                              # the Windows build cache variable on dev machines
    "kestrel-win-target",                         # the dev cargo target directory, same reason
    # Stored values and wire constants (review by the Mac session, 2026-10-26): a
    # renamed value here orphans rows, re-triggers a first-launch wipe, or breaks key
    # exchange between devices on different versions during the rollout.
    "kestrel-sync-group-key-wrap",                # HKDF info for the group key wrap (core/sync)
    "kestrel_data_plane",                         # outbox connector value stored on rows
    "kestrel.sync.keychainCleaned",               # iOS first-launch marker (missing = wipe keys)
    "kestrel.sync.storeRemovalPending",           # iOS interrupted-wipe marker
    ".kestrel-snapshots",                         # snapshot folder marker retention looks for
    "kestrel-notary",                             # notarytool keychain profile on the release Mac
    # Deployed resources (decision 2): hostnames compiled into shipped builds, the
    # bucket, the D1 database, the per-account database prefix, the website cookie.
    "kestrel-updates-worker",
    "kestrel-sync-worker",
    "kestrel-updates",                            # R2 bucket (also matches .../kestrel-updates/)
    "kestrel-leads",
    "kestrel-sync-${",
    "kestrel-sync-abc123",                        # relay test fixtures for that prefix
    "kestrel-sync-existing",
    "kestrel_lead",
    "kestrel-website",                            # the marketing Worker and its hostname
    "mpointer/kestrel",                           # the GitHub repository: workflow gates, release
                                                  # URLs, runner registration. The public export
                                                  # (step 4d) is where this becomes mpointer/whimbrel.
    "kestrel_hp",                                 # the website's old honeypot field, still read
    # The one place the old name is meant to appear in code after the rename: the
    # stored-skin-id migration the followups add, and its test.
    'skin == "kestrel"',
    'd.set("kestrel", forKey: skinKey)',
    # Sentences that name the old name on purpose (the website's rename note and FAQ,
    # the docs' "renamed" banners, references to the two rename specs by file name).
    "kestrel-to-whimbrel",
    "littlebird-to-kestrel",
    "Kestrel to Whimbrel",
    "Kestrel became Whimbrel",
    "Kestrel is now called Whimbrel",
    "Kestrel was renamed",
    "renamed from Kestrel",
    "used to be called Kestrel",
    "still say Kestrel",
    "kestrel app (former name",
    "the earlier kestrel",
    # UpdateCheckerTests exercises the transition updater (step 4a) with the real old
    # bundle name next to the new one; renaming its fixtures would make the "renamed
    # bundle" cases test a same-name swap.
    'for name in ["Kestrel.app", "Whimbrel.app"]',
    'two.appendingPathComponent("Kestrel.app")',
    'file.appendingPathComponent("Kestrel.app")',
    '"Applications/Kestrel.app"',
    '"staged/Kestrel.app"',
    # The skin file extension exported before the rename, accepted on import; the
    # followup writes it with this comment so only that line is kept.
    "kestrelskin</string><!-- exported before the rename -->",
    # The Windows import picker offers the same old extension (apps/windows skin_maker.rs).
    '"kestrelskin"]; // exported before the rename',
]
# Kept-prefix identifiers that were renamed after all; --check flags them even though
# KEEP's "com.mpointer.kestrel" covers them. The iOS app moved to com.mpointer.whimbrel.mobile
# (and .tests, and its OSLog subsystem) before its first TestFlight build, when no
# installed copy held data worth keeping.
RETIRED = ["com.mpointer.kestrel.mobile"]

# The self-hosted runner label is the same text as the Windows crate name. It stays
# only where it is a label: the workflows and the runner install script.
RUNNER_LABEL_FILES = {".github/workflows", "apps/windows/scripts/install-actions-runner.ps1"}

REPLACEMENTS = [("Kestrel", "Whimbrel"), ("KESTREL", "WHIMBREL"), ("kestrel", "whimbrel")]
PLACEHOLDER = "\u0000KEEP{}\u0000"


def tracked_files() -> list[Path]:
    out = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True).stdout
    return [ROOT / p for p in out.decode().split("\0") if p]


def skipped(rel: str) -> bool:
    parts = rel.split("/")
    for i in range(1, len(parts)):
        if "/".join(parts[:i]) in SKIP_DIRS or parts[i - 1] in SKIP_DIRS:
            return True
    return rel in SKIP_FILES


def is_binary(path: Path) -> bool:
    """Binary by suffix or by a NUL in the first 8 KiB."""
    if path.suffix.lower() in BINARY_SUFFIXES:
        return True
    with open(path, "rb") as f:
        head = f.read(8192)
    if b"\0" in head:
        return True
    return False


def read(path: Path) -> str:
    # surrogateescape keeps any byte that is not UTF-8 (one Windows-1252 ellipsis exists
    # in apps/windows/ui) exactly as it was, so the rename never alters encoding.
    return path.read_text(encoding="utf-8", errors="surrogateescape")


def write(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8", errors="surrogateescape")


def protect(text: str, rel: str) -> tuple[str, list[str]]:
    kept: list[str] = []
    keep = list(KEEP)
    if any(rel.startswith(p) for p in RUNNER_LABEL_FILES):
        keep.insert(0, "kestrel-windows")
    for s in keep:
        if s in text:
            kept.append(s)
            text = text.replace(s, PLACEHOLDER.format(len(kept) - 1))
    return text, kept


def restore(text: str, kept: list[str]) -> str:
    for i, s in enumerate(kept):
        text = text.replace(PLACEHOLDER.format(i), s)
    return text


def rename_text(text: str, rel: str) -> str:
    text, kept = protect(text, rel)
    for old, new in REPLACEMENTS:
        text = text.replace(old, new)
    return restore(text, kept)


def rename_path(rel: str) -> str:
    out = []
    for part in rel.split("/"):
        for old, new in REPLACEMENTS:
            part = part.replace(old, new)
        out.append(part)
    return "/".join(out)


def apply() -> int:
    changed = 0
    renamed = 0
    for path in tracked_files():
        rel = path.relative_to(ROOT).as_posix()
        if skipped(rel) or not path.exists():
            continue
        if not is_binary(path):
            original = read(path)
            updated = rename_text(original, rel)
            if updated != original:
                write(path, updated)
                changed += 1
        new_rel = rename_path(rel)
        if new_rel != rel:
            (ROOT / new_rel).parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "mv", "-k", rel, new_rel], cwd=ROOT, check=True)
            renamed += 1
    print(f"content changed in {changed} files; {renamed} paths renamed")
    followups()
    return 0


def edit(rel: str, old: str, new: str) -> None:
    path = ROOT / rel
    text = read(path)
    if text.count(old) != 1:
        sys.exit(f"followup edit did not match exactly once in {rel}: {old[:60]!r}")
    write(path, text.replace(old, new))


def followups() -> None:
    """The edits the token pass cannot express. Each one is a single exact match, so a
    tree the pass did not produce fails loudly instead of being half-edited."""
    # A skin choice stored under the built-in skin's old id must keep pointing at it:
    # a selected skin that stops existing falls back to the default and looks like a bug.
    edit(
        "apps/macos/Whimbrel/Support/SkinStore.swift",
        """        if let skin = defaults.string(forKey: Self.skinKey) {
            storedSkinId = skin
""",
        """        if let skin = defaults.string(forKey: Self.skinKey) {
            // The built-in skin was renamed with the product (rename design step 4b); a
            // choice stored under its old id keeps pointing at it and is rewritten once.
            storedSkinId = skin == "kestrel" ? SkinDefaults.whimbrel.id : skin
            if storedSkinId != skin { defaults.set(storedSkinId, forKey: Self.skinKey) }
""",
    )
    edit(
        "apps/macos/WhimbrelTests/SkinStoreTests.swift",
        """    // MARK: migration
""",
        """    // MARK: migration

    func testAStoredOldSkinIdReadsAsWhimbrelAndIsRewritten() {
        let d = defaults()
        d.set("kestrel", forKey: skinKey)
        let (store, _) = store(d)
        XCTAssertEqual(store.selected.id, SkinDefaults.whimbrel.id)
        XCTAssertEqual(d.string(forKey: skinKey), SkinDefaults.whimbrel.id, "rewritten once, under the live id")
    }
""",
    )
    # Skins exported before the rename keep opening: the UTI (an identifier, kept)
    # gains the old extension next to the new one.
    edit(
        "apps/macos/Whimbrel/Info.plist",
        """\t\t\t\t\t<string>whimbrelskin</string>
""",
        """\t\t\t\t\t<string>whimbrelskin</string>
\t\t\t\t\t<string>kestrelskin</string><!-- exported before the rename -->
""",
    )
    # Feed and relay hostnames in app config move to the custom domains (decision 2).
    # The Workers keep their names and their workers.dev hostnames for shipped builds.
    U = "kestrel-updates-worker.mpointer.workers.dev"
    SY = "kestrel-sync-worker.mpointer.workers.dev"
    for rel, old, new in [
        ("apps/macos/Whimbrel/Support/UpdateChecker.swift", f"https://{U}/version.json", "https://updates.whimbrelapp.com/version.json"),
        ("apps/macos/scripts/release-publish.sh", f"https://{U}/$UPDATE_ZIP_NAME", "https://updates.whimbrelapp.com/$UPDATE_ZIP_NAME"),
        ("apps/windows/src-tauri/tauri.conf.json", f"https://{U}/windows/latest.json", "https://updates.whimbrelapp.com/windows/latest.json"),
        ("apps/macos/Secrets.xcconfig.example", f"https://{SY}", "https://sync.whimbrelapp.com"),
        ("apps/macos/Secrets.xcconfig.example", f"https:/$()/{SY}", "https:/$()/sync.whimbrelapp.com"),
        ("apps/macos/README.md", f"https://{SY}", "https://sync.whimbrelapp.com"),
        ("apps/macos/README.md", f"https:/$()/{SY}", "https:/$()/sync.whimbrelapp.com"),
        ("apps/mobile/ios/Base.xcconfig", f"https:/$()/{SY}", "https:/$()/sync.whimbrelapp.com"),
        (".github/workflows/release-windows.yml", f"UPDATES_URL: https://{U}", "UPDATES_URL: https://updates.whimbrelapp.com"),
        ("website/wrangler.jsonc", f'"UPDATES_URL": "https://{U}"', '"UPDATES_URL": "https://updates.whimbrelapp.com"'),
        ("website/wrangler.jsonc", '"UPDATES_HOSTS": "updates.whimbrelapp.com"', f'"UPDATES_HOSTS": "{U}"'),
        ("website/test/index.test.ts", f'const UPDATES = "https://{U}"', 'const UPDATES = "https://updates.whimbrelapp.com"'),
    ]:
        edit(rel, old, new)
    # Historical narration keeps its wording and points here (design, step 4b, Docs).
    edit(
        "docs/05-roadmap.md",
        "# 05 \u2014 Roadmap\n",
        "# 05 \u2014 Roadmap\n\n> Renamed: the product became Whimbrel on 2026-10-21 (`docs/superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md`). This file keeps its historical wording; \"Kestrel\" below is the same product.\n",
    )
    edit(
        "docs/superpowers/specs/2026-08-05-littlebird-to-kestrel-mechanical-rename-design.md",
        "# littlebird \u2192 kestrel mechanical rename \u2014 design\n",
        "# littlebird \u2192 kestrel mechanical rename \u2014 design\n\n> Superseded in part: the product was renamed again, Kestrel to Whimbrel, on 2026-10-21. See `2026-10-21-kestrel-to-whimbrel-rename-design.md`, which reuses the method here.\n",
    )
    # Two tests encode consequences of the rename: the built-in skin's id now sorts
    # last, and the website's "other feed host" is the workers.dev address, not the
    # custom domain, because the custom domain is now the primary.
    edit(
        "apps/windows/ui/src/skin.test.ts",
        '["almanac", "game-day", "high-contrast", "hypermodern", "whimbrel", "reference", "retro-terminal"]',
        '["almanac", "game-day", "high-contrast", "hypermodern", "reference", "retro-terminal", "whimbrel"]',
    )
    edit(
        "website/test/index.test.ts",
        """  it("accepts an installer on the updates Worker's custom domain when UPDATES_HOSTS lists it", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const custom = "https://updates.whimbrelapp.com/Whimbrel-1.0.0.zip";
""",
        """  it("accepts an installer on the updates Worker's workers.dev address when UPDATES_HOSTS lists it", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const custom = "https://kestrel-updates-worker.mpointer.workers.dev/Whimbrel-1.0.0.zip";
""",
    )
    edit(
        "website/test/index.test.ts",
        """      env: { UPDATES_HOSTS: "updates.whimbrelapp.com" },
""",
        """      env: { UPDATES_HOSTS: "kestrel-updates-worker.mpointer.workers.dev" },
""",
    )
    print("followups applied: SkinStore maps a stored kestrel skin id (with a test); old skin extension accepted; app hostnames on the custom domains; history pointers; two tests follow the rename")


def check() -> int:
    """After the pass: no renameable token left outside the skipped history, no path
    still carrying the old name. Kept strings are invisible to this check by
    construction (they are protected before matching). Exit 1 on any finding."""
    findings = 0
    token = re.compile(r"[A-Za-z0-9_.\\/-]*[Kk]estrel[A-Za-z0-9_.\\/-]*")
    for path in tracked_files():
        rel = path.relative_to(ROOT).as_posix()
        if skipped(rel) or not path.exists() or is_binary(path):
            continue
        text = read(path)
        for s in RETIRED:
            for m in re.finditer(re.escape(s), text):
                line = text.count("\n", 0, m.start()) + 1
                print(f"{rel}:{line}: retired identifier survives: {s}")
                findings += 1
        protected, _ = protect(text, rel)
        for m in token.finditer(protected):
            if PLACEHOLDER[0] in m.group(0):
                continue
            line = protected.count("\n", 0, m.start()) + 1
            print(f"{rel}:{line}: renameable token survives: {m.group(0)}")
            findings += 1
        if "estrel" in rename_path(rel):
            print(f"{rel}: path still carries the old name")
            findings += 1
    print("check: clean" if findings == 0 else f"check: {findings} findings")
    return 1 if findings else 0


if __name__ == "__main__":
    os.chdir(ROOT)
    sys.exit(check() if "--check" in sys.argv else apply())
