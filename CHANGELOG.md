# Changelog

Notable changes to Whimbrel. Releases ship to Mac and Windows at the same time.

## [Unreleased]

## [0.9.83] - 2026-11-25

### Mac and Windows
- Tell a friend: the menu bar or tray, the popup and Settings → Sync & Devices can copy a short message with a link to Whimbrel to paste wherever you like. Nothing is sent from Whimbrel, and no code or reward is involved yet.
- The skin maker no longer offers whole-page layouts. It offers the five Home element choices. Skins you already have load as before.

### Mac
- The live transcript no longer repeats one line's text in every row said by the same person within a minute.
- On the live meeting screen the transcript column grows with the window, slides appear under the notes as they are captured, and an alert's jump or a cited line in Ask highlights and scrolls to that line again.
- The live transcript follows the newest line, and stops following while you scroll up.
- "Still recording?" now asks after 1 hour instead of 2, for anyone who never changed it (a choice you made stays). If it gets no answer, the recording stops and is trimmed to the last words plus 30 seconds, the same trim "Still in a meeting?" uses. A short filler line like "Thank you." from near-silent audio no longer delays "Still in a meeting?" or cancels its prompt.

### Windows
- Narrator now reads a corner alert (for example "Meeting detected" or "Question for you") when it appears, and Tab reaches its buttons. With no screen reader running, alerts still open without taking focus. This was a known issue in 0.9.82.
- Skins can now set font weights, italic, and separate fonts for numbers and labels. Thread looks the same as before.
- When a skin's colors are hard to read against Home's parts, Windows adjusts them, as the Mac does, and says which colors it changed.
- Short filler lines such as "Thank you." no longer delay "Still in a meeting?"

### Website
- The home and download pages have a Tell a friend block that copies the same message as the apps, in English, Spanish and French.
- The privacy policy and terms describe referral codes, which are not offered yet.

## [0.9.82] - 2026-11-25

### Mac and Windows
- Whimbrel has a new look, called Thread, and everyone gets it after updating. Home is one time line of your day, and the menu bar and tray, alerts, hotkey window, Settings and the first-run screens are rebuilt in it. Topics, Automations, the Meetings list and the capture, episode and thread windows keep their layout and take Thread's colors, type and shapes.
- Thread is in the skin list. The cream look is still there as a skin named Classic (it was named Whimbrel). The first time you open this version, if you were on that cream look, you are moved to Thread once; pick Classic in Settings to go back and it stays. If you had chosen another skin, or the Slate or Sepia colors, nothing changes.
- The control for screen capture is now called Memory, with a timed pause (15 minutes, an hour or until tomorrow), in the menu bar or tray, the popup and Settings.
- The recording notice is on unless you turned it off. A setting you already changed stays as you set it.
- While a meeting records you can pause the recording and add marks to it. There is a small in-meeting window for them.
- New summaries ask the model for sources, so a line can point back to where in the meeting it came from. New recordings also store when each line ends.
- High Contrast follows your system. On the Mac, with Increase Contrast on, every screen draws in high contrast colors, whichever skin you picked. Turn the system setting off and your skin comes back; your choice is not changed. On Windows a contrast theme draws Whimbrel in that theme's colors, as it did before.
- Find opens a question on its best matches first, ignores common words, and keeps the rows already shown when you load more. An imported note is listed once, not also as its one-item episode.
- Ask keeps conversations: "Continue as a conversation" turns an answer into a conversation you can come back to.
- Settings is reorganized, with a Plan & billing section, a Plan row on the Status page, Local-only mode first in AI settings, and a search box that scrolls to the setting it finds.
- Tags and chips are readable on every skin.
- Most new strings are machine-translated (German, Spanish, French, Canadian French, Portuguese) and still need review.

### Mac
- Stop works while a detected or calendar meeting is still being saved.
- A question asked on Today with filter chips on goes to Find.

### Windows
- Right-clicking the tray icon while its popup is open closes the popup, so the menu opens.
- Narrator no longer repeats the recording time every second.
- A Stop that cancels a start before any audio is recorded leaves no meeting behind.
- A paused meeting reads Paused, not Recording, and a meeting still recording no longer shows a Recorded tag.
- Live meetings no longer lose words after Resume, sit on Resuming, leave their window open after Stop, or show the recording state twice.
- Known issue: Narrator does not read a corner alert (for example "Meeting detected" or "Question for you") when it appears, and the keyboard can't reach one until you click it. Once you click into it, Narrator reads it and Tab moves through its buttons.

### Phones
- Phone reminders are free on every plan. They run on your phone from what it last synced, so any signed-in phone gets a notification before each meeting and a morning list of the steps you owe that are due. Before, they needed Cloud. Settings → Reminders turns each kind on or off as before.

### Website
- Plus is sold at $15 a month, with updated terms and privacy pages in English, Spanish and French.
- The pricing page and the terms state a 14-day refund policy.
- The pricing and compare pages show Littlebird's monthly prices.

## [0.9.81] - 2026-11-20

### All devices
- Exclude from hosted models now treats a client's known contact addresses like the client's name. Before, an address in the text itself was checked only in the hotkey window. Now it is also checked when Whimbrel builds these requests for a hosted model: Ask, the material Ask finds, meeting summaries, titles, live questions and topic names. A known contact address is one met only in that client's material, never the address of an account you connected. It is matched as whole words, in any case, percent- or HTML-encoded, with hidden characters, or spoken ("jane at example dot com"). A question that holds the address is not sent to a hosted model: a model you run yourself answers it, or it is not answered.
  - Unlike a name, an address keeps out only the lines that hold it, not the whole meeting. In a recording that is not filed under the client, the other lines can still go to a hosted model.
  - An address spoken with extra words between its parts, or encoded another way (for example base64), is not matched.
  - If you have an excluded client, an Ask follow-up to an answer written by 0.9.80 or earlier is answered on your computer. It goes back to your usual provider after you ask that question again and Whimbrel records the new answer.
