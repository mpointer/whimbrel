# Security Policy

Whimbrel is an early-access product from a one-person team. Signed Mac and Windows
builds are distributed from the website; the update pipeline, the sync relay and the
connectors are real and in use.

## Reporting a vulnerability

Please do not open a public GitHub issue for a security report.

- Preferred: GitHub's private vulnerability reporting for this repository
  (Security tab, "Report a vulnerability").
- Alternative: email mpointer@gmail.com with details and, if possible,
  reproduction steps.

Expect an initial response within a few days. There is no SLA.

## Supported versions

Only the latest tagged release is supported. There is no LTS or backport policy.

## Scope

In scope: the desktop apps, the Rust core, the MCP connectors, the sync relay
(`server/`), the update feed (`server-updates/`) and the release pipeline. The
hosted deployment of the relay is in scope for reports; please do not test it
with accounts you do not own.

## What is already covered

- Releases are cut from a clean, tagged, version-agreeing commit
  (`scripts/check-version.sh`, `apps/macos/scripts/release-publish.sh`) and
  published as a SHA-256-verified, Developer ID-signed, notarized Mac build and a
  Tauri-updater-signed Windows installer.
- The Mac updater verifies a download's SHA-256 against the published manifest
  before unzipping it, and verifies the app's code signature and Team ID before
  staging it for restart. The Windows updater verifies the manifest signature.
  A hash or signature failure is refused, not logged.
- Sync payloads are end-to-end encrypted on the device; the relay stores
  ciphertext and sees account emails, device names and token issuance only.
- The relay's sign-in allowlist fails closed.
- CI (`release-integrity.yml`) enforces the version-agreement and update-manifest
  contract on every push and pull request without signing credentials;
  `license-audit.yml` checks the dependency graph's licenses.

## Known gaps

- Dependency vulnerabilities are tracked by GitHub Dependabot; open alerts are
  not triaged on a fixed schedule.
- Six Rust alerts stay open on purpose (reviewed 2026-10-30) until `libsql`
  ships a release that fixes them:
  - `rustls` 0.22.4 and `rustls-webpki` 0.102.8 come only from libsql's TLS
    stack (`hyper-rustls` 0.25). libsql 0.10 still uses it.
  - `libsql-sqlite3-parser` 0.13.0 has no patched release.

  The risk is low. These are only used for the sync client's connection to
  Turso, and the advisories concern certificate revocation lists, certificates
  from a malicious authority, debug output of TLS sessions, and malformed SQL
  input, none of which Whimbrel exposes. Everything else in the app already
  uses `rustls` 0.23.46 and `rustls-webpki` 0.103.16.
- Meeting-recording consent affordances (visible indicator, one-click discard,
  announcement template; FR-MTG-6) are not complete. Check your jurisdiction's
  recording-consent rules before recording others.
