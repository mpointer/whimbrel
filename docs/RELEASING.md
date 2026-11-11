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
5. **Verify**: `version.json` and `windows/latest.json` serve X on both `updates.whimbrelapp.com` and `kestrel-updates-worker.mpointer.workers.dev` (shipped builds poll the second until the rename lands in a release).
6. **Public mirror**: from the same clean tree at the tag, `scripts/publish-public.sh vX`. It pushes the tree without `docs/superpowers/` to `mpointer/whimbrel` as one commit, tags it `vX`, and creates the public GitHub release with the private release's notes (private pull-request links stripped).
7. **Phones**, automatic: step 6 ends by running `scripts/release-phones.sh vX`, which loads `~/.config/whimbrel/release-env.sh` (the App Store Connect key and the Android keystore folder) and runs the TestFlight upload (`apps/mobile/ios/scripts/testflight-upload.sh`), then the Android APK upload (`apps/mobile/android/scripts/release-apk.sh vX`). Each copies its secrets file in from step 0. `scripts/publish-public.sh vX --no-phones` skips it; run `scripts/release-phones.sh vX` later from the same clean tree at the tag. The TestFlight build number is the UTC upload time (`YYYYMMDD.HHMM`), so it needs no bump. The APK goes on every release: the in-app update check offers only the newest release that has one. Never create a new keystore to get past a missing one, because a different key stops installed copies from upgrading. One-time setup: `apps/mobile/ios/README.md` (TestFlight) and `apps/mobile/android/README.md`.

The website's changelog reads the mirror's releases and the manifest's `release_notes_url` links to them (switched after the first publish, v0.9.35), so step 6 is not optional: skipping it leaves the changelog one release behind and the update dialog's link dead until it runs.

The GitHub release page carries no Mac or Windows installers (the Worker serves them); the only asset on it is the Android APK from step 7, which every release carries.