- Opening Whimbrel's library no longer fails with "database is locked" while another part of Whimbrel is writing; it waits up to 30 seconds.
- Most new strings are English only for now.

### Mac and Windows
- Whimbrel Plus: $15 a month or $144 a year, with a monthly Whimbrel AI allowance that is spent before any prepaid credit. The allowance does not roll over; a yearly plan gets it each month. Settings shows how much of this month's allowance is left and has the Plus checkout.
- A Plus account is asked once on each computer whether to use its Whimbrel AI allowance for AI features. The question shows the same data note as Settings. "Use Whimbrel AI" makes Whimbrel AI your provider (this syncs to your other devices), "Keep my current setup" changes nothing, and "Not now" asks again later. It isn't asked in Local-only mode, while a meeting records, or when Whimbrel AI is already your provider.
- The Meetings list no longer stops at the newest 200 meetings. Scrolling to the end (or Show older meetings) loads more, and the list says how many it is showing. On Windows, search looks through every meeting, not just the ones already loaded.

### Mac
- A second copy of Whimbrel (for example one in an iCloud Drive folder beside the one in Applications) no longer starts while one is running. It says "Whimbrel is already running." and Show Whimbrel brings the running copy forward. Two copies had recorded each calendar meeting twice.
- Sync now: Settings → Devices has a Sync now button, and so does the sidebar's "Sync hasn't worked" warning. The warning clears as soon as a sync works.
- A meeting with no summary (none yet, failed, refused, or kept local for an excluded client) shows Regenerate on the meeting page without opening edit mode.
- In a narrow window, Ask's provider picker becomes a menu instead of pushing the page under the sidebar.

### Phones
- The phone apps have a new look, shared with the Mac and Windows redesign. Your day is one thread: the next meeting, a Now marker, then briefs, routine runs and what already happened, each at its time.
- A meeting note shows who wrote each line. Whimbrel's lines are in their own color with a rule beside them, and "Written by Whimbrel" is said once at the top.
- Every screen was rebuilt: Today, Meetings, a meeting's summary and transcript, Ask, Topics, Routines, Settings, sign-in and the prep brief. Each was checked at the largest text size and on a small phone.
- The phones still read what your computers record. They don't record meetings.

### iPhone
- The device approval and device limit screens are easier to read, and each one says what went wrong when a step fails.
- At the largest text size, the Ask question box, the Settings pickers and a topic's step titles no longer cut off or overlap.
- "Written by Whimbrel" is machine-translated (Spanish, French, Canadian French, German, Portuguese) and still needs review.

### Android
- Ask is now on Android. Ask a question about your meetings and get an answer with its sources listed under it. A source that isn't on this phone says so. Each answer shows the model that wrote it and whose key was used. Dictation and read aloud aren't on Android yet.
- Topics is now on Android: one page per customer or project with your role, the next meeting, what you owe, what you're owed and recent items.
- The app has the same five tabs as the iPhone: Today, Ask, Meetings, Topics, Routines.
- Signing out, or having this phone removed from your account, clears every question and answer from Ask.
- A time on Today no longer breaks in the middle ("10:00 P" over "M"). At large text sizes the time sits above its card.
- Tab names stay on one line at large text sizes.
- The app no longer asks you to sign in to Google again every few minutes. The fix is on the sync service, so it did not need an app update.
- Settings shows when Whimbrel last checked for a new version and what it found.
- New Android text is in English only.

### Website
- The client controls page says that a question holding a known contact's address is not sent to a hosted model: a model you run yourself answers it, or it is not answered, and its FAQ states the limits of address matching.
- A new page explains encrypting the memory database, and the client controls page is linked from the homepage. The Spanish and French homepage and Windows download page are up to date.

## [0.9.80] - 2026-11-19

### All devices
- Client controls. A client's page (Mac, Windows, and phones through sync) has four new controls:
  - **Delete this client** shows what will be removed (linked items, possible matches left unticked, and what is left for you to check), asks you to type the client's name, then gives a receipt. The delete reaches your other devices; a phone applies deletes but can't start one. Settings → Sync shows how many devices still need to update, with "Remove this device".
  - **Keep for N months** deletes the client's material after that time. Each device asks before deleting its own copies.
  - **Export** saves the client's material to a folder you pick, and says what it leaves out.
  - **Exclude from hosted models** keeps material that names the client out of anything sent to an AI service off your device. A request that involves the client runs on a local model or is refused. Matching uses the client's name, aliases and email addresses, not a model, and is whole-word.
  Copies Whimbrel made from the client's material (routine results, profile suggestions, meeting briefs) can stay on other devices; the receipt says so. New strings are machine-translated and still need review.
- Building and sorting topics no longer fails with "database is locked" when another part of Whimbrel writes at the same moment; it now waits its turn.
- Delete this client's review is easier to read: each possible match shows the line that names the client, repeated screen captures are grouped by app and day, linked items say what they are, and the name field stays in view. If background work is running, the delete waits for it (with Cancel) instead of failing.
- A device that updates later now gets every client's settings, including Exclude from hosted models, set while it was on an older build.
- Sync no longer fails every round when the sync service answers slowly. Updates that cannot go out in one round go in the next, and the round still counts as working.

