# 11 — Meeting & Episode Lifecycle Restoration: Implementation Record

**Status:** Implemented (Tasks 1-12 complete, plus a final cross-branch review fix wave, merged to branch).  
**Scope:** Atomic meeting deletion, episode hide/show, episode activity forgetting, episode regeneration, and foundational lifecycle menu UI.  
**Boundary:** Single-device durable; no cross-device sync for episode state changes yet; Phase 1 (1A/1B/1C) complete; Phase 2-6 deferred.

---

## Handoff Work Packages Addressed

This plan addresses the first two phases of the broader lifecycle restoration roadmap:

| Phase | Scope | Status |
|---|---|---|
| **Phase 0: Baseline Discipline** | This documentation file, proving complete design → code → test traceability | ✓ In this doc |
| **Phase 1A: Shared Meeting Lifecycle Menu** | Unified "Forget meeting…" affordance across all four meeting surfaces: `MeetingDetailView`, `TodayView`, `MeetingsListView` (all three via the shared `MeetingOptionsMenu` view), and `TimelineView` (its own context menu, deliberately — right-clicking a list row is a different interaction from a toolbar button — but with the same copy, the same confirmation text, the same "Open in new window" item, and the same `MeetingLifecycle.forgetMeeting` call) | ✓ Task 6, Timeline copy aligned in the final fix wave |
| **Phase 1B: Atomic Meeting Deletion** | Transactional delete operation at the Rust layer; rollback on failure | ✓ Task 1 |
| **Phase 1C: Episode Hide/Forget/Regenerate** | Episode visibility toggle, activity scavenging, summary re-generation, with device-local persistence | ✓ Tasks 2, 3, 4, 8, 9, 10 |
| **Phase 2: Layout & Visibility Workflow** | Responsive episode list redesign with show/hide controls, empty states | → Separate plan (not in scope) |
| **Phase 3: Preferences & Defaults** | Episode retention policy, auto-hide rules, user settings | → Separate plan (not in scope) |
| **Phase 4: Transcript Import** | User-supplied transcript upload for re-summarization | → Separate plan (not in scope) |
| **Phase 5: Connection Center** | Device pairing, cross-device sync of episode state | → Separate plan (not in scope) |
| **Phase 6: Broader Validation & Docs** | User-facing help, edge-case testing, accessibility audit | → Separate plan (not in scope) |

---

## Files Created

### Swift UI (Tasks 5-10)

**Production files:**
- `apps/macos/Whimbrel/UI/MeetingLifecycle.swift` (Task 5) — Not a state machine; a two-function enum enforcing one ordering rule (durable DB row deleted FIRST, local files cleaned up only after that succeeds):
  - `forgetMeetingSync(deleteRow: () -> Bool, deleteFiles: () -> Void) -> ForgetResult` — the pure, synchronously-testable core. Returns `.failed` without calling `deleteFiles` when `deleteRow` fails, so a failed delete never leaves a surviving row pointing at deleted files.
  - `forgetMeeting(store:meetingId:audioPath:transcriptPath:completion:)` — the async production wrapper: runs the above on `CoreMemoryStore.accessQueue`, calls back on the main queue.
  - `ForgetResult` is `.deleted` / `.deletedWithFileCleanupWarning` / `.failed`.
- `apps/macos/Whimbrel/UI/EpisodeOptionsMenu.swift` (Task 9, extended Task 10) — Menu for per-episode actions: hide/unhide, forget, regenerate
- `apps/macos/Whimbrel/UI/MeetingOptionsMenu.swift` (Task 6) — Menu for meeting-level actions: "Open in new window" and "Forget meeting…" (with confirmation)

**Test files:**
- `WhimbrelTests/MeetingLifecycleTests.swift` (Task 5)
- `WhimbrelTests/MeetingOptionsMenuTests.swift` (Task 6)
- `WhimbrelTests/SettingsViewDuplicateCleanupTests.swift` (Task 7)
- `WhimbrelTests/CoreBridgeEpisodeTests.swift` (Task 8)
- `WhimbrelTests/EpisodeOptionsMenuTests.swift` (Task 9, extended Task 10)

---

## Files Modified

### Core Memory Layer (Tasks 1-3)

