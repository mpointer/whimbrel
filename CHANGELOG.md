# Changelog

Notable changes to Whimbrel. Releases ship to Mac and Windows at the same time.

## [0.9.54] - 2026-11-04

### Mac
- A meeting Ask answer no longer hides under its Copy buttons. The first answer in a meeting could show one word per line with nothing to scroll.
- Two more ready buttons beside "Make me sound smart" in a meeting: "Bring me up to speed" and "Did anyone ask me a question?". The second looks for your name and the other names in Settings → Profile.
- Settings → Routines shows which device chose where routines run, and when, so a change nobody meant to make is easy to spot.

### Windows
- Your profile now syncs with your Mac. The first sync takes the Mac's profile, so the PC can't blank it.

## [0.9.53] - 2026-11-04

### Mac
- Every German, Spanish, French and Portuguese string in the app has been checked by hand, and about 350 read better: German is formal throughout, Spanish uses Latin American wording, and French and Canadian French each use their own terms. The Routines tab is now translated, and "models available", "other devices" and "pending approvals" use the right plural in every language.
- Settings → Your data now says when the update check really runs: when the app starts, then once a day. Only the random identifier it sends changes weekly.

### Website
- The changelog page now shows these release notes, written for people who use Whimbrel, instead of a list of code changes.
- People who register get a short personal note the next day with a quick start guide, in the language they signed up in. Every email has a link to choose whether to get product news, and product news now needs a confirmed opt-in.

### Windows
- Whimbrel now listens for questions someone asks you during a meeting it is recording, and shows them with a draft answer from your memory, on this PC and your other devices. Turn it on in Settings → Meetings once your name is in Settings → Profile.

## [0.9.52] - 2026-11-04

### Mac
- New Settings → Capture & Privacy → Your data: what Whimbrel keeps on this Mac and how much space it takes, every way data can leave your computer and whether each is on here, and what hosted models cost you in the last 30 days.
- Export everything writes your meetings with their transcripts, screen text, email, calendar, conversations, routines and topics to a folder you can read without Whimbrel.
- Delete everything on this Mac removes the database, the data folder, and this Mac's recordings and slide images, and can delete your sync account in the same step. Settings, keys and downloaded models stay. The deletion runs as Whimbrel restarts, so nothing is removed while the app has it open.
- Screen capture can skip sites you list in Settings → Capture, including their subdomains. The list syncs to your other devices.
- The menu bar shows a plain dot while only activity capture is running.

### Windows
- When another of your devices hears someone ask you something in a meeting, this PC now shows it too: a popup with Open meeting, and a "Questions for you" section on the meeting page. Turn it off in Settings → Meetings.

## [0.9.51] - 2026-11-03

### Mac
- Whimbrel now warns you while a meeting is recording if it has heard nothing for 3 minutes, with a notification (tap it to open the meeting) and a banner on the meeting page that goes away when sound returns. It never stops the recording by itself.
- Question alerts are tidier. The alerts on a meeting page keep updating for 30 minutes after it ends, so a late draft or an alert from another device shows up without reopening it. Your own microphone no longer counts as room audio once the other side has been heard, tapping an alert from another device opens its meeting, a source cited twice shows once, and the Spanish and Portuguese "asked at" wording reads better.

## [0.9.50] - 2026-11-03

### Mac
- A meeting you deleted that came from another of your devices now comes back when that device syncs it again, instead of being silently dropped. Old meetings from before August are no longer copied a second time.
- Meetings that ended up with no transcript no longer fail quietly. A failed transcription now says so on the meeting and is logged, a recording with no speech says "No speech detected" and still offers Transcribe again, and a recording cut short by a quit or crash is transcribed on the next launch.

## [0.9.49] - 2026-11-03

### Mac
- Whimbrel can tell you when someone in a meeting asks you something, with a notification and a draft answer from your memory, and can email it to your own Gmail. The meeting page lists these under "Questions for you"; turn it on in Settings → Meetings once your name is in Settings → Profile.