### Mac and Windows
- The memory database can now be encrypted on your computer. A copy of the database file (one in a backup, a synced folder, or another account on the same computer) can't be read without its key, which Whimbrel keeps in the Keychain on the Mac and in Credential Manager on Windows. New installs are encrypted from the first launch. An existing install stays as it is until you choose Settings → Capture & Privacy → "Encrypt the database", which encrypts at the next launch (on the Mac it offers Export everything first).
  - Encrypting asks you to create a recovery key: a recovery file with a passphrase, plus 24 words. Keep them somewhere other than this computer. If the key is lost (a new computer or a reset Keychain), a recovery screen opens the database with either one. Whimbrel never deletes or replaces a database it can't open.
  - Only the memory database is encrypted. Audio and other files are not. Phones are unchanged.
  - "Faster search (uses more memory)" gives search up to 1 GB more memory on a computer with at least 16 GB. "Check encryption speed" times search on your own data and keeps the report on your computer.
  - Delete everything also removes the database keys. API keys and sign-ins are kept.
- Background work (sync, email and topic indexing, retention) runs on its own database connection, so screens and Ask no longer wait behind it.
- Ask shows the model that actually answered, and never waits behind background model work.
- Excluding a client no longer makes Whimbrel rescan its whole library every few seconds; new items are checked as they arrive.

### Mac
- A meeting summary a model refused now says why, instead of only "No summary yet".
- A scheduled Teams meeting is recorded from the right window.
- Hiding or showing the sidebar no longer crashes the app while the "Sync hasn't worked" warning shows.
- Screen text capture can no longer freeze Whimbrel when the app in front stops responding.

### Windows
- Releases now include a native Windows on Arm installer (`Whimbrel_x.y.z_arm64-setup.exe`) beside the x64 one, and the update manifest lists it, so an Arm install updates to Arm builds. It runs without emulation; speaker separation isn't in the Arm build yet. The download page offers it ("Download for Windows on Arm") once a release carries it.
- The installers no longer include Whimbrel's developer tools, so the download and the install are smaller.

## [0.9.72] - 2026-11-14

### All devices
- Meeting summaries and Ask about a meeting use the whole transcript. The amount sent now follows the model's context window, so a cloud model reads a long meeting in one call, and a local model reads it part by part instead of dropping parts. Action items now list every commitment and request with an owner and due date when the meeting states them, and each part of a long summary knows how the one before it ended.
- Gmail and Google Calendar text now goes only to a model on your device or to an AI service whose terms say it doesn't train on your data. It's left out of what Gemini (a Google AI Studio key) and a remote or cloud Ollama model receive unless you turn that on, it never goes to OpenRouter, and TypeSafe gets none of it for now. Every OpenRouter request now asks for providers that don't store or train on data.
- The same rule now covers text Whimbrel writes from Gmail and Calendar. Routine output is held back from AI that may not receive Gmail or Calendar text. So are earlier Ask answers (in later turns and in conversation titles) and past meetings that came from Google Calendar in meeting prep. The calendar title check for TypeSafe no longer lets a meeting through after its event is renamed or removed. Whimbrel's MCP server leaves Gmail and Calendar items out of `assemble_context` unless the client's token has the `google_data` scope. Meeting Ask, the profile suggestions and speaker naming no longer send a calendar meeting's title or invitee names to an AI service that may not receive Google data, and the MCP meeting and search tools leave a calendar meeting's title, attendees and summary out unless the client has `google_data`. A calendar meeting that may be from Google Calendar is now summarized only by an AI service allowed to receive Google data (otherwise the summary fails and says so), and its summary stays out of prompts for the others. Topics built from such a meeting stay off those services too. Older answers and meetings whose source was never recorded count as Google data, so they are left out for those AI services.

