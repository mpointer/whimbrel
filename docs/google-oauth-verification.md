# Google OAuth verification

This is the working file for getting Whimbrel's Google OAuth app verified (issue #761), so that users stop seeing the "unverified app" warning and the Gmail and Calendar scopes are no longer capped.

All the Whimbrel clients (Mac, Windows, iPhone, Android) live in one Google Cloud project, so one consent screen and one verification cover all of them.

## Status (2026-11-13)

- **Out of Testing:** 2026-10-29. The app is "In production" but unverified.
- **Submitted for verification:** 2026-10-30, with the demo video at https://youtu.be/UrIZPJp29fc (unlisted). The scope list submitted then came from an earlier version of this file, which listed `gmail.send`; check the Console's Data Access page to confirm it.
- **Google replied on 2026-11-02** with an "[Action Needed] Your Google APIs Verification Request" email from its Third Party Data Safety Team. The review does not continue until Mike replies to it or updates the request in Cloud Console. It has no reply yet. Section 5 is built on that checklist.
- **The scope problems are fixed in code, on `main`, but not released.** Mac #779 and Windows #786 narrowed the public builds. Both merged after the latest release (v0.9.71, 2026-11-12), so **v0.9.71 and earlier public builds still request the old scopes** (`calendar.readonly`, `gmail.readonly`, `gmail.compose`). Anyone who connected on those builds keeps the wider grant until they reconnect.
- **CASA is no longer needed.** The app's verified project now requests no restricted scope (section 3). Gmail reading exists only through the user's own Google Cloud project.
- **Limited Use is enforced in code** (#785, #790). Two shell parts are still open (section 4). The privacy page does not yet say any of this.

## 1. Scopes each client requests

A scope counts as "requested" only if it is in the `scope` parameter sent to Google's consent page. Constants that are only compared against a grant are listed separately at the end of this section. All line numbers are on `origin/main` at `0f073c86`.

### Mac app (`apps/macos`)

OAuth client: a "Desktop app" client. The ID and secret come from `GOOGLE_OAUTH_CLIENT_ID` and `GOOGLE_OAUTH_CLIENT_SECRET` in `apps/macos/Secrets.xcconfig` (gitignored; template `apps/macos/Secrets.xcconfig.example:9-10`). They are written into `Info.plist` as `GoogleOAuthClientID` (`apps/macos/Whimbrel/Info.plist:34`) and read by `apps/macos/Whimbrel/Support/SyncConfig.swift:13`. The flow is Authorization Code with PKCE over a loopback redirect on 127.0.0.1, shown in `ASWebAuthenticationSession` (`core/api/src/oauth.rs:1-8`, `:59`, `:237-238`).

| Feature | Scopes requested | Where |
|---|---|---|
| Sync sign-in | `openid email` | `apps/macos/Whimbrel/Support/SyncState.swift:208` |
| Google Calendar connector | `openid email https://www.googleapis.com/auth/calendar.events` | `GoogleConnectorScopes.calendar`, `apps/macos/Whimbrel/Support/GoogleConnectorClient.swift:46-47`; used at `CalendarConnectorState.swift:42` |
| Gmail connector, app's own client | `openid email https://www.googleapis.com/auth/gmail.send` | `GoogleConnectorClient.swift:50-51`; used at `GmailConnectorState.swift:64` |
| Gmail connector, user's own Google Cloud client | `openid email gmail.readonly gmail.send` | same line, `customClient` branch |

The own-client branch applies only when the user saves their own Desktop OAuth client in Settings, Connectors, "Use your own Google Cloud project" (`apps/macos/Whimbrel/UI/Settings/ConnectorsSettingsPane.swift:67-74`). `GoogleOAuthClient.active()` (`GoogleConnectorClient.swift:23`) returns that client when one is saved and the built-in client otherwise, and `isCustom` (`:15`) decides whether reading is requested. Sync sign-in never uses the custom client.

### Windows app (`apps/windows`)

OAuth client: the same Desktop client as the Mac. The values are `google_oauth_client_id` and `google_oauth_client_secret` in `apps/windows/src-tauri/Secrets.toml` (gitignored; template `Secrets.toml.example:10-11`, key name at `src-tauri/src/syncing/config.rs:13`). The template explains why: the sync server checks ID tokens against one `GOOGLE_CLIENT_ID`. The connectors reuse the same client when no custom client is saved.

| Feature | Scopes requested | Where |
|---|---|---|
| Sync sign-in | `openid email` | `apps/windows/src-tauri/src/syncing/config.rs:44` |
| Google Calendar connector | `openid email https://www.googleapis.com/auth/calendar.events` | `apps/windows/src-tauri/src/connectors/accounts.rs:28` (`CALENDAR_SCOPE`) |
| Gmail connector, app's own client | `openid email https://www.googleapis.com/auth/gmail.send` | `accounts.rs:39` (`GMAIL_SCOPE`) |
| Gmail connector, user's own Google Cloud client | `openid email gmail.readonly gmail.send` | `accounts.rs:41-42` (`GMAIL_READ_SCOPE`); chosen by `Kind::scope(own_client)`, `accounts.rs:88-94` |

The own-client setting is in Settings, Connectors, Advanced (`apps/windows/ui/src/settings/GoogleClientCard.tsx:13`, `:55`; module doc at `connectors/google_client.rs:1-10`). The tests at `accounts.rs:370` and `:428-431` assert that the public scope strings contain none of `gmail.readonly`, `gmail.compose`, `gmail.modify` or `mail.google.com`.

### iPhone app (`apps/mobile/ios`)

OAuth client: an "iOS" client registered for bundle ID `com.mpointer.whimbrel.mobile`. The ID is `GOOGLE_IOS_CLIENT_ID` in `apps/mobile/ios/Secrets.xcconfig` (template line 6), written into `Info.plist:28` via `project.yml:98`. It has no secret. The sync server knows it as its own `GOOGLE_IOS_CLIENT_ID` (`server/src/googleAuth.ts:30`).

| Feature | Scopes requested | Where |
|---|---|---|
| Sync sign-in | `openid email` | `apps/mobile/ios/Whimbrel/GoogleSignIn.swift:32` |

The iPhone app does not connect Gmail or Calendar.

### Android app (`apps/mobile/android`)

This mapping comes from library behaviour and the server code, not from a scope string in the app, so treat it as inferred and check it against the Console's client list.

OAuth client: `GOOGLE_WEB_CLIENT_ID` in `apps/mobile/android/secrets.properties` (template line 1), compiled into `BuildConfig` at `app/build.gradle.kts:83`. It is a Web-type client used as the "server client ID", so the ID token's audience is that client. The sync server matches it against its `GOOGLE_ANDROID_CLIENT_ID` (`server/src/googleAuth.ts:31`, `server/src/index.ts:280`). Google Play services also needs an Android-type client in the same project with the package name and signing SHA-1; no value for that client appears in the code.

| Feature | Scopes requested | Where |
|---|---|---|
| Sync sign-in, Credential Manager | `openid email profile` (what `GetGoogleIdOption` asks for) | `apps/mobile/android/app/src/main/java/com/mpointer/whimbrel/mobile/auth/GoogleSignIn.kt:222` |
| Sync sign-in, legacy fallback | `openid email profile` (`GoogleSignInOptions.DEFAULT_SIGN_IN` plus `requestIdToken` and `requestEmail`) | `GoogleSignIn.kt:217` |
| Silent token renewal | `audience:server:client_id:<web client>`: an ID token only, no API access | `GoogleSignIn.kt:154-157` |

Android is the only client that brings in `profile` (inferred: `GetGoogleIdOption` and `DEFAULT_SIGN_IN` ask for the basic profile, but the code does not spell out the scope). The Android app does not connect Gmail or Calendar.

### Bundled connectors (`mcp-servers/`)

The connectors request no scopes and hold no client ID. The host app runs the consent flow, then starts the connector with the access token in an environment variable, `GMAIL_ACCESS_TOKEN` or `GOOGLE_CALENDAR_ACCESS_TOKEN` (`apps/windows/src-tauri/src/connectors/node.rs:17-19`; the Mac uses the same names). The API calls they make decide which scopes are needed:

| Connector tool | Google API call | Narrowest scope that allows it | Called by shipped code? |
|---|---|---|---|
| `sync_messages` (`mcp-servers/gmail/src/index.ts:74`, `gmail.ts:388`) | `users.getProfile`, `users.history.list`, `users.messages.get` | `gmail.readonly` | Only for an account that holds a read grant: the user's own Google Cloud client, or a grant made before the narrowing. The Mac poller skips send-only accounts (`GmailPollingService.swift:44`); Windows checks `can_read_mail` (`accounts.rs:51`). |
| `search_threads` (`index.ts:24`, `gmail.ts:172`) | `users.threads.list`, `users.threads.get` | `gmail.readonly` | Same rule. Hidden from send-only accounts (`GmailReadAccess.readTools`, `GoogleConnectorClient.swift:61`). |
| `get_thread` (`index.ts:41`, `gmail.ts:331`) | `users.threads.get` | `gmail.readonly` | No caller in `apps/` or `core/` |
| `create_draft` (`index.ts:58`, `gmail.ts:488`) | `users.threads.get`, `users.drafts.create` | `gmail.compose` | **No caller in `apps/` or `core/`.** Offered only to an old grant that holds `gmail.compose` (`GoogleConnectorClient.swift:70-77`). |
| `send_email` (`index.ts:8`, `gmail.ts:122`) | `users.messages.send` | `gmail.send` | Yes: emails the user approves (`RoutineScheduler.swift:313-314`, `CoreBridge.swift:542`, `GmailSelfSend.swift:42`; Windows `connectors/write.rs:112`) |
| `list_events` (`mcp-servers/google-calendar/src/index.ts:8`, `calendar.ts:85`) | `events.list` on `primary` | `calendar.events` (or a read-only events scope) | Yes: connect-time check (`CalendarConnectorState.swift:271`) |
| `sync_events` (`index.ts:40`, `calendar.ts:135`) | `events.list` on `primary`, with sync tokens | same | Yes: calendar polling on both desktops (`CoreBridge.swift:4426`, Windows `connectors/calendar.rs:43`) |
| `create_event` (`index.ts:24`, `calendar.ts:109`) | `events.insert` on `primary` | `calendar.events` | Yes: approved routine actions (`core/api/src/write_action.rs:108`, `RoutineScheduler.swift:284`, Windows `write.rs:111`) |

### Constants that are checked, not requested

- `gmail.send`: `RoutineScheduler.swift:56` and `write.rs:32`. Before sending, the app checks that the grant covers it. A `gmail.compose` grant also passes, because core treats it as a superset (`core/api/src/oauth.rs:436`), so older grants keep sending.
- `calendar.events`: `RoutineScheduler.swift:55` and `write.rs:29`, the same check before creating an event. A grant with `calendar.readonly` plus `calendar.events` from before the change still passes (`oauth.rs:432`).
- `GmailReadAccess.canRead` (`GoogleConnectorClient.swift:63`) and `can_read_mail` (`accounts.rs:51`) decide reading from the scopes Google actually granted. A grant with no recorded scope counts as able to read, so older accounts keep working until they reconnect.

### Nothing else requests a Google scope

A search of `apps/`, `core/`, `server/` and `mcp-servers/` for `googleapis.com/auth`, `gmail.*` and `calendar.*` finds no other request. The remaining hits are the checks above, the grant-superset table at `core/api/src/oauth.rs:432-436`, tests, and comments.

## 2. Classification and whether each scope is needed

Gmail classes come from Google's Gmail scope table (https://developers.google.com/workspace/gmail/api/auth/scopes). Google's Calendar scope page does not print classes. The Calendar row follows Google's sensitive-scope page, which gives reading calendar events as an example of a sensitive scope (https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification). The Console assigns the class, so confirm each one on its Data Access page.

| Scope | Class | Requested by the app's own client | Needed? | Console action |
|---|---|---|---|---|
| `openid` | Non-sensitive | All clients | Yes. Sync identifies the account by its Google ID; the connectors label each connected account. | Keep. |
| `email` (`userinfo.email`) | Non-sensitive | All clients | Yes, for the same reason. | Keep. |
| `profile` (`userinfo.profile`) | Non-sensitive | Android only, implicitly | Not used: the server reads the account ID and email, not the name or photo. | Keep in the list because Android sends it. Low stakes either way. |
| `calendar.events` | Sensitive | Mac, Windows | Yes: reading events, and creating the events a user approves. | Keep. Option: `calendar.events.owned` is narrower; check its class in the Console before switching. |
| `gmail.send` | Sensitive | Mac, Windows | Yes, for the emails a user approves. | Add. |
| `calendar.readonly` | Sensitive | Nobody on a new build | No. | Remove. |
| `gmail.readonly` | **Restricted** | Nobody on a new build; only a user's own client asks for it | Not for the verified project. | Remove. |
| `gmail.compose` | **Restricted** | Nobody on a new build | No. Nothing creates drafts. | Remove. |

After the change, the full list for the consent screen is: `openid`, `email`, `profile`, `calendar.events`, `gmail.send`.

### The code change (done, unreleased)

- Mac #779 and Windows #786, on `origin/main`, in no release yet. Mac: `GoogleConnectorClient.swift:39-52`. Windows: `accounts.rs:28`, `:39`, `:41-42`.
- Tests: `apps/macos/WhimbrelTests/GoogleOAuthScopeTests.swift`, and the Windows asserts at `accounts.rs:370` and `:428-455`.
- Existing users keep their older, wider grants. Sending and creating events pass the superset checks above, and reading stays on for an account whose grant has `gmail.readonly`. Those grants end only when the user reconnects or revokes Whimbrel at myaccount.google.com/permissions. Removing a scope from the Console does not revoke anything already granted.
- Not done, optional: the unused `create_draft` and `get_thread` tools are still in `mcp-servers/gmail`. Deleting them would make the code an assessor reads match the scopes.
- Stale comment: `connectors/write.rs:30-31` still says `gmail.compose` "is what Whimbrel asks for".

## 3. Restricted Gmail scopes and the CASA assessment

**Not needed for the verified project.** Any app that requests a restricted scope must pass a Cloud Application Security Assessment (CASA) before Google approves it, and repeat it every 12 months. Google's 2026-11-02 email says: "Only apps requesting Restricted scopes are required to go through a third-party security assessment with a CASA authorized assessor before the Restricted scopes can be approved." The app's Cloud project now requests none. `gmail.send` and `calendar.events` are sensitive, and the rest are non-sensitive. Gmail reading is available only when the user brings their own Google Cloud project and their own OAuth client. That client belongs to the user, so Whimbrel's project never requests or receives the restricted scope.

**What to tell Google.** The app does not read Gmail with its own client. A user who wants Gmail reading creates a Desktop OAuth client in their own Google Cloud project, saves it under Advanced, and consents to their own app. Before saying this, re-check the code paths in section 1, and re-check after any change to `GoogleConnectorScopes` (Mac) or `Kind::scope` (Windows).

**Risks to watch.**

- Google could take the view that an app built to work with a restricted scope still counts, even through the user's own client. Nothing on Google's pages says so, but they do not address this case either. Say it plainly in the reply to Google, so the reviewer sees it up front.
- Until the release in Mike's list ships, the app's client still requests `gmail.readonly` and `gmail.compose` on v0.9.71 and earlier, and the unverified-app cap on restricted scopes applies to those users.
- If Whimbrel later wants Gmail reading in the verified app, CASA applies again. The earlier research is in the next paragraph.

**Earlier research, if CASA is ever needed again.** An assessor on the App Defense Alliance list does it (https://appdefensealliance.dev/casa/casa-assessors). Prices are not published. Third-party figures run from about $540 (TAC Security, Tier 2, "as of 2024", Switch Labs) to $3,000 (Leviathan Security's own page), so budget roughly $600 to $3,000 a year and get two quotes. Lab turnaround is 1 to 4 weeks, and Google's review can add several more. Google required CASA of Mimestream, a native Mac Gmail client with no server (https://mimestream.com/blog/casa-verified), so keeping data on the device does not avoid it.

## 4. Limited Use: what the code does

Google's required statement, word for word: "Whimbrel's use and transfer of information received from Google APIs will adhere to the Google API Services User Data Policy, including the Limited Use requirements." Everything below was read from `origin/main` at `0f073c86` (#785 and #790 merged).

**What counts as Google data.** Gmail messages and the threads and summaries built from them, and events read through the Google Calendar connector (`core/memory/src/limited_use.rs:7-37`). An Apple Calendar (EventKit) event does not count, even when a Google calendar sits behind it, and neither does screen text. A routine run counts, because its text is written from Gmail and Calendar (`:34`). An email with no recorded source counts as Gmail.

### Data Access

The app's own client asks for `openid email` (sign-in), `calendar.events` and `gmail.send` (section 1). It reads the user's primary Google Calendar events. It does not read Gmail with its own client. Gmail messages are read only through a Google Cloud client the user supplies, on request, in Advanced settings. Tokens are kept in the Keychain (Mac) or Credential Manager (Windows).

### Data Use

Google data is used only for features the user sees: the schedule, meeting preparation, recognising the meeting being recorded, conversations waiting on the user, answers, briefings and recaps, and the emails and events the user approves. No ads, no sale, no training of generalized models. Nothing is used for any other purpose.

### Data Transfer

Google data leaves the device only for a request the user made, and only to the AI provider the user chose, under that provider's no-training API terms. The rule is per chain link, so a fallback never sends text to a link that may not receive it (`core/llm/src/limited_use.rs:1-18`, `:62-77`):

| AI link | Gmail and Calendar text |
|---|---|
| Anthropic (user's key), OpenAI | Yes |
| Whimbrel AI (through Whimbrel's server to Anthropic, not stored) | Yes |
| Bundled local model | Yes |
| Ollama on loopback, not a `:cloud` or `-cloud` model | Yes |
| Ollama on another host, or a cloud model | Only with the user's setting `remote_ollama_google_data` (off by default) |
| Google Gemini (AI Studio key) | Only with the user's setting `google_ai_billing_confirmed` (off by default; the user states the key is on a paid project) |
| OpenRouter | Never. Requests also carry `data_collection: "deny"`. |
| TypeSafe reply ranking, calendar title check, topic judging, paid topic steps | Never. `TYPESAFE_GOOGLE_DATA_CONFIRMED` is `false` (`core/memory/src/limited_use.rs:20`); see `core/api/src/jobs.rs:255-264`, `title_check.rs:60`, and `core/llm/src/typesafe.rs:396`. Gmail threads and Google Calendar meetings are held back. |

Text a link may not receive is left out of its prompt. The request still runs, and the answer just does not use those items. Both shells save the two settings, both off by default (Windows `apps/windows/src-tauri/src/settings.rs:211-214`, `:245-246`; Mac `CoreBridge.swift:862-863`).

Other paths:

- **MCP clients.** `assemble_context` and the meeting tools leave out Gmail and Calendar text unless the client's token has the `google_data` scope (`core/mcp-server/src/lib.rs:315`, checks at `:355`, `:451`, `:477`, `:504`, `:606`). No token has it by default. **Neither shell can grant it yet:** the Mac scope list is `memory`, `meetings`, `transcripts` (`MCPClientsSettingsPane.swift:29`) and so is Windows (`commands/mcp.rs:22`). Windows PR #793 is open and a Mac PR is in progress. Until they land, no MCP client gets Google data, which is the safe direction.
- **Summaries and other prompts about a Google meeting.** A meeting that may come from Google Calendar is summarized, prepared for and speaker-named only by links that may receive it, and its title is replaced by "(title withheld)" elsewhere (`core/api/src/summary.rs:246-258`, `speaker_names.rs:145`, `meeting_prep.rs:262`, `profile_suggest.rs:147`, `core/llm/src/limited_use.rs:117-154`). If every link is restricted, the call fails with a note the shell shows ("google_data_withheld").
- **Answer provenance.** Each Ask answer is recorded as built from Google data or not (`answer_provenance`, `core/memory/src/limited_use.rs:55-64`, `:155`, `:167`). A later question that includes an earlier answer sends that answer only to a link that may receive it (`core/api/src/ask.rs:42-51`, `:519-523`). An answer with no record counts as Google data (test at `core/memory/src/limited_use.rs:353`).
- **Sync relay.** Data derived from Google data is end-to-end encrypted on the device before upload. The relay cannot read it.
- **People.** Nobody at Whimbrel reads it, except where the user shares it with support or the law requires it.

### Data Protection

OAuth with PKCE, so Whimbrel never sees a Google password. Access and refresh tokens are in the macOS Keychain or Windows Credential Manager. Data comes from Google over TLS and is stored in the app's local database in the user's folder, protected by the operating system's account and disk encryption (Whimbrel does not add its own database encryption; check before claiming more). The user's own OAuth client ID and secret are kept in the same credential stores, never in settings, settings sync or logs. Requests to an AI provider go over TLS, with known secrets redacted first (`memory::redact`).

### Retention and Deletion

What the code actually does:

- **Local data stays until the user deletes it.** The retention policy (`Store::apply_retention`, `core/memory/src/lib.rs:7145`) covers captures, meetings and conversations. It does not age out Gmail or Calendar items.
- **Messages Google reports as permanently deleted are removed** from the local store on the next sync (`delete_email_items`, `lib.rs:4150`; `delete_calendar_event_items`, `lib.rs:4003`).
- **Disconnecting a Google account deletes its tokens from the device.** For Calendar it also hides that account's events here and on the user's other devices (`forget_calendar_account`, `core/memory/src/calendar_presence.rs:113`); the rows stay in the database. For Gmail, the stored mail stays.
- **Deleting stored Gmail.** Windows has "Delete stored mail" per account (`gmail_delete_stored_mail`, `apps/windows/src-tauri/src/commands/connectors.rs:359-375`, which calls `Store::delete_email_data_for_account`, `core/memory/src/lib.rs:4171`). It removes the messages, threads, summaries, reply scores and pending jobs for that account. **The Mac has no such button.** `delete_email_data_for_account` has no Mac caller. On the Mac the way out is "Delete everything on this Mac" (Settings, Your data; `YourDataSettingsPane.swift`) after disconnecting the account, or removing the app's data folder. Delete everything keeps connected Google accounts connected (Task 15, 2026-11-17: `KeptGoogleAccounts` in `YourData.swift`), so an account still connected fetches its mail again into the new library; disconnect it first.
- **The app does not call Google's token-revoke endpoint** on disconnect (no call found in `apps/`, `core/` or `server/`). The grant stays at Google until the user removes Whimbrel at myaccount.google.com/permissions. The app can no longer use it once the tokens are gone.
- **Sync account:** deletable from the Devices settings; deleted automatically 14 days after a notice following six months with no device connecting (privacy page).

Two things to fix or word carefully before telling Google "delete in the app": the Mac Gmail gap, and the disconnect-versus-revoke difference. The draft text in section 6 says only what is true.

## 5. Verification checklist

This follows Google's 2026-11-02 email, plus the standard branding requirements. Status values: **Done** (evidence shows it), **Not done** (nothing has been done yet), **Mike to do** (needs Mike or a Console login, with the work described).

### Branding and domain

| Requirement | Status | Evidence |
|---|---|---|
| Home page on a verified domain | Done | https://whimbrelapp.com/ (`website/site/pages/index.html`). It describes the app, names Gmail and Google Calendar, and links the privacy policy from the header and footer (`website/site/layout.html:50`, `:90`). |
| Home page explains why the app wants Google data | Mike to do (optional) | The home page names the connectors but not what each is for. One line under "Connected to your tools": "Whimbrel reads your Google Calendar to prepare you for meetings, and it sends an email or creates an event only after you approve it." Do not mention Gmail reading: the public app does not read Gmail. Needs a website deploy. |
| Privacy policy on the same domain | Done | https://whimbrelapp.com/privacy/ (`website/site/pages/privacy.html`); "Google account data" starts at line 62. The content is out of date; see the privacy rows below. |
| Domain verified in Search Console | Done | A `google-site-verification=` TXT record was seen on `whimbrelapp.com` on 2026-11-12, and Search Console emailed on 2026-10-23 that the Domain property was verified. The Search Console owner must also be an Owner or Editor of the Cloud project (Mike to check). |
| Authorized domains | Mike to do | Console must list `whimbrelapp.com`. Add `futuremeanswered.com` only if a consent-screen contact email uses it. No redirect URI uses another host. |
| App name consistent | Done | "Whimbrel" everywhere users look: Mac `apps/macos/Whimbrel/Info.plist:16`, Windows `apps/windows/src-tauri/tauri.conf.json:3`, iOS `apps/mobile/ios/Whimbrel/Info.plist:12`, Android `res/values/strings.xml:3`, website `config.json`. |
| Logo consistent | Mike to do | Use the mark from `website/site/assets/icon-512.png`, resized to 120x120, the same as the app icon. Changing the logo starts a new brand review (2 to 3 business days). |
| Support email and developer contact | Mike to do | `hello@whimbrelapp.com` (website `config.json` `contactEmail`), entered in the Console. |
| Terms of service link | Done | https://whimbrelapp.com/tos/ (`website/site/pages/tos.html`). |

### Scope configuration

| Requirement | Status | Evidence |
|---|---|---|
| Code requests only the narrow scopes | Done in code, **not released** | #779 and #786 on `main`. v0.9.71 and earlier still request the old scopes. **Release to ship: Mike to do.** |
| Least privilege | Done in code; Console edit is Mike to do | `calendar.readonly`, `gmail.readonly` and `gmail.compose` are gone from the app's requests (section 1). They remain in the Console list until Mike removes them. |
| Production-ready features only | Done in code | No request is made for `gmail.compose`; nothing creates drafts. |
| Scope matching (the Console list equals what the apps request) | Mike to do | Console Data Access must become `openid`, `email`, `profile`, `calendar.events`, `gmail.send`, after the release ships. Editing it before the release would mismatch the live v0.9.71 requests, so do the release first. |
| Justification for each sensitive scope | Mike to do | Section 8 has the two paragraphs to paste (`calendar.events`, `gmail.send`). No restricted scope, so none is needed for Gmail reading. |

### Demo video

| Requirement | Status | Evidence |
|---|---|---|
| Public or unlisted on YouTube | Done | https://youtu.be/UrIZPJp29fc (the old video; replace it) |
| Consent screen with all scopes expanded ("Show all services") | Mike to do | Re-record. The old video did not expand them, and the scopes changed. |
| Each scope shown working | Mike to do | Re-record. The old script shows a `gmail.compose` draft step and Gmail reading. Section 9 has the new script: `gmail.send` and `calendar.events` in use. |
| Writes shown in the Google account | Mike to do | Show the new event in Google Calendar and the sent email in Gmail's Sent folder. |
| Client ID visible in the consent URL | Mike to do | Show the `client_id=` part of the `accounts.google.com` address. |
| Upload and link in the Console | Mike to do | Upload as unlisted, paste the link in the demo video field. |

### Reviewer access

| Requirement | Status | Evidence |
|---|---|---|
| Test credentials | Done (none needed) | Connecting Google needs no Whimbrel account. Say so in the reply. |
| No authentication blockers | Mike to do | The download sits behind a name and email form (`website/site/pages/download/index.html:16`, `:25`). Give reviewers a direct installer link (the GitHub release asset) in the reply. Use a build that has #779. |
| Clear path to the integration | Draft ready | Put this in the reply: "Install Whimbrel for Mac, open Settings, then Connectors. Click Connect next to Google Calendar or Gmail." |

### Privacy policy disclosures

The live page predates the narrowing and Limited Use. "Not done" means the live page lacks it; the text is in section 6 and the exact edits are in section 7. Deploying the website is Mike to do.

| Requirement | Status | Evidence |
|---|---|---|
| Data accessed | Not done (text drafted) | Live page lists `calendar.readonly` and `gmail.readonly`, `gmail.compose`, `gmail.send`, and says sign-in reads "name". All wrong for the new build. |
| Data use | Done | Same section covers schedule, meeting prep, recognising the meeting, waiting conversations, briefings. Minor edit: Gmail reading is only with the user's own project. |
| Data transfer | Not done (text drafted) | Live page does not say providers do not train on the data, and does not mention that Gmail and Calendar text is withheld from some providers, OpenRouter, TypeSafe and MCP clients. |
| Data protection | Not done (text drafted) | No paragraph says how Google data and tokens are protected. |
| Retention and deletion | Not done (text drafted) | Live page says "stays until you delete it in the app", which is not true for Gmail on the Mac, and does not say disconnect does not revoke at Google. |
| Limited Use statement | Not done (text drafted) | `privacy.html:77` says "adheres to"; Google's required wording is "will adhere to". |
| No ads, no selling, no AI training | Done | `privacy.html:75`. |
| No transfer to third-party AI that trains on the data | Done in code, not yet on the page | Section 4. The Limited Use paragraph in section 6 states it. |
| Allowed use case | Done | A productivity assistant over the user's own calendar. No bulk or cold email, no email warming. |
| Deploy the privacy page | Mike to do | After the release (so the page matches the build). |

### Security assessment

| Requirement | Status | Evidence |
|---|---|---|
| CASA | Not needed | No restricted scope is requested by the app's project (section 3). If Google's reply says otherwise, see section 3. |

### Submission steps

| Step | Status |
|---|---|
| Release with #779 and #786 | Mike to do |
| Console: branding, audience, Data Access, justifications | Mike to do |
| Record and upload new video | Mike to do |
| Update the request in the Verification Center | Mike to do |
| Reply to Google's 2026-11-02 email | Mike to do |
| Mac MCP `google_data` permission PR, Windows PR #793 | Not done (open) |

## 6. Draft text for the privacy page

These are drafts only. This PR does not edit the page. Each block says what it replaces. The text is true of `origin/main` at `0f073c86`; the scope bullets become true for users only once the release ships, so deploy the page after it.

**Replace the three bullets under "What the app accesses"** (`privacy.html:66-68`):

> **Sign-in** (`openid`, `email`): your Google account's email address and account identifier. They identify your account when you turn on sync and approve your own devices. On Android, Google's sign-in also hands the app your basic profile; Whimbrel does not use it.
>
> **Google Calendar** (`calendar.events`): the events on your primary calendar, including titles, times, attendees and conferencing links. Whimbrel uses them to show your schedule, prepare you for upcoming meetings, and recognise which meeting it is recording. It creates a calendar event only when you approve that action in the app.
>
> **Gmail** (`gmail.send`): Whimbrel sends an email, such as a recap or a request for a meeting recording, only after you approve that exact message in the app. With Whimbrel's own Google sign-in it cannot read your mail. If you want Whimbrel to read your Gmail, you can enter an OAuth client from your own Google Cloud project in Settings, Connectors, Advanced. Google then asks you to let your own app read your mail (`gmail.readonly`), and Whimbrel uses it to show the latest email on each client's page, find conversations waiting on you, and include them in the answers, briefings and recaps you ask for. Whimbrel never deletes, labels or changes your mail.

**Replace the AI provider bullet** (`privacy.html:74`) with:

> When you ask Whimbrel for a summary, answer or briefing, the text needed for that request is sent only to the AI provider you chose. Text from your Gmail or Google Calendar goes only to a provider that does not train on API data: Anthropic or OpenAI with your own key, Whimbrel AI (through Whimbrel's server to Anthropic, not stored), or a model on your own computer. It is never sent to OpenRouter or to TypeSafe. It goes to Google Gemini only if you confirm that your Google AI key is on a paid project, and to an Ollama server on another machine or an Ollama cloud model only if you turn that on in AI settings. Both settings are off by default. When a provider may not receive that text, Whimbrel leaves it out of the request. Known secrets such as API keys and passwords are redacted first. In Local-only mode, or with a local model, nothing leaves your computer.
>
> Apps that connect to Whimbrel through MCP receive no Gmail or Google Calendar text unless you give that one app the "google data" permission. It is off for every app by default.

**Add after "How it's used, stored and shared", under the heading "How Google data is protected":**

> Google sign-in uses OAuth with PKCE, so Whimbrel never sees your Google password. The access and refresh tokens are kept in the macOS Keychain or Windows Credential Manager, not in plain files. Gmail and Calendar data travel from Google to your computer over TLS and are stored in the app's local database in your user folder, which relies on your operating system's account and disk encryption. If you turn on sync, data derived from it is end-to-end encrypted on your device before it is uploaded. Requests to an AI provider go over TLS, and known secrets are removed first. If you use your own Google Cloud project, its client ID and secret are kept in the same credential store.

**Replace the sentence at `privacy.html:77` with this Limited Use paragraph:**

> Whimbrel's use and transfer of information received from Google APIs will adhere to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements. Whimbrel uses Google user data only to provide the features you see in the app. We do not use it, or anything made from it, to develop, improve or train generalized AI or machine-learning models. We do not sell it, use it for advertising, or let people read it, except where you share it with us for support or the law requires it. We transfer it only to the AI provider you chose, as described above, and only to providers that do not train on it.

**Replace the paragraph at `privacy.html:79`** (first two sentences and the third, which now says what disconnecting does):

> Disconnect a Google account in the app's Settings to stop Whimbrel using it. Disconnecting deletes that account's access tokens from your computer. It does not tell Google to revoke the grant, so also remove Whimbrel at [myaccount.google.com/permissions](https://myaccount.google.com/permissions) if you want Google to stop it. Data the app already stored stays on your computer after you disconnect. On Windows, Settings has a "Delete stored mail" button for each Gmail account. On the Mac, disconnect the account, then use "Delete everything on this Mac" in Settings, Your data, or remove the app's data folder. Delete everything keeps accounts that are still connected, and they fetch their mail again. Mail or events that you delete in Google are removed from Whimbrel the next time it syncs. You can delete your sync account and its encrypted data at any time from the app's Devices settings, or email us at the address below. If none of your devices connects to the sync service for six months, we email you, and delete the account and its encrypted data 14 days later unless one connects.

If the Mac gets a per-account "Delete stored mail" button, change the Mac sentence to match.

## 7. Live privacy page against the draft: the exact diff

Source: `website/site/pages/privacy.html` at `0f073c86`. Line numbers are that file's.

| Lines | Live text says | Change |
|---|---|---|
| 28-33 (AI providers row) | The provider receives the text "under its own terms". | Add: Gmail and Google Calendar text only goes to providers that do not train on it, and never to OpenRouter or TypeSafe. |
| 46-49 (Google row) | "Google sees the app reading your Gmail and Calendar." | "Google sees the app signing you in, reading your calendar, and creating events and sending emails you approve. It sees Gmail reading only if you use your own Google Cloud project." |
| 52-56 (TypeSafe row) | "TypeSafe receives email subjects and snippets to score which threads need a reply." | Add: "Never Gmail or Google Calendar text." Gmail threads and Google Calendar meetings are held back (`TYPESAFE_GOOGLE_DATA_CONFIRMED` is false), so TypeSafe sees only mail from other sources. |
| 66 (sign-in) | Lists `profile` and says the app accesses your "name". | Drop `profile` and "name" for the app's own sign-in, per the draft bullet. The server reads only the account ID and email. Android's `profile` is a library side effect. |
| 67 (Calendar) | Lists `calendar.readonly`, `calendar.events`; "creates or changes a calendar event". | Only `calendar.events`; "primary calendar"; the app creates events, it does not change them. Replace with the draft bullet. |
| 68 (Gmail) | Lists `gmail.readonly`, `gmail.compose`, `gmail.send`; says the app reads your mail with Google sign-in. | Only `gmail.send` with the app's sign-in; reading only through the user's own Google Cloud project. Replace with the draft bullet. |
| 72 | "no Whimbrel server receives or stores it" and Whimbrel AI passes an Ask through Whimbrel's server to Anthropic. | No change. |
| 73 (sync) | End-to-end encrypted. | No change. |
| 74 (AI provider) | "under its terms"; "Optional features ... such as TypeSafe reply ranking are off by default". | Replace with the draft paragraph (providers that do not train; withheld providers; MCP). |
| New, after 74 or 76 | Nothing on protection. | Add "How Google data is protected". |
| 75 | No sale, no ads, no AI training. | Keep. The Limited Use paragraph repeats it. |
| 77 | "adheres to" and "Limited Use requirements". | Replace with the draft paragraph. It uses Google's required wording, "will adhere to". |
| 79 | "Disconnect ... to stop all access"; stored data "stays ... until you delete it in the app or remove the app's data folder". | Replace with the draft paragraph: disconnect does not revoke at Google, and Gmail deletion is a Windows button and a Mac "Delete everything". |
| Home page, `index.html` | Names the connectors only. | Optional one line, section 5. |

## 8. Scope justifications for the consent screen form

Paste one paragraph per scope into Data Access. The non-sensitive scopes (`openid`, `email`, `profile`) need none. There is no restricted scope, so there is no Gmail reading paragraph. Do not add `gmail.readonly` or `gmail.compose` to the list.

**`https://www.googleapis.com/auth/calendar.events` (sensitive)**

> Whimbrel is a desktop meeting assistant for Mac and Windows. It reads the events on the user's primary Google Calendar to show today's meetings, prepare a briefing before each one, and recognise which meeting it is recording, so the notes are filed under the right meeting. It also creates an event when the user approves one in the app, for example a follow-up meeting or a focus-time block that a routine proposed. Nothing is created without that approval. Events are stored in the app's local database on the user's own computers. If the user turns on sync between their devices, the data is end-to-end encrypted on the device before upload, and Whimbrel's sync server cannot read it. Calendar text is sent only to the AI provider the user chose, and only to providers that do not train on API data; it is never used to train a model. A read-only scope is not enough because of the approved event creation, and calendar.events is the narrowest scope that both reads and creates events. Whimbrel does not change calendar settings, sharing or the calendar list, so it does not request the full calendar scope.

**`https://www.googleapis.com/auth/gmail.send` (sensitive)**

> Whimbrel sends an email only when the user approves that exact message in the app. Examples are an end-of-day recap the user asked to have sent to themselves, and a request to a meeting's organiser for the recording. The app shows the full recipient, subject and body in an approval card, and nothing is sent until the user clicks Approve. gmail.send is the narrowest scope that allows sending. With this scope Whimbrel cannot read, create drafts for, or change any existing mail, and it does not request any scope that can.

## 9. Demo video script (2 minutes 30 seconds)

Record on a build that includes #779 (Mac), so the consent screen shows the new scopes. Use the Mac app in English, at a readable size. In Safari, turn on Settings, Advanced, "Show full website address" first, or the `client_id=` part of the address is hidden. Use a test Google account that has a few events. Upload it to YouTube as unlisted. Use a voice-over or captions that name each step.

| Time | Shot | What must be on screen |
|---|---|---|
| 0:00 to 0:10 | Intro | The whimbrelapp.com home page, then the Whimbrel app window. Caption: "Whimbrel for Mac". |
| 0:10 to 0:35 | Calendar consent | Settings, Connectors, Google Calendar, Connect. The browser opens `accounts.google.com`. Zoom in on the address bar so `client_id=...apps.googleusercontent.com` is readable. Pick the account. On the consent page, click "Show all services" or expand each item so every scope is readable in full: the account identity items and "See, edit, share, and permanently delete" events (`calendar.events`). Hold on the expanded list for three seconds. Click Continue. The app shows the account as connected. |
| 0:35 to 0:50 | Calendar events in use | The schedule shows the day's meetings from Google Calendar. Open one meeting's preparation brief. Caption: "calendar.events: reading events". |
| 0:50 to 1:15 | Calendar write | A routine proposes a calendar event. The approval card shows the title and time. Click Approve. Switch to calendar.google.com and show the new event. Caption: "calendar.events: creating an approved event". |
| 1:15 to 1:40 | Gmail consent | Settings, Connectors, Gmail, Connect. Zoom in on `client_id=` again. Expand every item on the consent page so the identity items and "Send email on your behalf" (`gmail.send`) are readable, and so it is clear no read access is asked for. Click Continue. The account shows as connected. |
| 1:40 to 2:05 | Gmail send | An approval card shows a full email: recipient, subject, body. Click Approve. Switch to Gmail's Sent folder and show the message. Caption: "gmail.send: sending an approved email". |
| 2:05 to 2:30 | Data and disconnect | Settings, Your data, showing the Google rows. Disconnect both accounts in Settings. Open myaccount.google.com/permissions, show Whimbrel, and remove it to show that access is gone. |

Notes:

- Use the test account only. Do not show real mail.
- Do not show the Advanced "Use your own Google Cloud project" setting. If a reviewer asks about Gmail reading, the reply covers it (section 3).
- If the consent screen shows any read access for Gmail, the build or client is wrong. Stop and check.
- If a shot runs long, shorten the intro and the last shot, not the consent or write shots.
- If the app has reached the 100-user cap, Google's email says to record with a separate Cloud project. The client ID in the video will then differ from the submitted one; say so in the reply.

## 10. Mike's steps

In order. Steps 1 to 3 come first, so the live build, the page and the Console all say the same thing.

1. **Release** Mac and Windows builds that include #779 and #786 (v0.9.71 does not). Open the released build once and check that the Calendar and Gmail consent screens show only the scopes in section 1. Until this ships, public builds request the old scopes.
2. **Update the privacy page** with the section 6 text (edit list in section 7), and deploy the website. Optionally add the one-line explanation to the home page. Check the current API no-training terms for Anthropic and OpenAI (and for the Whimbrel AI path) the day you deploy, since the page promises them.
3. **Google Cloud Console, Google Auth Platform, Branding:** app name "Whimbrel", the 120x120 logo, support email `hello@whimbrelapp.com`, home page `https://whimbrelapp.com/`, privacy policy `https://whimbrelapp.com/privacy/`, terms `https://whimbrelapp.com/tos/`, authorized domain `whimbrelapp.com`, developer contact email. Save.
4. **Audience:** confirm the publishing status is still "In production" and the user type is External.
5. **Data Access:** remove `calendar.readonly`, `gmail.readonly`, `gmail.compose` and anything else not on this list: `openid`, `email`, `profile`, `calendar.events`, `gmail.send`. Paste the two section 8 paragraphs. Check the class the Console shows for each scope.
6. **Record and upload the new video** (section 9) as unlisted, and paste its link into the demo video field.
7. **Clients:** check that the Desktop, iOS, Web and Android clients are all in this project, and delete test clients that are no longer used.
8. **Verification Center:** submit the updated request.
9. **Reply to Google's 2026-11-02 "[Action Needed]" email** in the same thread. Go through its checklist item by item: the scopes narrowed to `calendar.events` and `gmail.send` and why, no restricted scope so no CASA, Gmail reading only through the user's own Google Cloud project, the AI rules in section 4, the new video link, the new privacy page sections, a direct installer link for reviewers, the path to the integration (Settings, then Connectors), and that no test account is needed.
10. **If Google still asks for CASA,** see section 3 and get two quotes.
11. **After approval:** check with a new Google account that the warning is gone, and update the Status section of this file.

Open code work, not Mike's to do but it affects the page and the reply: Windows PR #793 and the Mac PR for the MCP `google_data` permission; a Mac per-account "Delete stored mail"; revoking the grant at Google on disconnect.

## Open questions

1. **Mac Gmail deletion and revoke on disconnect.** Add a Mac "Delete stored mail" per account, and call Google's revoke endpoint when an account is disconnected? Both would make the retention text simpler. Until then the draft states the real behavior.
2. **TypeSafe.** Keep Google data held (the constant is `false`), or get written confirmation of TypeSafe's no-training terms and flip it? The page says never, so flipping needs a page edit.
3. **`calendar.events.owned` instead of `calendar.events`.** Narrower; its class is unprinted. Check in the Console before switching, and switching means a code change.
4. **Existing wide grants.** Leave older users on `gmail.readonly` and `calendar.readonly` until they reconnect, or prompt them to reconnect? The first is simpler. The second makes the live grants match the Console.

## Sources

- Google, restricted scope verification: https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification
- Google, sensitive scope verification: https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification
- Google, Gmail API scopes and classes: https://developers.google.com/workspace/gmail/api/auth/scopes
- Google, Calendar API scopes: https://developers.google.com/workspace/calendar/api/auth
- Google API Services User Data Policy: https://developers.google.com/terms/api-services-user-data-policy
- App Defense Alliance, CASA Tier 2: https://appdefensealliance.dev/casa/tier-2/tier2-overview
- App Defense Alliance, authorized assessors: https://appdefensealliance.dev/casa/casa-assessors
- Mimestream, CASA Tier 2 for a desktop Gmail client: https://mimestream.com/blog/casa-verified
- Switch Labs, CASA lab prices (third party, "as of 2024"): https://www.switchlabs.dev/post/casa-tier-2-tier-3-security-review-providers-pricing-and-the-cheapest-option
- Leviathan Security, CASA pricing: https://leviathansecurity.com/programs/google-casa-cloud-application-security-assessment
- Google's 2026-11-02 "[Action Needed] Your Google APIs Verification Request" email (not public).
