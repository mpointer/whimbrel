# Changelog

Notable changes to Whimbrel. Releases ship to Mac and Windows at the same time.

## [0.9.41] - 2026-10-30

### Mac
- Meeting audio and slide images now live in one media folder, which you can put on another drive (Settings → Retention). Audio is compressed after 30 days and slide images become thumbnails after 30 days; deleting old audio is off until you turn it on (#451).
- A per-meeting "Keep media" switch keeps a meeting's full recordings and images (#451).
- Slide snapshots are saved as HEIC and repeated frames are skipped (#451).
- Skins you make on the Mac now sync to your other computers, and skins from them appear on the Mac (#450).

### Windows
- Skins sync with the Mac (#450).

## [0.9.40] - 2026-10-29

### Mac
- Profile suggestions disappear as soon as you press Dismiss or Add (#447).
- Updated sign-in screens for device limits, swapping devices, the waitlist, fresh start and delete. They appear once the server rollout is done (#446).

### Windows
- Send Feedback in Settings, About and in the tray opens an email to the Whimbrel team with your version filled in (#442).
- Check for Updates is in Settings, General (#445).

### iPhone
- Updated sign-in screens, and you can approve devices from the iPhone (#446).

## [0.9.39] - 2026-10-29

### Mac
- Check for Updates in Settings and the app menu (#443).

### Windows
- Make and refine skins by chat (#433).
- Rename, delete, export and import skins (#434).
- Skins sync between your devices (#438).
- Skin commands for turns, images and keys (#432).

### iPhone
- Ask on the phone (#435).
- New Whimbrel app icon (#431).

### Website
- Contact address is hello@whimbrelapp.com, and the app has Send Feedback (#439).
- The privacy policy lists the gmail.send permission (#440).
- Failed email sends are now logged with the reason (#429).

### Sync service
- Skin rows now sync between devices (#436, #437).
- Cheaper models are tried first, and there is a new way to add your own key (#428).

## [0.9.38] - 2026-10-29

### Windows
- Meeting snapshots arrive on Windows: slides and faces are captured, and you get snapshot folders and two new settings (#413, #414, #416, #417).
- A Slides strip in meetings, with snapshot settings (#419).
- A privacy exception for snapshots, a way to open the folder, and a manual pass (#421).

### Website
- The changelog keeps its full history (#422).
- Removed the Kestrel rename notice from the download pages (#425).

## [0.9.37] - 2026-10-28

### Mac
- Sign-in tokens are saved more reliably in the Keychain (#423).

### iPhone
- Sign-in tokens are saved more reliably in the Keychain (#423).

## [0.9.36] - 2026-10-28

### Mac
- Fixed a menu-bar label loop that froze recordings on macOS 26 (#418).

### Windows
- Slides appear in summaries, search and Ask. Forget removes the snapshot folder (#411).

### Website
- The changelog and the update link now point to the public Whimbrel repository (#415).

## [0.9.35] - 2026-10-28

### Mac
- New Whimbrel menu-bar icon (#399).
- The data folder moves from Application Support/Kestrel to Whimbrel (#404).
- Away Mode: Whimbrel attends the Zoom meetings you skip (#405).

### Windows
- Tray icons now use the Whimbrel mark (#400).
- The crash supervisor stays in place after an update (#403).
- The data folder moves to %LOCALAPPDATA%\Whimbrel\data (#406).

### Website
- The privacy policy covers Google data, and terms of service are at /tos (#401).

## [0.9.34] - 2026-10-27

### Mac
- The Whimbrel icon and wordmark appear at the top of Today (#397).

### Windows
- Installing Whimbrel removes the old Kestrel install (#396).

## [0.9.33] - 2026-10-27

Kestrel is now Whimbrel. The app, its icon and its installer carry the new name. Recordings, transcripts, settings, API keys and sync enrollment carry over.

### Mac
- The update installs Whimbrel.app next to Kestrel.app, then removes the old app.
- If you installed the crash-restart LaunchAgent, run `apps/macos/scripts/install-launch-agent.sh /Applications/Whimbrel.app` again.

### Windows
- The installer is now `Whimbrel_0.9.33_x64-setup.exe`. It replaces the Kestrel install in place.
- New Whimbrel app icon (#392).

### Website
- New Whimbrel app icon, favicon and share image (#390).

## [0.9.32] - 2026-10-25

### Windows
- A recording stranded by a quit or crash is kept (#384).
- Whimbrel links a recording stranded by a crash back to its meeting (#386).

## [0.9.31] - 2026-10-24

### Mac
- Recording safeguards: a check-in prompt, a 6 hour cap, disk limits, smaller 16 kHz mono audio, and protection when the app quits or crashes (#382).

## [0.9.30] - 2026-10-24

### Mac
- Rejoining a dropped call continues the same meeting (#379).

### Website
- Whimbrel now lives on whimbrelapp.com, with a new look, the open-source licence and a new pricing model (#373, #374, #375, #377).

## [0.9.29] - 2026-10-23

### Mac
- Meeting snapshot fixes (#368).

### Windows
- Skins: load and apply them, choose from a gallery in Appearance, and six new layouts for Today (#362, #363, #364).

## [0.9.28] - 2026-10-22

### Mac
- Meeting snapshots: Whimbrel saves slides and faces shown during a call (#365).

## [0.9.27] - 2026-10-22

### Mac
- Settings no longer freezes when a skin change rebuilds it (#353).
- A meeting on two connected calendar accounts appears once (#354).

### Windows
- A meeting on two connected calendar accounts appears once (#354).

## [0.9.26] - 2026-10-22

### Mac
- The Daily Brief's Today line lists only today's meetings (#350).

## [0.9.25] - 2026-10-22

### Mac
- Make a skin by talking to Whimbrel (#343).
- Skin fixes: an Appearance caption, and a live Ask survives a skin switch (#336).
- The updater accepts a renamed app and installs it at the new path (#344).

### Windows
- Today refresh and schedule card (#334).

### Website
- Automatic deploys, welcome and lead-alert email, and a changelog page (#335).