### Mac
- A meeting page can send its recap to the attendees: Send recap opens a draft with the invitees ticked, an editable subject and message, and a "Sent with Whimbrel" footer. With Gmail connected it sends from your account; otherwise it opens in your mail app. The page then shows who it went to. Copy now also copies the summary as Markdown or for Slack. A recap never includes your notes or the transcript.
- Background work (embeddings, summaries, topics, profile suggestions, routines) now waits while a meeting is recording, so it no longer slows a call; Meeting Prep still runs.
- A summary meant for Whimbrel AI that a model on your Mac wrote instead (you were signed out, or Whimbrel didn't answer) now says why under the summary, with a "Re-summarize with Whimbrel" button, or "Sign in to Whimbrel" when signing in is the fix.
- Storage fixes: Free up space now says when compacting or deleting old screen captures fails instead of reporting space freed, "Not enough free disk" asks only for the space still missing, and an external exFAT or network drive no longer triggers a false low-disk warning.
- Google connectors ask for fewer permissions: Calendar asks only to manage events, and Gmail asks only to send. To keep reading Gmail, add your own Google Cloud project's OAuth client under Settings → Connectors → Advanced. Accounts connected before this keep reading until you reconnect them. Gmail and Calendar text now goes only to AI that doesn't train on it; Settings → AI has switches to allow Gemini on a billed project and a cloud or remote Ollama.
- Today, At a Glance and Upcoming now also show calendar events synced from your other computers (for example a work calendar that is only on another Mac), without duplicates, and Meeting Prep covers them. They never start a recording prompt on this Mac. Other-computer events reach the lists up to two days ahead. Core: new `whimbrel_calendar_synced_meetings` for the Windows lists to follow.
- Settings → MCP Clients has a "Gmail and Google Calendar" permission, off by default. Without it an assistant gets no Gmail or Google Calendar items, and no titles, people or summaries of meetings from Google Calendar. It needs Memory, Meetings or Transcripts as well, and clients made earlier don't have it. Meetings recorded from a Google Calendar event are marked as coming from Google Calendar.

- Removing a Google account in Settings → Connectors now asks first, then deletes that account's Gmail data or Calendar events from this Mac and revokes Whimbrel's access with Google (best effort: if you're offline the account is still removed here). Meetings you recorded are kept.

### Windows
- When a summary meant for Whimbrel AI was written by another model, a line under the summary says which model and why. It offers "Re-summarize with Whimbrel", or "Sign in to Whimbrel" when this PC is signed out. A failed re-summary keeps the summary and says why.
- Disconnecting a Google account (Gmail or Calendar) now asks first. It then deletes that account's stored mail or calendar events from this PC and revokes Whimbrel's access with Google. Whimbrel keeps the access while another of its connections on this PC uses the same address. Recorded meetings are kept.
- A meeting's summary has Copy (plain text, or Markdown and Slack from its menu) and Send recap. Send recap lists the invitees with an email address, all ticked, minus your own. You can add others and edit the subject and text. With Gmail connected it sends when you click Send; otherwise it opens a draft in your mail app. The email ends with "Sent with Whimbrel". The meeting then says who it went to: "Sent the recap to 3 people". New strings are English only.
- The pause on background work while a meeting records now uses core's shared recording signal, so core's own background work waits too. If Whimbrel quits mid-meeting, the pause lifts on its own within 10 minutes.
- Finding steps in topics and the TypeSafe topic check now also wait while a meeting records, like topic sorting already did.
- Restart now waits while a meeting is being recorded or processed, and says why. A restart at the end of a meeting had cut the recording short and thrown away the transcription that had just started.
- On a PC with an Arm processor, Whimbrel now says that Windows runs it through emulation, so transcription and summaries are slower and calls may stutter while a meeting records, and suggests Whimbrel AI for summaries. The diagnostics file shows the processor and the build.
- A summary or answer that fails because Ollama isn't running now says so, and suggests Whimbrel AI. When Ollama wasn't installed or running, a slow connection attempt was read as Ollama being too slow, and the message said the local model "didn't finish".
- Settings → About and the tray menu have "Join the Whimbrel Discord…", which opens the Whimbrel Discord invite in your browser.
- Today, Upcoming, the next-meeting card and Meeting Prep now also show calendar events synced from your other computers (for example a work calendar that is only on your Mac), without duplicates. They never prompt or record on their own; Start on one records it as a manual meeting.
- Settings → About has "Copy performance report": the last hour of Whimbrel's CPU and memory use (the app, its windows and the local model separately, every minute during a meeting), live transcription speed, snapshot and background job timings, and the PC's graphics. Numbers only, with no meeting titles, text or names; the diagnostics file from Send feedback includes it too.
- While a meeting records, background work (summaries, indexing, topics, profile suggestions and routines) waits until it ends; Meeting Prep still runs. If the PC stays short of memory (85% in use) or CPU (80%) during the meeting, the live transcript pauses and meeting snapshots slow to one every 30 seconds; the meeting is still recorded in full and transcribed afterwards.
- Gmail on the app's own Google client now asks only to send, and Calendar only for events, so Whimbrel no longer needs Google's restricted Gmail permissions. Reading mail needs your own Google Cloud project: Settings → Connectors → Advanced → "Use your own Google Cloud project". Accounts already connected keep reading until they sign in again; a send-only account says so on its row and is never polled.
- Settings → AI has two switches, both off: "My Google AI Studio project has billing on" (Gmail and Calendar text goes to Gemini only then) and "Allow Gmail and Calendar text with a cloud or remote Ollama model". The TypeSafe descriptions now say Gmail and Google Calendar items wait until TypeSafe confirms it doesn't train on them.
- Free up space (Settings → Storage) can compress recordings: "Compress all" turns each transcribed meeting's recordings into 32 kbps AAC (`.m4a`, the Mac's format), about a tenth of the size, using Windows's own encoder. Transcribe again and speaker labels still work on a compressed meeting. A meeting being recorded or transcribed is skipped, and a recording that doesn't compress cleanly is left as it was.
- Fixed: "Delete old recordings" in Free up space never found a meeting recorded on Windows.

## [0.9.71] - 2026-11-12

### All devices
- A computer that falls far behind another device's sync now catches up quickly: it saves the backlog first and builds search for those items in the background, instead of falling further behind (a Windows PC was two weeks behind a busy Mac).

### Mac
- Settings → Storage now shows how much space Whimbrel uses on this Mac, split into the database, recordings and images, speech models, the capture queue, and other files.
- Free up space (Settings → Storage) shows what each cleanup would free and runs it once: compact the database, compress all recordings, delete recordings older than 30, 90 or 180 days (transcripts, notes and summaries stay), or delete old screen captures. Your retention settings don't change.
- The database is compacted after retention runs when that gives back real space, so deleted captures free disk; "Compact Database" does it on request.
- The capture queue (screen-text.jsonl) is emptied once its captures are saved, instead of keeping every screen's text forever.
- When the disk holding Whimbrel's data or recordings has under 10 GB free, Storage says so and a notification (at most once a day) opens Free up space.

### Windows
- Settings → Capture has a Storage card: how much space Whimbrel uses on this PC, split into the database, recordings, speech models and other files.
- Free up space (Storage) shows what each cleanup would free and runs it once: compact the database, delete recordings older than 30, 90 or 180 days (transcripts, notes and summaries stay), or delete old screen captures here and on your other devices. Your retention settings don't change, and a meeting being recorded is never touched.
- The database is compacted after retention runs when that gives back real space; "Compact database" does it on request.
- When the drive holding Whimbrel's data has under 10 GB free, Storage says so and a notice (at most once a day) offers Free up space. Recording carries on.
- While another device has more to sync, the next round starts 10 seconds after the last, not a minute; each round's sync numbers go in the log.
- On a meeting recorded on another device, the action-item boxes look disabled and say why on hover, instead of looking clickable.
- Hide chat folds a skin's chat away (Show chat brings it back), and a long chat scrolls inside its own box instead of filling the page.
- Found images in the skin chat stay on screen to approve (they vanished as soon as they loaded), and an approved image shows on Today.
- Without an image search key the skin chat no longer promises image suggestions; it says an image search key in Settings → AI turns search on, or that you can add your own image.

### Android
- Whimbrel checks for a new version each time you come back to the app, and in the background every 6 hours. A new version also shows as a notification (once per version, if notifications are allowed); tap it to open Whimbrel and download it.

## [0.9.70] - 2026-11-12

### All devices
- With a Google key, a model Google won't serve to that key (such as the old default gemini-2.5-flash, refused for newer keys) is replaced with Google's current Flash model instead of failing every request.
- Long meetings summarize with a local Ollama model on slower computers. Whimbrel now waits for as long as the model keeps writing, instead of giving up after 10 minutes.
- When Google refuses to share a calendar, Whimbrel now records why: either the calendar wasn't allowed when you signed in, or Google or your organization blocks it. Gmail errors carry the same detail.
- Default AI models can no longer go stale. Each day Whimbrel reads the current default for Google, Anthropic and OpenAI from the updates server (today gemini-flash-latest, claude-haiku-5-5 and gpt-5.4-mini), and keeps its own list for when it can't. A model you never picked, including an old default, follows the current one.
- When Anthropic, OpenAI or Google says it no longer offers the model you chose, Whimbrel answers that request with the provider's current default instead of failing, keeps your choice, and never moves you to a pricier model family. Usage and budgets count the model that actually answered.

### Mac
- New: Ask about this window. Press Control-Option-Space (change it in Settings → Capture), or choose "Ask about this window…" in the menu bar, to ask a question about the window you're in. The panel says where the question will go before you send it, removes known secrets first, never reads password fields or excluded apps, and doesn't keep the window's text. This first version answers and copies; it doesn't draft or paste.
- With a Google key, Whimbrel now defaults to Google's current Flash model. The old default (gemini-2.5-flash) was retired by Google and returned errors.
- Settings → Connectors and the Schedule panel now say when Google didn't give Whimbrel access to your calendar (sign in again and allow calendar access), or when Google is blocking calendar access for the account.
- Settings → AI Providers says when a provider no longer offers the model you chose, and which model is answering for now. The Anthropic and OpenAI defaults now come from Whimbrel's shared list too (claude-haiku-5-5 and gpt-5.4-mini today).

### Phones
- A new phone shows your recent meetings first and fills in older history afterward. Until it's done, the sync line says "Older history is still loading."
- iPhone: Ask uses the current default model for your provider unless you picked one on your computer, and logs usage under the model that actually answered.

### Android
- With more than one Google account on the phone, Whimbrel renews its sign-in for the account you chose without asking you to pick an account again. A phone signed in before this update asks once more, then remembers.

### Windows
- New: Ask about this window. Press Ctrl+Alt+Space (change it in Settings → Capture), or use the tray menu, to ask a question about the window you're in. The panel says where the question will go before you send it, removes known secrets first, skips password fields and excluded apps, and doesn't keep the window's text.
- When Google keeps Whimbrel from a calendar, Settings → Connected accounts says so on that account: either calendar access wasn't allowed when signing in, with how to fix it, or Google or your organization is blocking it.
- With a Google key, Whimbrel now defaults to Google's current Flash model (gemini-flash-latest).
- Restart now no longer fails with "Failed to kill Whimbrel" when Whimbrel takes a moment to close: the update waits up to 15 seconds for it.
- When a meeting summary fails because the local model (Ollama) couldn't finish, the meeting says so and suggests Whimbrel AI or a smaller model, instead of "every model provider failed".
- Settings → AI says when a provider no longer offers the model you chose, and which model is answering for now. The Anthropic and OpenAI defaults now come from Whimbrel's shared list too.
- A stalled update download no longer stops Whimbrel from checking for updates. Each check now gives up after 30 minutes, and Send feedback's report says when Whimbrel last checked and what it found.

## [0.9.69] - 2026-11-11

### All devices
- Suggested topics get real names from your AI model (one short call each, within your spend limits), instead of a list of people's emails. Until then they show a friendlier placeholder.

### Mac
- The "Start Recording" button on the meeting alert is easier to read in light mode.
- Episode and thread summaries no longer run on the local model when "Use local model" is off.
- Today's schedule no longer shows meetings you declined or all-day events, and the day now ends at the right midnight on daylight saving change days.
- If sync stops after a sign-in problem, the Mac now retries on its own every 30 minutes and on wake, instead of waiting for a relaunch, and Settings and the sidebar say when sync stopped.
- Slides show live in the meeting view while you record. The slides toggle is now "Keep slides at full size", and says what it does.

### Windows
- Suggested topics get names written by your AI model, as on the Mac, instead of a list of the people in them. Settings → Topics says when some still wait for one.
- If sync stops after a sign-in problem, Windows now retries on its own every 30 minutes and when the PC wakes, instead of waiting for a relaunch. A brief Google outage no longer stops sync at all. Settings → Status and the sidebar say when sync stopped and how many changes are waiting.
- While you record, the slides strip sits under the notices and above the live transcript. Its toggle is now "Keep slides", with a line saying what it keeps.

## [0.9.68] - 2026-11-11

### All devices
- Phone reminders (Cloud, iPhone and Android): a notification before each meeting, and a morning list of the steps you owe that are due. Settings → Reminders sets how early, and when the morning list arrives.
- Cancelled Google Calendar meetings now disappear from Whimbrel on Mac and Windows.
- Meetings that are cancelled, declined, deleted or moved after they reach your phone now disappear there too, and a moved meeting shows only at its new time. Disconnecting a Google account hides its meetings; reconnecting brings them back. A meeting that comes back shows again.

### Mac
- Today shows your day as a timeline with a Now marker, and the Meetings tab is a compact list. Meetings are clear cards and rows that open with a click, with chips for prep, summary, action items and topic. An upcoming meeting opens its prep brief or topic. The next meeting says how soon it starts.
- Calendar sync covers the next 48 hours instead of the rest of today, so tomorrow's early meetings reach your phone before this Mac wakes. Meetings you declined are left out.

### Android
- The update prompt's Download now gets the newest app file directly, instead of a release page that may not have one.

### Windows
- A meeting's action items show their full text, wrapped onto more lines, instead of a short box you had to scroll inside.
- When a Google Calendar or Gmail sign-in expires, Whimbrel says which account on every page and in Settings, with a Sign in again button that opens Google at that account. A brief outage at Google no longer reads as an expired sign-in.
- Background summaries go easier on laptops. With "Use local model (Ollama)" off, they no longer run on Ollama just because it's running. Activity summaries now wait until the PC is plugged in and you've stepped away, as on the Mac.
- The app log records this PC's memory, cores and power, and every 5 minutes how much CPU and memory Whimbrel and Ollama use, with no content and no other programs' names. Send feedback's report includes the same facts, so a slow PC can be diagnosed from it.
- Meeting Prep's "Prep ready" and "Prep failed" lines in Routines show in your language.
- Calendar sync covers the next 48 hours instead of the rest of today, so tomorrow's early meetings reach your other devices before this PC wakes.
- Meetings you declined are no longer synced to your other devices, so phones won't remind you about them.
- Today's schedule is a timeline: one card per meeting, earlier ones included, on a line with a "Now" marker that moves through the day. Each card shows the time range, how many people, and chips for Prep ready, Summary, action items and the topic; click a card or chip to open it. The Meetings list groups meetings under Today, Yesterday and the day, with the time and length beside each one and the same chips. The schedule also refreshes after each calendar sync.

## [0.9.67] - 2026-11-10

### Mac
- Whimbrel AI offers Claude Opus 5.5, Sonnet 5.5, Haiku 5.5 and Fable 5.2, and its model list now comes from Whimbrel's server, so it stays current. Settings → AI Providers has Refresh Models on the Whimbrel AI card. With no model chosen, Whimbrel AI uses Sonnet 5.5.
- Meeting prep briefs show up wherever you look. A brief prepared on your other Mac appears on Today after a sync and posts "Prep ready". Today and Upcoming rows with a brief have a "Prep ready" chip that opens it. The menu bar's next meeting and the meeting-start pop-up have "Open brief". The pop-up's buttons now sit on their own row, and it shows the Whimbrel icon.
- When Meeting Prep can't write a brief (no model available, or an answer it can't read), the run shows as failed in Routines with the reason, once per meeting.

