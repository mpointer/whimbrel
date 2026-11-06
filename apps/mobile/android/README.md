# Whimbrel for Android

Phone companion app. Package `com.mpointer.whimbrel.mobile`, `minSdk` 29, `arm64-v8a` only.

```bash
source ~/.config/whimbrel-android-env.sh
cd apps/mobile/android
cp secrets.properties.example secrets.properties   # fill in GOOGLE_WEB_CLIENT_ID
./gradlew :app:testDebugUnitTest
./gradlew :app:assembleDebug
```

`versionName` is the repo `VERSION` file; `versionCode` is `major*1_000_000 + minor*1_000 + patch`
(override with `-PversionCodeOverride=<int>`). Both are computed in `app/build.gradle.kts` only.

`secrets.properties`, `keystore.properties` and `local.properties` are gitignored.

## Release signing

```bash
scripts/create-release-keystore.sh ~/.config/whimbrel-android-release   # once; refuses to overwrite
export WHIMBREL_KEYSTORE_DIR=~/.config/whimbrel-android-release
./gradlew :app:assembleRelease
```

The script writes `whimbrel-release.jks` and `keystore.properties` (format in `keystore.properties.example`)
to that directory, which stays outside the repo, and the public SHA-1 to `release-sha1.txt` (committed;
paste it into the Android OAuth client). Back the keystore directory up with the other release secrets.
If it is lost, every phone must uninstall and reinstall, because updates must be signed with the same key.

## Build and install a release

Requirements: JDK 17, the Android SDK and NDK (`source ~/.config/whimbrel-android-env.sh`), Rust with `cargo-ndk`,
and a phone with Android 10 or newer on an arm64 CPU with FP16 (ARMv8.2 or newer, as in the Galaxy Z Fold 2025). Older cores
cannot run the Rust core; the app says so instead of crashing.

Secrets, all outside git:

- `secrets.properties` in this directory (`GOOGLE_WEB_CLIENT_ID`, `SYNC_WORKER_BASE_URL`; format in `secrets.properties.example`).
- `$WHIMBREL_KEYSTORE_DIR/keystore.properties` and the keystore it names (see Release signing above).

```bash
export WHIMBREL_KEYSTORE_DIR=~/.config/whimbrel-android-release
./gradlew :app:assembleRelease          # app/build/outputs/apk/release/app-release.apk, R8-minified and signed
adb install -r app/build/outputs/apk/release/app-release.apk
```

Without a computer: the maintainer's release script (`scripts/release-apk.sh`) uploads `Whimbrel-<version>.apk` to the
public release page `https://github.com/mpointer/whimbrel/releases/tag/vX`. On the phone, open that page in the browser,
download the APK, and allow "Install unknown apps" for that browser when asked. No GitHub sign-in is needed.

Upgrades: every build is signed with the same keystore, so a newer APK installs over the old one and keeps the phone's
enrollment and meeting store. Android only installs a higher `versionCode`. To reinstall the same version for a test,
build with `-PversionCodeOverride=<current+1>`.

The debug build (`:app:assembleDebug`) is a separate app, "Whimbrel Debug" (`com.mpointer.whimbrel.mobile.debug`), that installs
beside the release build. It accepts demo launch extras for screenshots: `whimbrel_demo_synced`, `whimbrel_demo_tab`,
`whimbrel_demo_meeting`, `whimbrel_demo_settings` (see `app/src/debug/java/com/mpointer/whimbrel/mobile/DemoHooks.kt`).
