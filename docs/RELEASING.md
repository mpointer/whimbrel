# Releasing

A release has **two publish steps**, one per platform. Both must run, or that platform's auto-update stays on the old version.

0. **Secrets, one time:** put `macos-Secrets.xcconfig`, `ios-Secrets.xcconfig` and `android-secrets.properties` in `~/.config/whimbrel/secrets/` (or set `WHIMBREL_SECRETS_DIR`). Each publish script runs `scripts/release-secrets.sh` and copies its file into the release worktree when it is missing, so a fresh worktree needs no manual copy.
1. **Prepare** (branch `release/X`): bump every version file, regenerate the Xcode project, open a PR, merge when CI is green.
   - Files: `VERSION`, `apps/macos/project.yml`, `apps/mobile/ios/project.yml` (`MARKETING_VERSION`), `apps/windows/{package.json,package-lock.json,src-tauri/Cargo.toml,src-tauri/Cargo.lock,src-tauri/tauri.conf.json,ui/package.json,ui/package-lock.json}`, `core/{Cargo.toml,Cargo.lock}`, then `xcodegen generate` in both `apps/macos` and `apps/mobile/ios`. `scripts/check-version.sh` fails if either app's `project.yml` or `project.pbxproj` disagrees with `VERSION`.
   - In the `Cargo.lock` files bump only our own packages. A blanket `sed` also rewrites third-party crates that happen to share the number (`crossbeam-epoch 0.9.20` once broke the lock).
   - Check: `cargo metadata --locked --offline` in `core/` and `apps/windows/src-tauri/`.
   - Add the release's entry to CHANGELOG.md in the release PR.
2. **Tag** the merge commit: `git tag -a vX <sha> -m vX && git push origin vX`.
3. **Windows**: pushing the tag runs `.github/workflows/release-windows.yml`, which builds the signed installer and publishes it and `windows/latest.json` to the updates Worker. It fails the run if the served manifest is not this version.
4. **Mac** (on the maintainer's Mac, unlocked, from a clean tree with HEAD at the tag): `cd apps/macos && ./scripts/release-publish.sh`. It builds, signs, notarizes, uploads `Whimbrel-X.zip` and `version.json` to the Worker, and creates the GitHub release if none exists. Takes about 20 minutes.
5. **Verify**: `version.json` and `windows/latest.json` serve X on both `updates.whimbrelapp.com` (what the Mac and Windows apps poll) and `kestrel-updates-worker.mpointer.workers.dev` (the same Worker, outside the zone). After a Windows release, `.github/workflows/arm-smoke.yml` installs the published Windows on Arm installer on a GitHub-hosted Arm machine and checks it starts.
6. **Public mirror**: from the same clean tree at the tag, `scripts/publish-public.sh vX`. It pushes the tree without `docs/superpowers/` to `mpointer/whimbrel` as one commit, tags it `vX`, and creates the public GitHub release with the private release's notes (private pull-request links stripped).
7. **Phones**, automatic: step 6 ends by running `scripts/release-phones.sh vX`, which loads `~/.config/whimbrel/release-env.sh` (the App Store Connect key and the Android keystore folder) and runs the TestFlight upload (`apps/mobile/ios/scripts/testflight-upload.sh`), then the Android APK upload (`apps/mobile/android/scripts/release-apk.sh vX`). Each copies its secrets file in from step 0. `scripts/publish-public.sh vX --no-phones` skips it; run `scripts/release-phones.sh vX` later from the same clean tree at the tag. The TestFlight build number is the UTC upload time (`YYYYMMDD.HHMM`), so it needs no bump. The APK goes on every release: the in-app update check offers only the newest release that has one. Never create a new keystore to get past a missing one, because a different key stops installed copies from upgrading. One-time setup: `apps/mobile/ios/README.md` (TestFlight) and `apps/mobile/android/README.md`.

## Client controls gate

Do not ship, or advertise `/for/client-controls/`, until all of these pass for the release candidate:

- Mac, Windows, iOS and Android builds all compile.
- The classify-first test passes: a new meeting for an excluded client makes no hosted call before it is filed.
- The sweep test passes: every caller that builds a hosted prompt goes through the exclusion gate.
- Windows CI compiles `apps/windows/src-tauri`, including `client_control_update`.
- Android builds and its unit tests pass.
- Hand check of the Mac and Windows client UI, including a client name typed with smart quotes matching its straight-quote form.
- A tested phone catch-up: an older phone updates, pulls once, and applies the earlier deletes and the exclusion setting.
- The matrix in `docs/superpowers/specs/2026-11-12-client-controls-results.md` has no row left "not measured".
- Site wording stays inside spec section 9 (`website/test/client-controls.test.ts` checks it). Translated pages need a fresh translation before they say the same.

## Cloud routine runner gate

Do not announce, price or enable the runner or the Run in the cloud toggle for anyone except the author until `docs/superpowers/specs/2026-11-15-cloud-routine-runner-gate.md` has no row left "not measured".

- The author's account runs the daily brief from the cloud for 14 consecutive days with the laptop closed overnight. Mike fills the daily log on real runs. A failed day restarts the count.
- Every privacy case on the page passes: excluded clients, filing before the run, client delete, Google Calendar, ExclusionsUnread, revoke, a stale runner, sync off, a routine switched off on one device, and the device prep skip.
- The measurements are recorded: compute, rows read, runs per day, full pulls and relay rows read per day (every run with a cloud-flagged routine pulls in full, even with nothing due), model cost per run, p95 duration (it replaces `READY_BY_P95_MS` and `READY_BY_MARGIN_MS`) and the 180 s pull cap.
- The cloud UI stays behind a hidden switch until this gate passes. With it off and no Whimbrel Cloud on the account, nothing new shows. Mac: `defaults write com.mpointer.kestrel com.mpointer.whimbrel.cloudRoutines.enabled -bool true`. Windows: `"cloudRoutines.enabled": true` in `%LOCALAPPDATA%\Whimbrel\data\settings.json` (never synced). With the switch on, "Add Whimbrel Cloud" and the "Run in the cloud" toggle show; with it off, the toggle shows only once an approved Whimbrel Cloud is on the account (or for a routine already set to run in the cloud), and the runner's row and Remove always show. Removing the switch from the apps is part of the release that opens the runner to others. See `docs/self-hosting-runner.md`.
- `scripts/runner-gate-report.py` summarises the host logs. The runner's report line has no date, so the host logs need dates (a timestamp on each line, or one file per day named `YYYY-MM-DD.log`). The streak counts days with a daily brief. It suggests the margin as the larger of 60 s and half the p95: a proposal for Mike to confirm. Test it with `python3 -I scripts/test_runner_gate_report.py`.
- Deployment is Mike's: apply migration 0005, set `RUNNER_EMAILS`, and set up the container host from the runbook.
- Translations of the new strings are a separate step, on Mike's word. The l10n check fails until then.

The website's changelog reads the mirror's releases and the manifest's `release_notes_url` links to them (switched after the first publish, v0.9.35), so step 6 is not optional: skipping it leaves the changelog one release behind and the update dialog's link dead until it runs.

The GitHub release page carries no Mac or Windows installers (the Worker serves them); the only asset on it is the Android APK from step 7, which every release carries.