### Windows
- Whimbrel AI offers Claude Opus 5.5, Sonnet 5.5, Haiku 5.5 and Fable 5.2, and its model list now comes from Whimbrel's server, so it stays current. Settings → AI has Refresh Models.
- When Meeting Prep can't write a brief, the run shows as failed in Routines with the reason, once per meeting.

## [0.9.66] - 2026-11-10

### Mac
- Settings → AI Providers has Whimbrel AI: buy credit and use Claude models without your own key. You pay what you use plus 30%, and it stops when the credit runs out.

### Windows
- Settings → AI has Whimbrel AI: buy credit and use Claude models without your own key. You pay what you use plus 30%, and it stops when the credit runs out.

## [0.9.65] - 2026-11-09

### Mac
- Settings → Sync & Devices has a card for the phone apps' beta, with a QR code to scan with your phone. If Upgrade or Manage subscription can't reach Stripe, the message now shows right under those buttons.
- Settings → Your data can keep an Obsidian vault up to date: each topic's page, its people, organizations and meetings, with your own notes kept. This Mac also publishes its topic pages for your phone's Topics tab.

### Windows
- Settings → Sync & Devices has a card for the phone apps' beta, with a QR code to scan with your phone. If Upgrade or Manage subscription can't reach Stripe, the message now shows right under those buttons.
- Settings → Capture & Privacy can keep an Obsidian vault up to date: each topic's page, its people, organizations and meetings, with your own notes kept. This PC also publishes its topic pages for your phone's Topics tab.

