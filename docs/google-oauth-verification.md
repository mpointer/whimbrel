# Google OAuth verification kit

This document is for submitting Whimbrel's Google OAuth app for verification, so that anyone, not just test users, can connect Gmail and Google Calendar. Mike moved the app out of Testing on 2026-10-29.

## Status

- **Verification submitted:** 2026-10-30.
- **Demo video (unlisted):** https://youtu.be/UrIZPJp29fc
- **Next:** reply to Google's emails, then the CASA Tier 2 assessment when Google asks for it (see "Steps for Mike").

## Scopes Whimbrel requests

| Scope | Class | Requested by | Why Whimbrel needs it (paste as the justification) |
|---|---|---|---|
| `openid`, `email` (and `profile` for sync sign-in) | Non-sensitive | Every Google connection, and sync sign-in | Identifies which Google account is connected, so the app can label it in a list of several accounts and match a user's own devices when they turn on encrypted sync. |
| `calendar.readonly` | Sensitive | Calendar connection | Reads the user's events to show today's schedule, prepare a briefing before each meeting, and recognise which meeting the app is recording. Events are stored only in the app's local database on the user's computer. |
| `calendar.events` | Sensitive | Calendar connection; approved routine actions | Creates or updates a calendar event only after the user approves that exact change in the app, for example a follow-up meeting a routine proposed. |
| `gmail.readonly` | Restricted | Gmail connection | Reads the user's messages and threads to find conversations waiting on them, list open loops, and include them in briefings and recaps the user turns on. Messages are stored only in the app's local database on the user's computer and are never sent to our servers. |
| `gmail.compose` | Restricted | Gmail connection | Creates drafts the user asked for, for example an end-of-day recap or a weekly digest, which stay as drafts in the user's Gmail until they review and send them. |
| `gmail.send` | Sensitive | Approved routine actions | Sends an email only after the user approves that exact message in the app, for example a request to a meeting organizer for a recording. |

`gmail.readonly` and `gmail.compose` are restricted scopes. They need Google's review plus an annual third-party security assessment (CASA Tier 2) by an authorised lab. The other scopes need only the standard review.

Keeping `gmail.compose` costs nothing extra. The assessment covers the whole app, and `gmail.readonly` triggers it anyway.

## What reviewers check, and where it is

- **Verified domain:** `whimbrelapp.com`. The Google site-verification TXT record is already on the domain. Use the same Google account in Search Console and in Cloud Console.
- **Homepage:** https://whimbrelapp.com/
- **Privacy policy:** https://whimbrelapp.com/privacy/. Its "Google account data" section lists every scope, says data stays on the user's computer, rules out ads and AI training, discloses that hosted AI providers receive text only at the user's request and with the user's own key, and states the Limited Use sentence.
- **Terms of service:** https://whimbrelapp.com/tos/
- **App name, logo and support email** on the consent screen. Use "Whimbrel", the Whimbrel mark (`website/site/assets/icon-512.png` at 120×120), and `hello@whimbrelapp.com`.
- **Authorised domains:** `whimbrelapp.com`, and `futuremeanswered.com` if the support or developer email is on it.

## Demo video script (unlisted YouTube link, about 3 minutes)

Record on the Mac app in English, with the screen at a readable size. Show the browser address bar during consent, so the client ID is visible in the URL.

1. **Opening, 10 seconds.** Show the Whimbrel website and the app's main window.
2. **Calendar consent.**
   1. Settings → Connectors → Google Calendar → Connect.
   2. Show the full consent screen, with the app name and both calendar scopes.
   3. Approve.
3. **Calendar in use.** Today shows the day's meetings. Open a meeting's preparation brief.
4. **Calendar write.** A routine proposes a calendar change. The approval card shows it, the user approves it, and the event appears in Google Calendar.
5. **Gmail consent.** Settings → Connectors → Gmail → Connect. Show the consent screen with `gmail.readonly` and `gmail.compose` and approve.
6. **Gmail read in use.** The open-loops list shows a thread waiting on the user. Ask: "What emails are waiting on me?" The answer cites the thread.
7. **Gmail compose in use.** Run the end-of-day recap routine. A draft appears in Gmail's Drafts folder and is not sent.
8. **Gmail send in use.** An approval card shows a full email. The user approves it, and the message appears in Gmail's Sent folder.
9. **Where data lives.** Show Settings → the data folder on the user's computer. Say: "Gmail and Calendar data is stored only in this local database."
10. **Disconnecting.** Disconnect each account in Settings. Show `myaccount.google.com/permissions`, with Whimbrel's access removed.

## Steps for Mike

1. In Google Cloud Console, go to Google Auth Platform → Branding. Fill in the app name, logo, support email, homepage, privacy and terms links, and the authorised domains.
2. Under Data Access, add exactly the scopes in the table. Paste each justification and the video link.
3. Submit for verification. Google emails progress updates and questions.
4. When Google asks for the security assessment, pick a CASA Tier 2 lab from Google's App Defense Alliance list and schedule it. Whimbrel has no server that receives Google data, so the scope is the desktop apps and the website.
5. Until approval, users still see the "unverified app" warning, and restricted scopes are capped at 100 users.

Sources:
- [Restricted scope verification (Google)](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification)
- [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy)
