# Better transcripts, item 1.2: measurements

M4 Pro, macOS 27.2. Five-minute excerpts of three real meetings. Numbers only, no meeting text.

**These are not WER.** Mike deferred WER on 2026-10-31, so there are no hand-corrected references. "Difference" is the word error rate against large-v3-turbo's own output, which shows how far an engine sits from turbo, not which is right.

| Meeting | Audio | Words (turbo) | tiny.en | base | `SpeechAnalyzer` |
|---|---|---|---|---|---|
| #457, four people, Zoom | system track | 486 | 0.210 | 0.165 | 0.138 |
| #161, Zoom | mic track, compressed | 747 | 0.197 | 0.206 | 0.143 |
| #210, Zoom | system track, compressed | 703 | 0.132 | 0.128 | 0.088 |

Realtime speed:

| Meeting | tiny.en | base | large-v3-turbo | `SpeechAnalyzer` |
|---|---|---|---|---|
| #457 | 142× | 70× | 8.3× | 124× |
| #161 | 33× | 22× | 4.3× | 76× |
| #210 | 126× | 45× | 3.3× | 77× |

The tiny.en, base and turbo runs used `core/transcribe/examples/measure.rs`. The `SpeechAnalyzer` runs used a one-off script with the same API.

Rejected as test meetings: #455 (clipped room audio; its stored transcript repeats one line) and #436 (silent system track, no stored transcript).

Large-v3-turbo ran at 3.3× on real audio on this Mac, under the 4× live floor. It stays a batch engine in practice.