### iPhone
- A Topics tab shows each topic's page, built by Whimbrel on your computer.

### All devices
- After you cancel Cloud, Sync & Devices says "Cloud ends <date>. It won't renew." instead of "Cloud until <date>".

## [0.9.64] - 2026-11-09

### Mac
- Settings → Sync & Devices shows your plan. On the free plan (one computer and one phone) you can upgrade to Cloud for unlimited devices, $5 a month or $50 a year plus tax, in Stripe's checkout in your browser; on Cloud, Manage subscription opens Stripe. A Mac stopped at the device limit can upgrade from there and joins once the payment goes through. Translated into German, Spanish, French and Portuguese.

### Windows
- Settings → Sync & Devices shows your plan. On the free plan (one computer and one phone) you can upgrade to Cloud for unlimited devices, $5 a month or $50 a year plus tax, in Stripe's checkout in your browser; on Cloud, Manage subscription opens Stripe. A PC stopped at the device limit can upgrade from there and joins once the payment goes through.

## [0.9.63] - 2026-11-09

### Mac
- Fixed: on macOS 27, Today kept saying "No calendar connected" and Upcoming filled in only minutes after launch. Whimbrel now trusts the calendar access you granted, refreshes the moment it has it, and shows each Google account's meetings as soon as that account syncs. If Upcoming stays empty, grant Calendar once in Settings → General.
- Fixed: with no named topics, the Topics tab shows a centred "No topics yet" with how many suggested topics are waiting and a button to Settings → Topics.