**`core/memory/src/lib.rs`**
- Task 1: `delete_meeting` now transactional; fails atomically on any nested operation failure
- Task 2: Data migration added; `episodes.hidden` column creation + idempotent ALTER TABLE guard
- Task 3: Four domain methods added (signatures as they actually exist):
  - `set_episode_hidden(&self, id: i64, hidden: bool, now_unix_ms: i64) -> rusqlite::Result<bool>` — no device argument; `false` means no such episode id
  - `forget_episode_activity(&self, id: i64) -> rusqlite::Result<bool>` — reads the target episode's OWN `device_id` from its row, then deletes `kind='capture' AND device_id = <that episode's device> AND id BETWEEN start_item_id AND end_item_id`, plus the episode's summarize job and the episode row itself, in one transaction
  - `regenerate_episode_summary(&self, episode_id: i64, now_unix_ms: i64) -> rusqlite::Result<bool>` — requeues the existing `episode_summarize` job; `false` when no such job exists
  - `list_hidden_episodes(&self, _device_id: &str) -> rusqlite::Result<Vec<i64>>` — the `_device_id` parameter is retained for FFI/ABI stability and is deliberately unused (see "Known Limitation" below)

### FFI Layer (Tasks 4, 8)

**`core/ffi/src/lib.rs`** (Task 4)
- Four C-compatible wrappers for the Rust methods above (Task 4)
- 8 inline FFI-layer tests added (Task 4)

**`core/ffi/include/whimbrel_core.h`** (Task 8)
- Four FFI function declarations added (initially missing from Task 4, caught during Task 8 and fixed)

### Swift Bridges & UI (Tasks 6-10)

**`apps/macos/Whimbrel/Support/CoreBridge.swift`** (Task 8)
- Four Swift wrapper methods bridging the FFI calls to Rust

**`apps/macos/Whimbrel/UI/MeetingDetailView.swift`** (Task 6)
- Removed old unlabeled trash-icon delete UI
- `MeetingOptionsMenu` wired for meeting-level delete affordance

**`apps/macos/Whimbrel/UI/EpisodeDetailView.swift`** (Tasks 9, 10)
- Doc comment corrected (was falsely claiming read-only)
- `EpisodeOptionsMenu` wired for per-episode hide/forget/regenerate actions (Task 9)
- State added to track hidden status via `listHiddenEpisodeIds` (Task 10)

**`apps/macos/Whimbrel/UI/TodayView.swift`** (Task 6, final fix wave)
- `MeetingOptionsMenu` added to meeting items (previously zero delete affordance)
- `didBecomeKeyNotification` → `refresh()` added, so forgetting a meeting from the standalone detail window doesn't leave Today rendering the deleted meeting

**`apps/macos/Whimbrel/UI/MeetingsListView.swift`** (Task 6, final fix wave)
- `MeetingOptionsMenu` added to meeting list items
- Same `didBecomeKeyNotification` → `refresh()` hook, for the same reason

**`apps/macos/Whimbrel/UI/TimelineView.swift`** (Tasks 6, 10, final fix wave)
- `deleteMeeting(_:)` internals fixed to route through `MeetingLifecycle` (file-after-DB ordering fix)
- Episodes mode gained "Show hidden episodes" toggle
- Hidden episodes marked with badge
- Refocus-refresh fix for list consistency after hiding
- Meeting context menu aligned with the shared vocabulary ("Forget meeting…", "Forget this meeting?" and the same confirmation body) and gained "Open in new window". It keeps its own context-menu/alert wrapper rather than embedding `MeetingOptionsMenu` — a deliberate design choice for the right-click-a-row interaction, not an oversight.

**`apps/macos/Whimbrel/UI/SettingsView.swift`** (Task 7, reverted in the final fix wave)
- `cleanUpDuplicateMeetings()` deletes duplicate rows with a plain `store.deleteMeeting(id:)` and performs NO file cleanup. Task 7 added file cleanup on the "orphaned audio file" premise; the final cross-branch review found that premise does not hold on this path and the addition was a data-loss risk, so it was reverted. Duplicate groups are peer-sourced only (`find_duplicate_meeting_groups` filters `device_id != own_device_id`) and `apply_synced_meeting` always writes `audio_path` as NULL, so no local audio file can be orphaned here. `transcript_path` can be non-null, but every member of a group is the same sync payload applied repeatedly and they all share one path string — so deleting files for a dropped duplicate could destroy the file the KEPT row still references.

### Build Configuration

**`apps/macos/Whimbrel.xcodeproj/project.pbxproj`** (Tasks 5, 6, 7, 8, 9)
- Modified repeatedly to register each new Swift source file and test file with the build targets (necessary mechanical step; each new file requires `PBXBuildFile`, `PBXFileReference`, group listing, and `PBXSourcesBuildPhase` entries)

---

## Data Migration

### Schema Change

**Task 2 added one persistent column:**

```sql
ALTER TABLE episodes ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
```

(The initially-added `episodes.forgotten_at` column was removed during Task 3's post-review fix round after review found it would be dead code under the hard-delete design `forget_episode_activity` actually uses.)

### Idempotency Guarantee

The migration is idempotent and safe to re-run. It uses the codebase's established pattern (the same one `ensure_capture_item_source_column` uses) and is called once from the central `Store::init()`.

SQLite does NOT tolerate a repeated `ALTER TABLE ... ADD COLUMN` — it raises `duplicate column name`. Idempotency comes entirely from the guard in `Store::ensure_episodes_hidden_column`: it reads `PRAGMA table_info(episodes)` first and runs the `ALTER TABLE` only when `hidden` is absent from the result. On every run after the first, the `ALTER TABLE` statement is never reached at all.

---

## Test Coverage

### Test Functions Added (by Task)

**Rust layer (core/memory):**
- Task 1: `delete_meeting_is_atomic_and_rolls_back_on_failure`
- Task 2: `episodes_table_has_hidden_column_after_init`, `ensure_episodes_hidden_column_is_idempotent`
- Task 3 (including fix round): 8 tests covering `set_episode_hidden`, `forget_episode_activity` (including `forget_episode_activity_does_not_delete_out_of_scope_items` added in the fix round), `regenerate_episode_summary`, `list_hidden_episodes`

**FFI layer (core/ffi, Task 4):**
- `set_episode_hidden_true_then_false`
- `set_episode_hidden_null_handle_returns_false`
- `forget_episode_activity_returns_true_then_false_on_second_call`
- `forget_episode_activity_null_handle_returns_false`
- `regenerate_episode_summary_true_when_job_exists_false_when_it_does_not`
- `regenerate_episode_summary_null_handle_returns_false`
- `list_hidden_episodes_returns_populated_list_after_hiding_one`
- `list_hidden_episodes_null_handle_returns_null`

**Swift layer:**
- Task 5: `testForgetMeetingDeletesRowBeforeFiles`, `testForgetMeetingSkipsFileCleanupWhenRowDeleteFails`
- Task 6: `testForgetIsDisabledWhileRecording`, `testForgetIsEnabledWhenNotRecording`
- Task 7 (rewritten in the final fix wave): `testDroppedDuplicateRowIsDeletedAndSharedTranscriptFileSurvives`, `testSyncedDuplicateHasNoLocalAudioPathToOrphan` in `SettingsViewDuplicateCleanupTests.swift`. The original Task 7 tests were removed: they built a locally-inserted meeting with a peer `deviceId` AND real local files, a combination `apply_synced_meeting` cannot produce.
- Task 8: `testSetEpisodeHiddenRoundTrips`, `testSetEpisodeHiddenReturnsFalseForUnknownId`, `testRegenerateEpisodeSummaryRoundTrips`, `testRegenerateEpisodeSummaryReturnsFalseForUnknownId`, `testForgetEpisodeActivityRoundTrips`, `testForgetEpisodeActivityReturnsFalseForUnknownId`, `testListHiddenEpisodeIdsReturnsEmptyOnFreshStore`
- Task 9, extended Task 10: `testMenuLabelsMatchControlledVocabulary`

**Summary:** All added tests are green; all pre-existing tests continue to pass. Full test suite (Rust): `cd core && cargo test --workspace --exclude transcribe --exclude diarize --exclude ffi --exclude sync-engine && cargo build -p transcribe -p diarize -p ffi && cargo build -p sync-engine && cargo test -p ffi`. Full test suite (Swift): `cd apps/macos && xcodebuild test -project Whimbrel.xcodeproj -scheme Whimbrel -destination 'platform=macOS' -only-testing:WhimbrelTests CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY=""`.

---

## Known Limitation: Single-Device Durability Only

**Explicit constraint (stated plainly, not softened):**

Episode hide/forget/regenerate operations are durable on the device where the user acts (they survive the segmenter job re-running, app restarts, and local-only reads). **They do NOT propagate to other synced devices.**

**Why:** Episodes are not synced as database rows at all. Each device independently re-derives its own episodes from its own copy of synced `items`, so the same real-world activity can yield two separate episode rows on two devices, with different ids. Nothing propagates a lifecycle action from one to the other. A user hiding an episode on their laptop will not see it hidden on their desktop unless we build explicit cross-device sync for episode state (Phase 5), which is deferred.

**The two methods are scoped differently, and neither is scoped to "the device you're acting from":**

- `list_hidden_episodes` is **not device-scoped at all**. It queries `WHERE hidden = 1`, full stop. Hide/unhide is a local UI preference that applies to every episode this device displays, including episodes derived from a synced peer's captures. Its `_device_id` parameter survives only for FFI/ABI stability and is unused. (It previously filtered on `device_id = ?1`, which silently broke hiding for every peer-derived episode: the write side had no device check, so `hidden = 1` was set, but the viewing device could never read it back, never render the badge, and never offer "Unhide". Fixed in the final review fix wave.)
- `forget_episode_activity` **is** scoped — by the TARGET EPISODE's own recorded `device_id`, read from the episode row, not by the device the user is acting from. The practical consequence, stated plainly: forgetting a peer-captured episode DOES delete that peer's synced `items` from this device's local database. It is not restricted to "your own" episodes. The scoping exists because `items` is one unified table with a shared autoincrement id sequence, so ids interleave across devices via sync and the `start_item_id..end_item_id` range alone is not a safe filter. The deletion is local: it removes this device's copy, and does not propagate back to the peer that captured them.

**Where this is documented in code:** `ponytail:` comments at the relevant call sites (`EpisodeDetailView.loadEpisodeIfNeeded`, `TimelineView.refresh`'s `.episodes` branch) mark the no-sync ceiling, with cross-device sync of episode state as the upgrade path.

---

## Automated Validation (Task 12)

Run directly against the branch head after Task 11 landed (commit `d0d7955`):

| Suite | Command | Result |
|---|---|---|
| Rust workspace (excl. transcribe/diarize/ffi/sync-engine) | `cd core && cargo test --workspace --exclude transcribe --exclude diarize --exclude ffi --exclude sync-engine` | All green. `memory` crate: 271 passed, 0 failed, 8 ignored. Every other workspace crate: 0 failed. |
| Excluded crates, build-only | `cargo build -p transcribe -p diarize -p ffi` and `cargo build -p sync-engine` | Both succeed cleanly. |
| FFI crate tests | `cargo test -p ffi` | 106 passed, 0 failed, 4 ignored. |
| Swift `WhimbrelTests` | `cd apps/macos && xcodebuild test -project Whimbrel.xcodeproj -scheme Whimbrel -destination 'platform=macOS' -only-testing:WhimbrelTests CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY=""` | 256 tests, 0 failures. `TEST SUCCEEDED`. |
| Full app build | (performed as part of the Swift test run above, and separately during Task 6's review) | `BUILD SUCCEEDED`. |

Zero failures anywhere in the automated suite across all 12 tasks. No pre-existing failures were found or masked. The same suites were re-run after the final cross-branch review fix wave and stayed green.

**Not automatable in this session:** the manual QA checklist below requires interactive use of the running macOS app (clicking through menus, VoiceOver rotor navigation, visual light/dark comparison). No native-macOS GUI automation tool was available to the agent that produced this branch — only web-browser automation. This checklist is handed off for a human to execute; results should be filled in below once run.

## Manual QA Evidence

The following flows should be manually tested and their results recorded below. This checklist is left unchecked pending a human validation pass; do not pre-fill with assumed results.

| # | Flow | Evidence | Status |
|---|------|----------|--------|
| 1 | Delete a meeting from MeetingDetailView | Screenshot of delete menu, confirm flow; verify meeting disappears from all views (TodayView, MeetingsListView, TimelineView) and files are cleaned up | — |
| 2 | Delete a meeting from TodayView | Screenshot of meeting card → options menu → delete; verify removal | — |
| 3 | Delete a meeting from MeetingsListView | Screenshot of list item → options menu → delete; verify removal | — |
| 4 | Hide an episode in EpisodeDetailView | Screenshot of episode → options menu → hide; verify episode no longer appears in list until "Show hidden episodes" toggle is enabled | — |
| 5 | Show a hidden episode | Toggle "Show hidden episodes"; verify hidden episode reappears with badge | — |
| 6 | Forget episode activity | Select episode → options menu → forget; verify capture-type activity is removed; confirm screenshot of "forgot" state | — |
| 7 | Regenerate an episode summary | Select episode → options menu → regenerate; verify new summary is produced and displayed; check that old generation model/provider chip updates | — |
| 8 | Verify episode state is device-local | On Device A: hide an episode. Sync to Device B. On Device B: verify episode is still visible (proving no cross-device sync). | — |
| 9 | Verify atomic delete on failure | (Optional, requires test setup.) Trigger a delete and simulate a write failure mid-operation; confirm nothing is partially deleted and the meeting remains intact. | — |
| 10 | Light/dark mode rendering | Screenshots of delete menus and episode options menus in both light and dark mode; no rendering glitches. | — |
| 11 | Keyboard & VoiceOver labels | Keyboard-only navigation to menus; confirm Escape dismisses; verify accessibility identifiers present in code (already checked in Swift unit tests). | — |

---

## Commits

All work is on the `feat/meeting-episode-lifecycle-restoration` branch, created from `origin/main` at `01a2942` (the baseline the Deep Evaluation doc itself cites). Tasks 1-12 are complete and committed, followed by the final cross-branch review fix wave.

See the git log and the task reports under `/superpowers/sdd/2026-08-21-meeting-episode-lifecycle-restoration/` for the full audit trail.

---

**Next phases:**
- **Phase 2:** Responsive episode list redesign with show/hide controls
- **Phase 3:** Preferences pane for episode retention policy and auto-hide rules
- **Phase 4:** User-supplied transcript upload and re-summarization
- **Phase 5:** Cross-device sync for episode state
- **Phase 6:** Broader validation, user-facing help, accessibility audit