## [0.9.48] - 2026-11-02

### Mac
- Ask works with nothing set up. The empty Ask screen offers a small model, about 1.9 GB, that runs on this Mac with no account. Answers are slower and plainer than a hosted model's. Anything you've set up yourself comes first, and Settings → AI Providers can remove it.
- Meeting summaries read about twice as much of a long meeting at a time, so they see more context and take fewer model calls. Local Ollama summaries now ask for the same larger context window as Ask.
- Meeting Ask answers are formatted (headings, lists, bold) and selectable while the live transcript updates. Copy puts the answer on the clipboard as Markdown, or Copy as plain text without it.
- New "Make me sound smart" button in the meeting Ask bar: a quick read of where the meeting is, the key points, where it touches your profile, and two or three things you could say next. Typing the phrase works too.

### Windows
- Ask works with nothing set up. The empty Ask screen offers a small model, about 1.9 GB, that runs on the PC with no account; a key or Ollama still comes first. It uses the graphics card where there is one, and Settings → AI & routing can remove it.
- Meeting summaries read about twice as much of a long meeting at a time.

## [0.9.47] - 2026-11-02

### Mac
- A short "Room audio only" copy of a calendar meeting is now grouped with the recording from the Mac that was in the call, instead of showing as a second meeting.
- Settings → Clean up duplicate meetings no longer loses transcripts. A meeting synced from another device could arrive twice, with its transcript on only the older copy, and the cleanup deleted that copy. It now moves the transcript, slides and action items to the copy it keeps.
- A recording with no speech, notes or slides no longer gets a summary. The model was being asked to summarize nothing, and it wrote one from your profile instead (the 10/30 "Quick Debrief").
- A calendar meeting now starts recording on its own only on a Mac that's in the call: its microphone is in use and it's playing audio. Another Mac in the room keeps the prompt without a countdown, and Start still works there. This now applies to Google Calendar events too.
- Search and Ask leave out copies of a call recorded on more than one device, unless only a copy has the match.
- A meeting split into two by a restart, crash or early stop can be merged back: Meeting options › Merge with previous meeting, when the earlier one has the same title and ended under 10 minutes before. The merged meeting is transcribed and summarized again.

### iPhone
- Search and Ask leave out copies of a call recorded on more than one device.

### Windows
- Search and Ask leave out copies of a call recorded on more than one device.
- While the device list is loading, a PC that isn't the primary waits instead of running routines alongside the primary. The daily profile suggestions wait too.

## [0.9.46] - 2026-11-01

### Mac
- Only the primary device runs routines now. Other Macs ran them too unless Settings → Devices had been opened since launch. While the device list is loading, a Mac that isn't the primary waits.
- The "Better transcripts" offer now appears on Today, not only in Library.

