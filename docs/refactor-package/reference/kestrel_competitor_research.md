# Competitive Research Notes — Granola and Littlebird

## Granola: Official transcription workflow

Source: https://docs.granola.ai/help-center/taking-notes/transcription

Granola's desktop app begins audio capture only after the user deliberately opens a scheduled meeting note, reacts to a notification, or creates a New Note for an ad-hoc call. It does not silently transcribe a meeting merely because it is on a calendar. While active, it shows both a green animated capture indicator inside the note and a repositionable, cross-application floating meeting indicator. Its transcript interface supports show/hide, stop/resume without ending the meeting, full and chunk-level copy, chunk deletion, and search.

Granola states it can end transcription through an explicit stop action, call-end detection, a 15-minute no-audio timeout, or computer sleep. It warns users about the limits: macOS automatic call-end detection requires admin rights, and back-to-back calls can merge if the meeting app keeps the microphone active; manual stop is positioned as the reliable override. Granola captures combined system audio plus microphone, keeps system and user utterances visually distinct, and sends audio directly to a transcription provider without saving audio/video. It is live-only and does not accept prerecorded audio imports.

## Littlebird: Official meeting-notes workflow

Source: https://littlebird.ai/features/meeting-notes

Littlebird presents bot-free transcription from computer audio, claiming compatibility with any meeting platform. Its public flow is: connect calendar, join or launch a meeting, then automatic transcription starts; users review a summary, action items, and transcript after the call. It also describes a deliberate pre-meeting "Prep for meeting" action that composes context from email, prior meetings, and relevant projects. During a call it supports asking a question without leaving the meeting, while its broader product positioning centers on cross-application work memory and recall.

## Initial implication for Kestrel

A competitive product must make recording state impossible to miss, make stop/resume an always-available and deterministic control, keep a live transcript independently inspectable, expose graceful automatic behavior with clearly explained fallbacks, and convert captured content into grounded long-term work context rather than isolated summaries.

## Research limits

This is public-product research. Claims have been kept within the vendors' own documentation and product pages; behavior must be separately validated through hands-on testing before being treated as a contract-level requirement.

## Re-verification

On 2026-08-27, Granola's official transcription documentation was independently re-reviewed. The verified controls and caveats above are suitable for comparison with Kestrel's recording-state machine and desktop controls. The key principle is explicit user-intent to begin capture, persistent visibility while capture is active, and a reliable manual stop override regardless of automatic detection quality.

## Granola: Official calendar synchronization workflow

Source: https://docs.granola.ai/help-center/getting-started/syncing-your-calendars

Granola scopes calendar sync to Google and Microsoft/Outlook, shows a bounded five-item upcoming-meeting view by default to keep the home surface uncluttered, allows selection of additional calendars, and expects changes to appear within a few minutes. It excludes declined events and non-meeting blocks. It documents duplicate-event causes and the user remedy, and identifies its related-meeting identity approach: recurring event ID plus exact title. Users can deliberately separate work and personal accounts through an account switcher. This is a useful usability baseline, not necessarily a sufficient data architecture for Kestrel.

## Ollama: Official streaming contract

Source: https://docs.ollama.com/capabilities/streaming

Ollama documents streaming as a supported response mode for rendering content as it is produced. It is enabled by default in the REST API and must be enabled explicitly in SDK clients. Streaming chunks can carry partial assistant content, thinking content, and tool calls. The client is responsible for incrementally accumulating fields to maintain correct subsequent conversation context. For Kestrel, this makes a non-streaming chat response a product implementation deficit rather than an Ollama platform constraint. Cancellation must be implemented by closing or aborting the in-flight client request and discarding late chunks using an application-level generation token; cancellation semantics and cleanup must be validated against the exact Rust HTTP client in use.

## Google Gemini: Official streaming contract

Source: https://ai.google.dev/gemini-api/docs/streaming

Google's Gemini Interactions API can stream responses with `stream: true` over server-sent events. The documented event model exposes interaction creation and status, step lifecycle events, incremental text (and other typed) deltas, completion, and structured error events. Therefore the addition of Google models should not necessitate a second incompatible or blocking UI path. A provider-neutral inference adapter can normalize each provider's events into common `started`, `delta`, `tool-progress`, `usage`, `completed`, `cancelled`, and `failed` application events. Kestrel should render partial visible output immediately, preserve an explicit stop control while work is in flight, and handle late or failed stream messages without changing the completed message state.

## macOS UX quality bar

Source: https://developer.apple.com/design/human-interface-guidelines/layout

Apple's guidance is explicit that layouts should adapt gracefully while remaining familiar as context changes. It recommends grouping related elements, giving essential information sufficient space, and keeping controls distinct from content. Kestrel's reported overlap after additional Google model controls is therefore a reliability defect, not simply a cosmetic issue: the window must retain stable hierarchy at small sizes, large text, localization expansion, and every provider/model count. Prefer SwiftUI layout primitives or equivalent constraint-driven composition, not calculated absolute frames.

## Littlebird: Public privacy posture

Source: https://littlebird.ai/privacy-policy

Littlebird publicly describes a broader work-memory product that may collect screen, browser, supported-app, chat, and task/relationship data. Its stated posture includes encryption in transit and at rest, user access/export/deletion options, opt-outs for feature-level collection, and a claim that user data is not used to train models. Kestrel need not capture the same broad data to compete; it should instead make data boundaries, local-versus-cloud processing, model-provider transmission, retention, and deletion unmistakable at the moment of capture and inference. Granola's limited audio-retention model and Littlebird's memory-with-controls model represent two different but instructive trust patterns.
