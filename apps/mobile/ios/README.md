# Whimbrel for iPhone

Native SwiftUI app over the shared Rust core (`core/ffi`), per
[docs/15-mobile-app-design.md](../../../docs/15-mobile-app-design.md) §5.6. The app currently does
device enrollment (M4.0 task 6b): sign in with Google, wait for a Mac to approve this
iPhone while showing the device comparison code, then keep the unwrapped group key in the
Keychain. Once approved it pulls the account's content into its own local store and shows
the synced meeting count (6c).

## Requirements

- Xcode with the iOS 17+ SDK, [XcodeGen](https://github.com/yonaskolb/XcodeGen)
  (`brew install xcodegen`)
- Rust with the iOS targets: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- iOS 17 or later. The device build uses `-C target-feature=+fp16` (the candle/gemm-f16
  NEON code in `core/memory` needs it), so the phone needs a chip with FEAT_FP16
  (Apple A11 and later). Every iOS 17 device has one.
- Simulator: Apple Silicon Macs only. The xcframework has no x86_64 simulator slice, and
  `project.yml` excludes x86_64 for the simulator SDK.

## Build

```sh
apps/mobile/ios/scripts/build-ffi-xcframework.sh     # -> apps/mobile/ios/build/WhimbrelCore.xcframework
cp apps/mobile/ios/Secrets.xcconfig.example apps/mobile/ios/Secrets.xcconfig   # optional, gitignored
cd apps/mobile/ios && xcodegen generate              # only after editing project.yml
xcodebuild build -project Whimbrel.xcodeproj -scheme Whimbrel \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO
```

The script builds `ffi` with `--no-default-features` (no `diarize`: sherpa-onnx has no iOS
ONNX Runtime) in release by default; `--debug` builds the dev profile, which is what CI uses.
Rerun it after changing `core/`.

`Whimbrel.xcodeproj` is generated from `project.yml` but committed, because CI builds the
committed project. Regenerate and commit it whenever `project.yml` changes.

## Configuration

`Secrets.xcconfig` (gitignored; copy from `Secrets.xcconfig.example`) holds
`GOOGLE_IOS_CLIENT_ID` (the iOS OAuth client, written into Info.plist as
`GoogleOAuthClientID`, the key the shared `SyncConfig` reads) and can override `PRODUCT_BUNDLE_IDENTIFIER`. The default bundle id,
`com.mpointer.whimbrel.mobile`, is set in `Base.xcconfig`. It replaced the pre-rename id before the first TestFlight build;
that made it a new app, so a phone that ran the old id signs in and is approved again.
`Base.xcconfig` also sets `SYNC_WORKER_BASE_URL` (the deployed Worker, Info.plist
`SyncWorkerBaseURL`); `Secrets.xcconfig` can override it, e.g. for `wrangler dev`.
Without `Secrets.xcconfig`, or with the `REPLACE_ME` placeholder, the app still builds and
runs and the sign-in screen says Google sign-in is not configured.

## TestFlight

`scripts/testflight-upload.sh` builds the release xcframework, archives (Release, automatic
signing, team GP8PFLM3Q3), checks the archive's bundle id and version, and exports with
`method app-store-connect`, `destination upload`, which uploads it. It refuses a dirty tree
and a missing or placeholder `Secrets.xcconfig`. `MARKETING_VERSION` follows `VERSION`
(`scripts/check-version.sh`); the build number is the UTC time of the run, `YYYYMMDD.HHMM` (HHMM without leading zeros),
set at archive time, so each upload is higher than the last without a commit.

One-time setup:

1. App Store Connect > Apps > + > New App: iOS, name Whimbrel, bundle id
   `com.mpointer.whimbrel.mobile` (register it first under Certificates, Identifiers &
   Profiles > Identifiers if it isn't offered).
2. Users and Access > Integrations > App Store Connect API > Team Keys: generate a key with
   the App Manager role. Note the Key ID and the Issuer ID, and download `AuthKey_<KEYID>.p8`
   (it downloads once).
3. Store it outside the repo: `mkdir -p ~/.appstoreconnect/private_keys && mv
   ~/Downloads/AuthKey_<KEYID>.p8 ~/.appstoreconnect/private_keys/ && chmod 600
   ~/.appstoreconnect/private_keys/AuthKey_<KEYID>.p8`.
4. Export `ASC_KEY_ID` and `ASC_ISSUER_ID` in your shell profile (`ASC_KEY_PATH` if the
   `.p8` lives elsewhere).

Then, from a clean tree: `apps/mobile/ios/scripts/testflight-upload.sh`.

`Info.plist` sets `ITSAppUsesNonExemptEncryption` to NO (reasoning in `project.yml`), and
`Whimbrel/PrivacyInfo.xcprivacy` is the privacy manifest. Recheck its API declarations
with `nm -u` on the xcframework's `ios-arm64` library when the core's dependencies change.

## Enrollment

`EnrollmentModel` mirrors the Mac's `SyncState` enrollment path (keypair, `POST
/v1/auth/google`, the waiting poll with backoff, group-key unwrap) over an
`EnrollmentBackend` protocol. Unlike the Mac, the phone never generates the account's
group key: if no approved desktop exists on the account it says to set up Whimbrel on the
Mac first (`needsDesktopFirst`). `WhimbrelTests` drives it with a fake backend. Google sign-in
(`GoogleSignIn.swift`) uses `ASWebAuthenticationSession` with PKCE and the reversed client
id as the redirect scheme; refresh goes through the core's `whimbrel_oauth_refresh`. The
device secret key, group key and Google tokens are Keychain items with
`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (ordinary Keychain: X25519 keys can't
live in the Secure Enclave).

A phone that already holds the group key starts approved, even offline. The waiting poll
stops when the app goes to the background and re-checks on return to the foreground;
being signed out of Google while polling is a permanent error, like a 401/403.

## Sync and the meeting count

`SyncModel` runs the core's pull-only content sync (`whimbrel_sync_content_pull_only`, 30-day
capture window) into `Application Support/Sync/memory.sqlite` (`CoreMemoryStore`, iOS side; the directory is excluded from backup, file protection `completeUntilFirstUserAuthentication`), now
and then every 60 s while approved and in the foreground, doubling to 15 min on failures
(the Mac's `ContentSyncCoordinator` cadence, mirrored). No background work. The Turso token
comes from `POST /v1/auth/google` and is kept only in memory, renewed by re-authenticating
(the Mac's `SyncState.renewIfDue` rules, mirrored). The count is
`whimbrel_meetings_recent_grouped(200)`, the Mac Meetings list's call and limit, so copies of
one call count once. Like the Mac's Meetings list, the count is capped at the latest 200 rows (grouped). While a Mac is recording, its live meeting shows on the Mac but not on the phone until recording stops: meetings sync once recording ends. `core/api/tests/phone_meeting_count.rs` checks the phone's count equals
the Mac's against a local relay. Failures show a generic message; the core's cause goes to
the log as a private value.

Debug builds accept a `-WhimbrelDemoEnrollment` launch argument: a fake backend that stays
pending, to see the waiting screen without Google (`-WhimbrelDemoNeedsDesktop`: the
no-desktop-yet screen; `-WhimbrelDemoSynced`: approved at once, and each "pull" seeds
demo meetings, two copies of one call among them, into a throwaway store in the temp
directory, recreated each launch). The demo never touches real state: its made-up group key
goes to an in-memory Keychain (`KeychainAccess`, debug iOS builds only) and its store is
not `Application Support/Sync/`. The real Keychain needs a signed build, even
on the simulator: build without `CODE_SIGNING_ALLOWED=NO` (`CODE_SIGN_IDENTITY=-` signs
ad hoc) to run the app; the unit tests use an in-memory Keychain and run unsigned.

With `-WhimbrelDemoSynced` the screens read fixture data (`DemoFixtures.swift`: a brief, a
next meeting with prep, 12 meetings with summaries and one transcript, routine runs).
For screenshots, also pass `-WhimbrelDemoTab 0|1|2|3`, `-WhimbrelDemoMeeting <id>` (opens that
meeting), `-WhimbrelDemoTranscript` (opens its Transcript tab) or `-WhimbrelDemoSettings`.

## Shared code

The app compiles these Mac sources in place (listed by path in `project.yml`, not copied):
`apps/macos/Whimbrel/Support/` `Theme.swift`, `ThemeDefaults.swift`, `ThemeStore.swift`,
`KeychainAccess.swift`, `SyncKeychain.swift`, `SyncConfig.swift`, `SyncClient.swift`,
`SyncCoreBridge.swift`, `SyncTiming.swift` (the sync backoff and token-renewal timing, used by both apps).
`ThemeStore.swift` splits its AppKit color code behind `#if os(macOS)` with a UIKit
equivalent; `SyncKeychain.swift` sets the iOS accessibility class behind `#if os(iOS)`.
`CoreBridge.swift` can't compile on iOS (it references many Mac-only types), so the
sync-enrollment wrappers live in `SyncCoreBridge.swift` as an `extension CoreMemoryStore`,
and `Whimbrel/CoreMemoryStore+iOS.swift` declares the type on iOS with the few store calls
the phone makes (open, set device id, pull-only sync, grouped meetings, last sync error). The `themes/` folder is
bundled as a resource, as in the Mac app.

## CI

The `ios` job in `.github/workflows/rust-tests.yml` runs the script (`--debug`), a
simulator `xcodebuild build` with signing off, and `xcodebuild test` (WhimbrelTests) on an
iPhone simulator with the newest iOS runtime >= 17 (`scripts/pick-simulator.py`; fails if
there is none). It runs when `core/`, `apps/mobile/ios/`,
`apps/macos/Whimbrel/Support/`, `themes/` or the workflows change.
