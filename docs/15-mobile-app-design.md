# 15 — Mobile App: Design

**Status:** Draft for review, 2026-10-17. Decisions recorded the same day (§10): iPhone first, native, on the free plan, approvals from the phone. Updated 2026-10-23 for the open-source commercial model ([16-open-core-commercial-model.md](16-open-core-commercial-model.md)): there is no hosted model, and push to the phone is a planned Cloud feature. Phase 4 of [05-roadmap.md](05-roadmap.md).
**Scope:** What the Whimbrel phone app does, how it fits the existing sync and routines architecture, what has to change in `core/` and `server/`, and the milestones to build it. The per-milestone implementation plans go under `docs/superpowers/plans/` as each milestone starts.
**Reconciled against:** FR-MOB-1…5 ([02-functional-requirements.md](02-functional-requirements.md), [08-littlebird-parity-spec.md](08-littlebird-parity-spec.md) §7), NFR-PRT-1/2, the primary-device routines spec (`docs/superpowers/specs/2026-10-08-primary-device-routines-design.md`), device enrollment (`docs/superpowers/specs/2026-07-29-device-enrollment-pairing-design.md`), `core/sync-engine/src/content.rs`, and a review of Littlebird's and Granola's phone apps on 2026-10-17 (§1).
**Renamed:** Kestrel became Whimbrel on 2026-10-21 (`docs/superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md`). Product names below say Whimbrel; identifiers that key user data, such as the iOS bundle id `com.mpointer.kestrel.mobile` and the sync relay's workers.dev host, keep the old name, as that design requires. New builds will reach the relay at `sync.whimbrelapp.com` once that custom domain is attached.
**Mockups:** six phone screens on the [Whimbrel Mobile design canvas](https://claude.ai/artifact/XdSSzJnTbrEQnFvJAePh5W): Today, Ask, Recording, Meeting, Approval, Pair this phone.

---

## 0. The honest summary

Littlebird's phone app is a **companion**: chat over memory the desktop captured, routines you can view and run, email sending, and in-person meeting recording on iPhone. It does not capture context on the phone, apart from an opt-in Android accessibility reader. That is the right shape for Whimbrel too, and FR-MOB-1…4 already say so.

The hard part is not the screens. **Littlebird's memory lives on its servers, so its phone app is a thin client. Whimbrel's memory lives on your desktops, so the phone has nothing to ask unless it becomes a full sync peer.** This design makes the phone exactly that: an enrolled device that pulls the encrypted timeline from the account's Turso database, decrypts it locally, and searches it locally. The sync layer for this already exists and runs live between Macs and Windows. What doesn't exist is a mobile build of the core, push, and a way to approve actions from a device that isn't the primary one.

Three consequences you should know before approving this:

1. **Anything that acts runs on your primary desktop, so it waits for that desktop to be awake.** Approving an email on the phone is instant; the email is sent when the Mac next syncs. With a MacBook asleep in a bag, that could be hours. The app says so plainly (§5.3), and a cloud runner stays out of scope.
2. **Users who run only Ollama get little on the phone.** Chat and phone-meeting summaries need a hosted model. Synced Anthropic, OpenAI or Gemini keys work on the phone with no setup. There will be no hosted Whimbrel model to fall back on (doc 16 rules out reselling AI usage), so an Ollama-only user gets transcripts, meeting notes and briefings captured elsewhere, but no chat.
3. **The app is free (decided, §10).** That matches Littlebird's free mobile chat. On the hosted relay a free account syncs one computer plus one phone; on a relay the user runs, sync is unlimited. Each free account on the hosted relay costs a Turso database. Under doc 16, **push notifications are a planned Cloud feature**, so on the free plan the phone shows new briefs, summaries and approvals when it is opened or refreshes in the background, not as they happen.

Decided: **iPhone first**, not Android as the roadmap said (§6), built as a native SwiftUI app on the existing `core/ffi` C interface the Mac app already uses. That's roughly three months for one developer with AI help, against the rough estimates in §8.

## 1. What Littlebird and Granola ship on phones (checked 2026-10-17)

| | Littlebird | Granola |
|---|---|---|
| Platforms | iPhone + iPad (iOS 17+, since March 2025, v0.86.57, about weekly releases); Android (10K+ installs) | iPhone (iOS 18+, since April 2025); Apple Watch (July 2026) |
| Store rating | 4.3 from 63 ratings (iOS); 4.3 from 54 (Android) | 4.96 from 14,551 ratings |
| What it is | Chat and routines over memory the desktop captured | Meeting capture first |
| Chat over memory | Yes, the main screen. Resume conversations started on the Mac | Chat with your notes |
| Routines | View, create, edit, run; open a report from a notification | No |
| Meeting capture | In-person meetings with the iPhone mic. Not online meetings ("System audio is not available on iOS") | In-person meetings and outbound phone calls made from the app; one tap from the Lock Screen; Watch complication |
| Email | Write and send from the phone | Share summaries |
| Other | Read aloud, dictation, image attachments, projects, PDFs and sheets open in-app, usage/credits screen. Android: "Ask Littlebird" in the text-selection menu | Lock Screen and Home Screen widgets, start notes from Google Calendar |
| Phone screen capture | No on iOS. Android: optional Accessibility Service, off by default | No |
| Where data lives | Littlebird's cloud (AWS US-East). Phone transcripts upload | Cloud; audio deleted after transcription |
| Navigation | Drawer (Favourites, Recents) plus a Chat / Routines toggle | Notes list plus record |

Sources: App Store listings and version history (ids 6737920045 and 6739429409), Google Play `com.genos.littlebird`, docs.littlebird.ai (meeting notes, privacy, changelog July 24–Aug 28, 2026), littlebird.ai/download and /pricing, docs.granola.ai iOS help, TechCrunch 2026-07-28. Reviews of Littlebird's phone app are thin. Complaints are about unresponsive buttons on one Android phone, audio playback that "hardly ever works", and getting data from phone to laptop.

Two takeaways:
- Littlebird has added features faster than Whimbrel can match: 25 iOS releases in five months. Don't chase its breadth. Compete on what it structurally can't do: private memory, on-device transcription, and approvals.
- Granola's rating count shows where phone demand actually is: **capturing in-person meetings in one tap**. Littlebird treats recording as secondary. Whimbrel should treat it as a headline feature alongside chat.

## 2. What the Whimbrel phone app is

**A pocket companion that is also a full peer on your timeline.** It does four things:

1. **Brief.** Today: the daily brief, your next meeting with its prep, and anything waiting for your OK.
2. **Ask.** Chat over everything your desktops captured (meetings, screen memory, email, calendar), with citations, from your pocket.
3. **Approve.** Review and approve or deny the emails and calendar changes your routines drafted, including from the Lock Screen.
4. **Record.** Capture in-person meetings with the phone mic, transcribed on the phone, so they join the same timeline as desktop meetings, labelled "iPhone".

What it is **not** (unchanged from FR-MOB-4 and the roadmap's out-of-scope list):
- No screen capture on the phone, including no Android Accessibility Service reader. Google Play restricts that API to accessibility uses, and it breaks the stance that the phone only adds meetings, chat and approvals.
- No capture of online calls or phone calls. iOS doesn't allow it, and recording phone calls brings the most consent-law exposure.
- Not a remote control for the desktop: it reads its own decrypted copy, so it works with every desktop off.

## 3. Features: v1, later, never

| Feature | Littlebird | Whimbrel | Notes |
|---|---|---|---|
| Daily brief and routine outputs | Yes | **v1** | Read the synced `routine_run` rows |
| Next meeting with prep | Partial ("Coming up") | **v1** | Synced `meeting_prep` plus calendar items |
| Meeting list, summary, decisions, action items, transcript | Yes | **v1** | Summary fields already sync on the `meeting` row |
| Chat over memory with citations | Yes | **v1** | Local retrieval over the phone's replica (§5.2) |
| Dictation and read aloud | Yes | **v1** | System speech recognition and text-to-speech. Cheap |
| Push for briefs, summaries and approvals | Yes | **v1, Cloud plan** | Content-free push; text is decrypted on the device (§5.4). Free accounts refresh in the background instead |
| Approve or deny routine write actions | Partial (send email) | **v1** | Needs new synced rows (§5.3). Executed by the primary desktop |
| Approve from the Lock Screen / notification | No | **v1** | Notification action that requires Face ID |
| In-person meeting recording | iPhone | **v1** | On-device whisper, live transcript, Live Activity |
| Record from Lock Screen widget / Action Button / Siri | No | **v1.1** | Granola's most-praised feature. App Intents make it small work |
| "Run now" for a routine | Yes | **v1.1** | Sent as a request; the primary desktop runs it |
| Create and edit routines | Yes | **Later** | Routine definitions would have to sync first |
| Resume desktop chats on the phone | Yes | **Later** | Chat threads are local-only today; needs a `conversation` row type |
| Share extension / "Ask Whimbrel" on selected text | Android only | **Later** | Share extension on iOS, PROCESS_TEXT on Android |
| Speaker labels on phone recordings | Unclear | **Later** | sherpa-onnx diarization isn't built for mobile yet |
| Apple Watch recording | No | **Later** | After Granola's lead shows it's worth it |
| Shared links | Yes | **Later** | FR-WEB-4 snapshots, built for web first |
| Projects, canvases, generated images, artifacts | Yes | **No** | Whimbrel has no such concepts |
| Phone screen capture | Android, opt-in | **No** | FR-MOB-4 |

## 4. Screens and navigation

Four tabs plus a record button that's always reachable. Settings sits behind the avatar on Today. Mockups are on the canvas linked at the top.

- **Today:** greeting and date; **Needs your OK** cards (each shows the draft, the routine that made it, and Approve / Review); the **brief** card (latest daily brief, collapsible); **Up next** (next meeting, time, attendees, a "Prep ready" chip that opens the prep); **Recent meetings**, each labelled with the device that captured it. A sync line at the bottom ("Synced 2 min ago · Mac last seen 4 min ago") makes freshness visible, because Littlebird users complain about not trusting it.
- **Ask:** chat with streaming answers. Each answer has citation chips naming the source and device ("Meeting · Acme pricing sync · Tue", "Screen · MacBook · Pricing.xlsx"); tapping one opens the cited moment. The composer has dictation and send; each answer has read aloud and copy. A subtle line shows which model answered ("Claude Sonnet · your key").
- **Meetings:** list grouped by day, filter by device, search. Detail has Summary (TL;DR, decisions, action items, open questions) and Transcript tabs, plus where it was captured.
- **Routines:** routine runs newest first (brief, prep, recap, weekly), each opening its output. v1.1 adds "Run now".
- **Record:** a large button in the tab bar. Recording screen shows a timer, live waveform, live transcript, "Transcribed on this iPhone · audio never leaves it", and Pause / Stop. A Live Activity keeps the timer and Stop on the Lock Screen and Dynamic Island. On stop: "Summarizing…", then the meeting detail.
- **Approval sheet:** full draft (to, subject, body), the routine and meeting that produced it, Edit, Deny, **Approve & send**, and an honest delivery line: "Sends from your MacBook Pro (primary device) · last seen 4 min ago". If the primary has been offline for more than 15 minutes: "Your Mac is offline. This will send when it next connects."
- **Onboarding:** sign in with Google → "Approve this phone from your Mac" (the existing approval flow, with a notification on the desktop) → initial sync with a progress bar and an honest size estimate → notification and microphone permissions, asked when first needed rather than up front.

Look: the same `themes/` tokens as desktop (default cream `#F7F2E7`, terracotta accent `#A8502E`, dark `#1C1712` / `#C97B54`), serif headlines over system sans, 12pt card radius, Dynamic Type, 44pt minimum touch targets.

## 5. Architecture

### 5.1 The phone is a sync peer

- **Enrollment:** unchanged protocol. The phone generates its X25519 device keypair (private key in the Secure Enclave–backed Keychain), signs in with Google, calls `POST /v1/auth/google` and shows "pending". The primary or any approved desktop shows "Approve iPhone?" and wraps the group key for it. The phone polls, unwraps, and becomes an ordinary device that can be renamed or revoked from any other device.
- **New: a comparison code.** Both screens show a short code derived from the phone's public key, for example the first 50 bits of its SHA-256 shown as five groups of two characters, `K7 · 4Q · 92 · 9K · 3M` (30 bits would be cheap to grind a colliding key for). The Mac also refuses to approve a pending device whose code another pending device shares. Today the desktop approves any pending device on the account. With the code, the user approves the phone in their hand rather than one that somebody else's stolen Google session enrolled. It costs a few lines on each side and no protocol change.
- **Server change:** the Worker verifies Google ID tokens against the Mac's OAuth client today. It must accept the iOS client ID as a second audience (and an Android one later). The `ALLOWED_GOOGLE_EMAILS` allowlist still applies, which is fine for dogfooding.
- **Pull:** the phone runs the same `sync-engine` pull into its own SQLite store (`memory`), the same two-pass parent and dependent ordering, and the same legacy row types. Settings sync gives it the user's provider keys and preferences, `routines.primaryDeviceId` included.
- **Storage budget:** meetings, transcripts, routine runs, prep, and calendar items sync in full: they're small. **Screen-capture items can be large.** Before M4.0, measure a dogfood account (`SELECT kind, count(*), sum(length(text)) FROM items GROUP BY kind`). Default: pull screen captures for the last 30 days and keep older ones out of the phone's index. This reuses FR-SYNC-6 (selective sync) from the receiving side. To confirm: whether the relay keeps old `sync_row` data long enough to fetch older captures on demand.
- **Push from the phone:** only its own rows: `meeting` and `items` (transcript kind) for phone recordings, plus the new approval rows (§5.3). Chat threads stay local, as on desktop.

### 5.2 Ask on the phone

- **Retrieval** runs locally over the replica. Keyword search works as soon as rows land. Embeddings are computed by `memory`'s local embedder in the background while the phone is charging, so semantic search catches up overnight. Measure the model size and battery cost on a real phone in M4.2; if it's too heavy, keep keyword search plus recency on mobile and say so.
- **Model:** `core/llm` clients with the provider keys that already sync end-to-end encrypted (`apiKey.anthropic`, `apiKey.openai`, …). No new key handling. Ollama doesn't run on a phone; Ollama-only users see an honest empty state that offers to add a key. There is no hosted model to offer ([16-open-core-commercial-model.md](16-open-core-commercial-model.md)).
- **Spend caps:** the phone uses the same `ledger` defaults, enforced per device.

### 5.3 Approvals from a device that isn't the primary

The primary-device spec said pending runs "never leave the device" and approving happens there. FR-MOB-2 needs the opposite, and the user approved the change on 2026-10-17; the spec now carries an amendment pointing here. Two new encrypted row types:

- **`approval_request`**, pushed by the primary when a run reaches `pending_approval`: run id, action kind (email send, calendar write), the full preview (recipients, subject, body or event fields), a SHA-256 hash of the exact draft, the routine and source meeting, and an expiry.
- **`approval_decision`**, pushed by any enrolled device: request id, `approve` or `deny`, the draft hash it approved, an optional edited draft (which becomes a new hash), and the time.

The primary applies a decision only if the request is still pending, unexpired, and the hash matches the draft it would send. Otherwise it records the decision as stale and the phone shows "This draft changed. Review again". It then executes and pushes the terminal `routine_run` as today, so every device shows "Sent". The first decision wins; later ones are ignored. Any approved device can already do anything the desktop can, so this adds no new trust; revoking a device stops its decisions as it does its reads.

The phone never sends email itself in v1: connector OAuth tokens live in the desktop's Keychain and don't sync. That's deliberate. It keeps write actions on the one device the user picked, and it's why §0's latency caveat exists.

### 5.4 Push that the server can't read

Push runs on servers the company operates (APNs credentials, the poke endpoint), so under doc 16 it is part of the planned Cloud plan. The design below is unchanged; the Worker checks the account's plan before fanning out a poke. Free accounts rely on background app refresh to pull new rows.

- **Worker additions:** `POST /v1/push/register` (device id, platform, token; authenticated like `/v1/devices`), and `POST /v1/push/poke` (an approved device asks the Worker to wake the account's other devices). The payload carries only a category (`approval`, `routine`, `meeting`). It never carries content, names or subjects. Tokens go in KV next to the device record and are deleted on revoke.
- **Who pokes:** the primary, after pushing an `approval_request` or a successful brief, and any device after a meeting's summary syncs.
- **iOS:** an alert push with `mutable-content`. A Notification Service Extension pulls the newest rows for that category from Turso, decrypts them with the group key from the shared Keychain group, and rewrites the notification ("Approve follow-up to Dana Reyes?"). Extensions get about 24 MB of memory and 30 seconds. If decryption can't finish, the user sees the generic text "You have something to review in Whimbrel". The extension needs a slim build: `core/sync` crypto plus a libsql HTTP client, without the full store.
- **Android (later):** FCM high-priority data message → the app decrypts in a short background job and posts the notification.
- Approve and Deny notification actions use `.authenticationRequired`, so an approval from the Lock Screen still needs Face ID.

### 5.5 Recording in-person meetings

- **Capture:** iOS `AVAudioSession` record category with the background audio mode, so recording continues with the screen locked. ActivityKit Live Activity for the timer and Stop. Android (later): a foreground service of type `microphone`.
- **Transcription:** on the phone, with the same `transcribe` crate (whisper.cpp, `ggml-tiny.en`). It already compiles for Android in CI, and iOS is the same toolchain. Live chunked transcription reuses `core/transcribe/src/live.rs`, the change that took desktop stop-to-summary from 3 min 17 s to 43 s, so a phone meeting's transcript is ready when it ends.
- **Speakers:** unlabelled in v1. `diarize` (sherpa-onnx) is excluded from mobile builds until its ONNX Runtime builds for iOS and Android.
- **Summary:** generated on the phone with the synced provider key, through the same `core/api` summary path as desktop. With no hosted model, the meeting syncs with its transcript and the summary says "Summary needs a model". It stays that way rather than being handed to the desktop, because a desktop can't rewrite a row another device owns.
- **Audio:** deleted after transcription by default, with a setting to keep it on the phone. It never syncs.
- **Consent:** a visible recording indicator, and the same "you're responsible for consent" note as desktop the first time.

### 5.6 Shell technology

| | Native SwiftUI over `core/ffi` (recommended for iOS) | Tauri v2 mobile, reusing the Windows React UI |
|---|---|---|
| Core access | `core/ffi` C ABI, the same one the Mac app uses today | `core/api` linked directly, as on Windows |
| UI reuse | Mac SwiftUI components and the Swift theme loader | Windows React components, which are desktop layouts and need rework for phones |
| Live Activity, widgets, App Intents, notification extension, background audio | Native, first-class | All need native Swift plugins or separate native targets anyway |
| Android | A second native shell later (Kotlin/Compose over UniFFI or JNI) | The same app |
| Risk | Two mobile UIs eventually | Tauri mobile is young; the hard parts end up native regardless |

The parts that make a phone app worth having (Lock Screen recording, Live Activity, push you can approve from, the decrypting extension) are all native-only APIs. With Tauri, that's still native code plus a web layer on top. Go native.

## 6. Why iPhone first (decided 2026-10-17)

The roadmap said Android first. The user chose iPhone first, for these reasons:
- Whimbrel's desktop users are Apple silicon Mac owners, who are mostly iPhone owners. Windows is still early access.
- The Swift shell, `core/ffi`, the theme loader, Keychain code and Apple developer account (signing, notarization, APNs) already exist. Android starts from zero UI and needs the `diarize`/`ffi` build split first.
- Both competitors launched on iPhone first, and Granola's 14,551 ratings show where the demand is.

Android follows on the same core and server work (§8).

## 7. Changes required outside the app

| Area | Change |
|---|---|
| `core/` | Cargo feature to build `ffi` and `api` without `diarize`. `aarch64-apple-ios` (and simulator) CI build of `ffi`. A slim crate or feature for the notification extension (crypto plus HTTP pull, no store). |
| `core/sync-engine` | `approval_request` and `approval_decision` row types, pushed and pulled like `routine_run`. Optional time window for pulling `items` of kind `capture`. |
| `core/routines` / `core/api` | The primary applies pulled decisions (hash check, expiry, first wins) and emits requests when a run is pending. |
| `server/` | Accept iOS (later Android) Google OAuth client IDs. `/v1/push/register` and `/v1/push/poke`. APNs credentials as Worker secrets. Delete push tokens on revoke. |
| Desktop apps | "Approve iPhone?" prompt reachable from a notification, showing the device's comparison code (§5.1). "Sent from approval on iPhone" in run history. Optional: a primary-offline warning in Settings. |
| Specs | Done with this doc: an amendment note in `2026-10-08-primary-device-routines-design.md` for §5.3, FR-MOB-1 (iPhone first, Free and Pro), the Phase 4 roadmap line, and the Free plan's device limit in the pricing proposal. |

## 8. Milestones (iPhone)

Effort is a rough range for one developer with AI help, before any measurement. Treat it as an order of magnitude.

| Milestone | Scope | Exit gate | Rough effort |
|---|---|---|---|
| **M4.0 Foundations** | iOS build of `core/ffi` in CI, `diarize` feature split, iOS OAuth client, enrollment and approval from the Mac, pull-only sync into the phone store with the capture window | The phone enrolls, is approved from the Mac, and shows a count of synced meetings that matches the Mac's | 2–3 weeks |
| **M4.1 Read** | Today, Meetings, Routines (read-only), sync status, themes | A desktop meeting's summary is readable on the phone within 2 minutes of the meeting ending (app open) | 2 weeks |
| **M4.2 Ask** | Local retrieval, chat with synced keys, citations into meeting and screen moments, dictation, read aloud | 20 dogfood questions answered with correct citations, compared with the same questions on the Mac | 2 weeks |
| **M4.3 Push and approvals** | Worker push endpoints, APNs, notification extension, `approval_request` and `approval_decision`, the primary applying decisions, Lock Screen approve with Face ID | Approve a routine's follow-up email from the Lock Screen; the Mac sends it; every device shows "Sent". The server sees no content (checked by logging the push payloads) | 3 weeks |
| **M4.4 Record** | Phone recording, live transcription, Live Activity, phone-side summary, sync | A 30-minute in-person meeting recorded on the phone appears on the Mac with transcript and summary, labelled iPhone (the Phase 4 gate) | 2–3 weeks |
| **M4.5 Reach** | Lock Screen and Home Screen widgets (next meeting, record), App Intents (Siri, Action Button: "Record a meeting"), "Run now" | Record started from the Lock Screen in one tap | 2 weeks |

That's about 13–15 weeks to an iPhone app that meets the Phase 4 exit gate and FR-MOB-5. Android adds roughly 8–10 weeks for a second shell on the same core and server work.

## 9. Risks

1. **The primary desktop is asleep.** Approvals and "Run now" wait on it. Mitigations: show "last seen" everywhere it matters, and recommend an always-on desktop as primary in onboarding. If dogfooding shows it really hurts, the only real fix is an end-to-end encrypted cloud runner. That's a separate design and a big change to the local-first story, not something to slip in.
2. **Ollama-only users get little** (§0). Measure how many users that is before investing in mobile marketing.
3. **Phone storage and battery** from pulling screen memory and embedding it. The 30-day window and charging-only embedding are guesses; M4.0 and M4.2 measure them.
4. **The notification extension's 24 MB limit** may not fit libsql plus decryption. The fallback is generic notification text, which is still private, just less useful.
5. **App Review:** background audio and microphone use need a clear user-facing reason ("records in-person meetings you start"); the Live Activity and indicator help. Low risk if recording is never automatic on the phone.
6. **Chat doesn't follow you between devices**, and Littlebird's does. Users will notice. A synced `conversation` row type is the fix and is listed under Later.
7. **Free accounts now sync.** Every Free user who adds a phone gets a Turso database, and Turso's free tier covers 100. Watch the count once sign-in opens, and price it in before then.
8. **The sign-in allowlist** means only the owner can use it until sign-in opens with billing. Fine for dogfooding; not shippable to others until then.

## 10. Decisions (user, 2026-10-17)

1. **iPhone first.** Android follows (§6, §8).
2. **Native shells.** SwiftUI on iOS, Kotlin/Compose on Android later (§5.6).
3. **Mobile on the free plan.** Free syncs one computer plus one phone on the hosted relay, end-to-end encrypted, and any number of devices on a relay the user runs. Revised 2026-10-23 by [16-open-core-commercial-model.md](16-open-core-commercial-model.md): the Pro plan and hosted AI are gone, and push to the phone moves to the planned Cloud plan.
4. **Approvals from the phone are allowed.** The primary-device routines spec is amended as §5.3 describes. The primary still executes.
