# Releasing

A release has **two publish steps**, one per platform. Both must run, or that platform's auto-update stays on the old version.

1. **Prepare** (branch `release/X`): bump every version file, regenerate the Xcode project, open a PR, merge when CI is green.
   - Files: `VERSION`, `apps/macos/project.yml`, `apps/windows/{package.json,package-lock.json,src-tauri/Cargo.toml,src-tauri/Cargo.lock,src-tauri/tauri.conf.json,ui/package.json,ui/package-lock.json}`, `core/{Cargo.toml,Cargo.lock}`, then `cd apps/macos && xcodegen generate`.
   - In the `Cargo.lock` files bump only our own packages. A blanket `sed` also rewrites third-party crates that happen to share the number (`crossbeam-epoch 0.9.20` once broke the lock).
   - Check: `cargo metadata --locked --offline` in `core/` and `apps/windows/src-tauri/`.
2. **Tag** the merge commit: `git tag -a vX <sha> -m vX && git push origin vX`.
3. **Windows**: pushing the tag runs `.github/workflows/release-windows.yml`, which builds the signed installer and publishes it and `windows/latest.json` to the updates Worker. It fails the run if the served manifest is not this version.
4. **Mac** (on the maintainer's Mac, unlocked, from a clean tree with HEAD at the tag): `cd apps/macos && ./scripts/release-publish.sh`. It builds, signs, notarizes, uploads `Whimbrel-X.zip` and `version.json` to the Worker, and creates the GitHub release if none exists. Takes about 20 minutes.
5. **Verify**: `version.json` and `windows/latest.json` serve X on both `updates.whimbrelapp.com` and `kestrel-updates-worker.mpointer.workers.dev` (shipped builds poll the second until the rename lands in a release).

The GitHub release page carries no installers; the Worker serves them.
