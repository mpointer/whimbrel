# Skin loader test corpus

Used by `SkinLoaderTests` (Mac). This directory is never loaded as real skins: `SkinLoader.loadDirectory` reads only the top-level `*.json` of `skins/`. Each file and its expected outcome:

- `not-json.json`: malformed JSON, skipped.
- `no-id.json`: no `id`, skipped.
- `no-name.json`: no `name`, skipped.
- `bad-id.json`: id `Game Day` is not lowercase letters, digits and hyphens, skipped.
- `duplicate-a.json`, `duplicate-b.json`: both use id `dup`; in `loadDirectory` the first (by file name) loads and the second is skipped.
- `bad-font.json`: loads; `Comic Sans` is not allowed so display falls back to Whimbrel's, `SF Mono/light/italic` is kept, `SF Pro/wide` has an unknown weight so numerals fall back.
- `long-voice.json`: loads; `upNextHeader` (over 24) and `tagline` (empty) fall back to Whimbrel's wording, `greetingMorning` "Hello, {name}" (37 characters with a 30-character name) is kept.
- `bad-archetype.json`: loads; unknown archetype becomes `standard` and unknown `headerBand` becomes `none`.
- `out-of-range.json`: loads; radius 99 is clamped to 24 and borderWidth -3 to 0.
- `unknown-fields.json`: loads; unknown fields are ignored and `radius` 4 is kept.
- `bad-symbol.json`: loads; `today` is not an SF Symbol so it keeps Whimbrel's `sun.max`, `meetings` is kept.

See docs/superpowers/specs/2026-10-20-skins-design.md.