### Windows
- Fixed: Today's Upcoming filled in only minutes after launch; it now updates as soon as the calendar syncs, and says when the calendar can't sync (an expired Google sign-in, say) instead of looking empty. In skinned layouts it sits after the schedule, as on the Mac.
- Fixed: with no named topics, the Topics tab shows a centred "No topics yet" with how many suggested topics are waiting and a button to Settings.

### All devices
- The Topics list opens faster: it reads your upcoming meetings once instead of once per topic.

## [0.9.62] - 2026-11-09

### Mac
- Whimbrel now notices meetings in Chrome, Edge, Arc and Brave, not only Safari. It needs the Accessibility permission.
- Excluded sites are now also skipped in Chrome, Edge, Arc and Brave. Before, their pages could still be read from the screen.
- Settings → Topics: name the customers, products, projects and subjects you care about, with other names they go by, and Whimbrel files your email, meetings, calendar and screen activity under them. It also suggests topics it finds.
- Optional: with a TypeSafe key, Whimbrel can ask TypeSafe to sort items that don't name a topic outright (Settings → AI).
- Screen activity summaries wait until the Mac is plugged in and idle, like email summaries, as on Windows. Activity is still recorded as it happens.
- Topics: a page per topic with the next meeting, what you owe, what's owed to you and recent activity. Open it from Today, a meeting, its prep brief, a search that names the topic, or the menu bar.
- Steps are found by your local model; with a TypeSafe key you can switch that to TypeSafe (Settings → Topics).
- Today has an Upcoming section under the schedule: your meetings for the next 7 days, by day, with how many people are invited and each meeting's topic. It shows the first 2 days with meetings; "Show the rest of the week" opens the rest, and Whimbrel remembers your choice on this Mac.

### Windows
- Meeting snapshots skip a slide that is the same as the last one saved, and narrow strips of the call window when nothing is shared, as on the Mac.
- Settings → AI has a "Sync to devices" switch under each API key, as on the Mac. It's off until you turn it on, and a key moves only between devices that have it on.
- While a meeting records, Windows transcribes it in pieces cut where someone pauses, as the Mac does, instead of every two minutes. Fewer words are cut off, silence is no longer sent to the transcriber, and the transcript is ready sooner after the meeting ends.
- The meeting page shows a live transcript while you record, as on the Mac. It is provisional, about a minute behind, and replaced by the full transcript after the meeting. The live text stays on your PC. Settings → Meetings → "Live transcript during meetings" turns it off and deletes it.
- Settings → Topics, as on the Mac: name the customers, products, projects and subjects you care about, with other names they go by, and Whimbrel files your email, meetings, calendar and screen activity under them. It also suggests topics it finds.
- Optional: with a TypeSafe key, Whimbrel can ask TypeSafe to sort items that don't name a topic outright (Settings → AI). The key stays on this PC.
- The meeting page has an Ask bar, as on the Mac. Ask about the meeting while it records, from the transcript so far, or after it from the full transcript. Answers cite transcript lines, and three quick questions are built in: "Make me sound smart", "Bring me up to speed" and "Did anyone ask me a question?". Answers aren't saved.
- If Whimbrel keeps closing right after it starts, it now stops retrying after six tries, saves a diagnostics report (email addresses and meeting titles removed) and offers to email it, since Send feedback can't be reached.
- Topics, as on the Mac: a page per topic with the next meeting, what you owe, what's owed to you and recent activity. Open it from the Topics tab, Today, a meeting, its prep brief, a search or question that names the topic, or the tray.
- Steps are found by your local model; with a TypeSafe key you can switch that to TypeSafe (Settings → Topics), where you can also list the names you go by.
- Today looks ahead: an Upcoming section lists the next 7 days' meetings by day, with each meeting's topic.
- Fixed: Settings → AI no longer shows the "Pick where AI runs" panel for a moment and then jumps to the model picker on a PC that already has a model.

### All devices
- Topics you name, and how items were sorted, reach your other devices.
- Steps, their state, your role on a topic and the names you go by reach your other devices.
- Ask reads your email: a question about something said in an email finds that message and cites its thread, even before the thread has a summary.
- Ask finds what you asked about in plain questions. Keyword search now matches any of your words, best matches first, so a question asked in ordinary words finds the meeting line or capture it is about. Search in the Library shows more results, best matches first.

