# 07 — Sync relay & web viewer (Phase 2 design)

Answers open question #3 in `06-risks-open-questions.md` ("Sync relay hosting") and extends FR-SYNC (`02-functional-requirements.md`) with a concrete design for a thin, E2E-encrypted, self-hostable relay plus a companion web viewer — the "path 1" option from the desktop-app-vs-web discussion: a read-mostly client that decrypts locally, never a server that holds plaintext.

**Scope note:** this is being built for personal/small-group use (the primary user plus a handful of partners), not public distribution or competing with the real Littlebird. That shapes the choices below toward "simple and self-hostable for a closed set of trusted devices" rather than multi-tenant SaaS — no signup flow, no billing, no scale headroom beyond a household's worth of devices. If that changes later, revisit before building on top of this.

## Goals

- Multi-device sync of the local core's data, per FR-SYNC-1…4.
- A companion web viewer: browse the timeline, search, and chat (mirrors `TimelineView.swift`'s Recent/Keyword/Semantic/Ask/Digest modes) from any browser.
- The relay never holds plaintext or keys, ever — matches the existing architecture doc's description almost exactly, this just fills in the "how."
- Self-hostable end to end: a first-party hosted instance and a self-hosted one run identical code against S3-compatible storage.

## Non-goals

- Multi-tenant accounts/billing — already out of scope per the roadmap ("Team/multi-user features").
- Public signup — device (and web-viewer) enrollment is invite/pairing based against a closed device set, not username/password or OAuth.
- Capture from the web viewer — browsers can't do OS-level screen/audio capture the way the native app does; the viewer is read/chat only, matching NFR-PRV-1's on-device-by-default model.

## Architecture

### Sync relay (`server/`)

Thin service, matching `04-architecture.md`'s existing description: authenticated blob store + WebSocket fan-out of encrypted change feeds. Never holds plaintext or keys.

- **Ciphertext blobs** (encrypted `capture_item`/`meeting`/`transcript_segment`/`summary` payloads, encrypted client-side before upload) → S3-compatible object store.
- **Metadata** (device registry, blob pointers/versions, change-feed cursors) → a small SQL DB. This is the `device` table already in `04-architecture.md`'s data model (id, pubkey, name, platform, enrolled_at, revoked_at) plus a blob-pointer table — nothing exotic.
- **Auth**: device keypairs, not accounts. A new device (or the web viewer) enrolls via existing-device approval per FR-SYNC-2 — a pairing code/QR scanned from an already-enrolled Mac, not a login form.
- The relay's job is fan-out and storage, full stop. It structurally cannot answer "what's in this blob" — there's no decryption capability anywhere on the server side to build in the first place.

### E2E encryption model

- Group key for the user's device set (already specced in `04-architecture.md`'s `sync` crate: "per-device keypairs, group key for the user's device set").
- Each device encrypts its own writes with the group key before upload; any enrolled device — including the web viewer, once it holds the key — can decrypt.
- The web viewer gets the group key the same way a new native device would: pairing with an already-enrolled device transfers the key over an authenticated encrypted channel (not through the relay), after which the key only ever leaves the browser as ciphertext.
- Browser-side key storage: WebCrypto with non-extractable `CryptoKey`s where possible + IndexedDB — the same trust model Signal's and Bitwarden's web clients use.

### Web viewer (new: `apps/web/`)

- Read-mostly: timeline browse, keyword/semantic search, chat with citations. No capture, no calendar triggers, no transcription — those need OS-level access and stay native-only.
- Decrypts fetched ciphertext blobs client-side; plaintext never touches any server, including this one's own backend if it has one.
- LLM calls (Ask/Digest modes): can't reach Ollama running on your Mac from a browser without extra exposure work, which is out of scope here. Simplest for personal use: the browser calls a hosted model (Anthropic/OpenAI) directly with a user-supplied API key stored locally — keeps the relay out of the LLM path entirely and preserves the "what left my machine" ledger concept (NFR-PRV-4) at the browser layer.
- Framework: Next.js fits the existing default stack — but note it splits across hosts (see below): the viewer itself is close to a static app once ciphertext is fetched, while the relay needs a long-running WebSocket-capable process that serverless platforms don't do well.

## Hosting recommendation

| Component | Recommendation | Why |
|---|---|---|
| Sync relay (auth + WS fan-out) | Fly.io or Railway, one small container | Needs persistent WebSocket connections — serverless platforms handle that poorly. Both give a plain Docker deploy that's trivially self-hostable (same image, different config). |
| Ciphertext blob storage | Cloudflare R2 (hosted) / MinIO (self-host) | S3-compatible API — one code path for both. R2 has no egress fees, which matters for a sync workload; MinIO is the standard self-host drop-in for the same API. |
| Metadata (device registry, cursors) | Postgres (hosted) or SQLite (self-host, single node) | Small dataset — a household's worth of devices, not a multi-tenant workload. Managed Postgres (Fly/Railway) if you want it; SQLite is genuinely enough otherwise. |
| Web viewer | Vercel (or any static host) | No server-side rendering of user data needed — it's a thin client once ciphertext is fetched, so plaintext never touches a server at all. |

At this scale (you + a few partners), a single ~$5–10/mo Fly.io or Railway instance plus R2's free tier covers it — no reason to build for scale that isn't there.

## What this unlocks vs. defers

- **Unlocks**: FR-SYNC-1…4 (E2E sync, device registry, consolidated timeline, offline-first/CRDT) plus a read/chat web viewer.
- **Defers**: FR-SYNC-5 (single-capturer coordination — only matters with 2+ native instances), FR-SYNC-7 (federated search, P3), any capture from the web viewer (architecturally out of reach without a lot more scope), Windows/Android clients (Phase 3/4).

## Sequencing

This is genuinely Phase 2 work — `core/sync` is currently an empty stub in the architecture diagram, nothing real exists yet. Roughly in order:

1. `core/sync`: per-device keypairs + group key + encrypt/decrypt of `capture_item`/`meeting`/etc. payloads.
2. `server/`: the relay itself (auth, blob store, WS fan-out) — start self-hosted-only, add a hosted option once it's proven.
3. Device enrollment/pairing, native side first — a second Mac syncing with the first is the smallest real end-to-end test of the whole loop, before a browser is in the picture at all.
4. `apps/web/`: the viewer, once there's actually something on the relay worth viewing.

Not recommended to start before the hands-on test pass on the current native app (M1.1–M1.5) finishes — this is a large new surface area, and none of this session's work has been exercised live yet.