### Windows
- An optional reminder to tell people you're recording, with wording you can copy, in Settings → Meetings. It's off by default. The consent note sits beside it and on the Record a meeting card.
- Import on the Meetings tab brings in transcripts (.vtt, .srt, .txt, Otter's text export) and a Granola CSV as meetings, and a folder of Markdown as searchable notes. Repeats and meetings already recorded here are skipped, and one import summarizes its newest 25 meetings. Regenerate now works on imported meetings and ones synced from the Mac.

## [0.9.45] - 2026-11-01

### Mac
- If sync fails or sticks for two hours, the sidebar says so.
- A device far behind now catches up about six times faster, with a short break between rounds so the app stays free.

### Windows
- New installs get better transcripts. Whisper now runs on the graphics card where there is one, which makes it several times faster, and a PC fast enough for it is offered a larger model, about 574 MB. Settings → Meeting models has the same Quality setting as the Mac: Automatic, Better or Fast. Audio stays on the PC.
- Existing installs keep their model and see one offer in Library, which can be dismissed. Past transcripts don't change.

## [0.9.44] - 2026-11-01

### Mac
- A new install opens on one screen asking for the microphone and system audio. Calendar, Accessibility and Notifications are now asked for when you first use what needs them.
- Models download in the background instead of behind a prompt, with progress in the sidebar. Keyword search works meanwhile.
- Every model download is now checked against a known hash before it is installed.
- New installs get better transcripts. On macOS 26 or later Whimbrel uses Apple's on-device speech engine; on older Macs fast enough for it, Whimbrel offers a larger model, about 574 MB. Audio stays on the Mac with either one.
- Existing installs keep their model and see one offer, which can be dismissed. Past transcripts don't change.
- Settings → Transcription has a Quality setting: Automatic, Better or Fast. A model file you chose yourself still wins.
- A new setting, off by default, reminds you to tell people you're recording, with wording you can edit and copy. A note about consent rules now appears on the first-run screen.
- Bring in what you already have: `.vtt`, `.srt` and `.txt` transcripts, Otter text exports and a Granola CSV as meetings, and a folder of Markdown as searchable notes. One import summarizes its newest 25 meetings.
- The default skin's greeting and section headers now follow your language, on Mac and Windows. They stayed in English before.
- A meeting that ends while models are still downloading now waits for them and is transcribed, instead of failing.

### Windows
- The whole app now follows your language: Spanish, French, German, Brazilian Portuguese, and British, Canadian and Australian English. The translations are AI-generated and still being reviewed. Meetings in other languages are transcribed as spoken (#475, #476, #477, #485, #486).
- Meeting and search models download in the background at first launch, with progress at the foot of the sidebar. Keyword search works meanwhile. Before, both waited for Settings → Meeting models and Settings → Semantic search.

## [0.9.43] - 2026-10-31

### Languages
- Whimbrel now speaks Spanish, French (Canada and France), German and Brazilian Portuguese, plus British, Canadian and Australian English. The translations are AI-generated and still being reviewed (#464, #469, #471).
- New settings: Language, and "Write notes and summaries in", which applies to notes, summaries, Ask answers and Away follow-ups (#464).
- Meetings in other languages: turn on "I have meetings in other languages" to have each meeting's language detected and transcribed as spoken, with a per-meeting "Transcript language" menu. Uses an optional larger model (#464).
- Search now understands 50+ languages, so a Spanish question finds an English meeting. On first launch Whimbrel downloads a new search model of about 470 MB and rebuilds its search index in the background; until that finishes, Ask and semantic search find less, and keyword search works as usual. The model is no longer bundled in the Mac app (#467, #468).
- Slides and windows are read in their own language (#464).

### Mac
- Back-to-back meetings no longer merge into one recording: Whimbrel splits at the next meeting's start, with a "Keep together" option (#463).
- A Join button in the meeting-start pop-up opens Zoom, Teams, Google Meet and Webex (#462).
- Away Mode is shown as "Beta coming soon" until it can join meetings. Its old switch only kept the Mac awake, so that setting is cleared (#473).

### Windows
- The language setting, the shell and Settings are translated; the rest of the app follows (#465, #470, #472).

### iPhone
- Strings are translated, with a Language row and the "Write notes and summaries in" picker. Read Aloud speaks in the output language (#464).

### Sync service
- Superseded copies of a row are now removed as devices push, which keeps sync small (#461).

## [0.9.42] - 2026-10-30

### Sync service
- Syncing no longer stalls when many rows are waiting for a related meeting. Those lookups are batched and time-limited, which fixes the iPhone's "Couldn't sync" timeouts (#457).
- Opening the database from two places at once (the app, its background work, the MCP server) no longer fails occasionally (#455).

### Windows
- With two Google accounts connected, shared calendar events are no longer re-saved on every poll; events are merged across accounts once per poll, today only (#458).

### iPhone
- Ready for TestFlight: new bundle ID `com.mpointer.whimbrel.mobile`, a privacy manifest, and the version now follows the Mac and Windows apps (#459).

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