## [0.9.61] - 2026-11-08

### Windows
- Fixed: on Windows on Arm laptops, Whimbrel closed a few seconds after every launch. Microsoft's Vulkan compatibility layer crashed it when transcription first used the GPU; Whimbrel now skips that layer. If the transcription speed check ever closes the app again, it isn't run again.

## [0.9.60] - 2026-11-07

### Mac
- Away Mode: turn it on in Settings → Meetings and Whimbrel joins the Zoom meetings you can't make, muted with the camera off, records them and posts a short note in the meeting chat.
- The Settings page in the main window is now a Status page: sync, AI, storage, updates and routines at a glance, each opening where it's changed.
- Settings → Devices links to the iPhone and Android beta sign-up.

### Windows
- Settings opens on a Status page: sync, AI, storage, updates and routines at a glance, each opening where it's changed.
- Settings → Sync & Devices links to the iPhone and Android beta sign-up.

### Android
- "Update available" appears when a newer release is out, with a link to the release page.

### All devices
- The Whimbrel phone app for iPhone and Android is in beta. Request to join at whimbrelapp.com/mobile-beta.

## [0.9.59] - 2026-11-06

### Android
- Whimbrel for Android, early access: read your meetings, transcripts, summaries and routines on your phone. Install it from the release page in your phone's browser; later releases update it in place.
- The phone keeps its sync sign-in across restarts and doesn't stop syncing to ask for your Google account.
- The Whimbrel logo is the app icon.

### Mac
- Settings → Devices shows "Changes waiting to send" when this Mac's changes have been waiting more than an hour to reach your other devices.
- A meeting window that's open when another device forgets its meeting now says the meeting is no longer available.
- Fewer near-duplicate screen captures: a strip of the call window is no longer saved when nobody is sharing a screen.

### Windows
- Settings shows one section at a time (General, Profile, Capture & Privacy, Meetings, AI, Sync & Devices, Connectors), as on the Mac.
- Clicks such as Dismiss respond at once instead of waiting behind meeting summaries.
- Error messages and the Ask button point to Settings → AI, the section's new name.
- The "Routines run on" choice is ready when you open Automations, and the Routines card no longer says "this PC" when another device runs them.

### All devices
- Dismissing or adding a suggested profile fact resolves it on every device.
- Forgetting a meeting reaches your other devices sooner, and a forget, routine run or other change sent late is no longer missed by a device that already synced past it.
- The iPhone catches up faster on its first sync.
## [0.9.58] - 2026-11-05

### Mac
- "Forget meeting" now removes the meeting from all your devices and from the sync server, including its transcript, notes, summary, speaker names and live alerts. The same goes for deleting a meeting from the Library or the Meetings list, and for "Delete meetings without audio" in Settings → Retention. Retention by age still removes meetings only on the device where it runs.
- A meeting forgotten on another device disappears from this Mac at the next sync, and its recordings are deleted.

### Windows
- "Forget meeting" removes the meeting from all your devices, as on the Mac. A meeting forgotten on another device disappears at the next sync, and its recordings are deleted.

## [0.9.57] - 2026-11-05

### Mac
- Speaker names: Whimbrel names meeting speakers from the calendar invitees, remembers voices, and lets you fix a name from a menu. Names sync across your devices.
- Monthly AI budgets per provider, counted across all your devices. You get a notice at 80% and 100%; at 100% Whimbrel stops using that provider and falls back to others, Ask included. You can raise a budget any time.
- Trim a recording: cut a meeting at a time you pick, for example at 1:05 when you know it ended there.
- "Still in a meeting?": when a recording keeps going after the talking stops, Whimbrel asks before ending it.
- Meeting Prep briefs you can see: a banner and a sheet when a brief is ready, "Open brief" in Routines, and "Prep brief" on the meeting page. Each meeting in a recurring series gets its own brief (iPhone too).
- When every AI provider fails, Whimbrel says why: the Ollama model isn't installed, Ollama isn't running, or no provider is set up. A failed automatic summary shows the reason.
- When the Ollama model you use isn't installed, Whimbrel uses an installed one in the background and says so in Settings. Your choice is kept and doesn't sync.
- Long participant lists are collapsed (iPhone too).
- The question-alert email is sent only when you're away (Away Mode on, or 5 minutes idle).

### Windows
- Speaker names, monthly AI budgets, trim, and "Still in a meeting?", as on the Mac. Browser calls get the "Still in a meeting?" question too.
- A recording stops when the PC goes to sleep, instead of counting the sleep as meeting time.
- When every AI provider fails, Whimbrel says why, and it uses an installed Ollama model when the one you use isn't installed.
- Each meeting in a recurring series gets its own prep brief.
- Fixed a "file not found" error at the end of a recording.

## [0.9.56] - 2026-11-04

### Windows
- Send feedback attaches a diagnostics bundle (logs with personal details removed) and opens it in File Explorer, so it's easy to attach to an email.
- A meeting whose summary failed says why.

## [0.9.55] - 2026-11-04

### Mac
- Ask answers in a live meeting no longer overlap. An answer could spill past its card when the window offered it no width.

### Windows
- Routing, retention, prompts, routines and hidden episodes now sync with your Mac.
- A meeting opened from Today, Library or Ask shows its full page, with its stage, why a summary failed and Transcribe again. It no longer says recording arrives in a later version.
- Updater and Tauri updated (Tauri 2.12.5, updater 2.14.1).

### All platforms
- Dependency updates that clear security alerts (candle 0.11, zod 4.6.7).

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
