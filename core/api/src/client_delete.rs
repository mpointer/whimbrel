//! "Delete this client" (X6 client controls, spec 4.2-4.5): the client and the material Whimbrel holds on them,
//! on this device and, through tombstones, on the others that apply them (what stays is listed under "left for
//! you").
//!
//! [`delete_client`] removes the client's linked material and the possible matches the user ticked, each
//! item in its own transaction with its tombstone for other devices, then the derived text that names the
//! client, the meeting files inside the folders it is given, the vault notes, and last the client's seed
//! topic itself (hard-deleted: no name kept). It keeps going when one kind fails, lists that kind in
//! `failed`, and leaves the client in place so a re-run finds what is left; a re-run skips what is gone.
//!
//! The [`Receipt`] holds a date, counts per kind and the places Whimbrel cannot reach. It carries no client
//! name, alias or text. The stored copy (kv `client_delete_receipt:<date>`) keeps only counts and the
//! categories of the "left for you" list, never a path (a folder name can carry the client's name).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::client_inventory::{ClientItem, ItemKind, KnownFolders, LeftForYou};
use crate::envelope::{ApiError, ApiResult};
use memory::{ClientItemKind, TombstoneOutcome};

/// What the user confirmed on the review screen.
#[derive(Debug, Clone, Default)]
pub struct DeleteSelection {
    /// The client's seed topic key.
    pub key: String,
    /// The possible matches the user ticked (unticked ones are never deleted).
    pub include_possible: Vec<ClientItem>,
    /// Delete the export folders Whimbrel recorded for this client (`client_export`; only those, and only when
    /// each is still a real folder at its recorded path, named as an export folder). Off: they stay in "left
    /// for you".
    pub delete_exports: bool,
    /// The folder the Obsidian vault export writes to (it holds `Whimbrel/`), when the user has one and vault
    /// sync is on.
    pub vault: Option<PathBuf>,
    /// Vault sync is off: the vault is never touched (even if `vault` is set), and "left for you" says so.
    pub vault_off: bool,
}

/// What happened to the client's own record (its seed topic, name and aliases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientRecord {
    /// Deleted here and on the other devices (a tombstone is queued).
    Removed,
    /// Deleted here only: this device has no sync id, so the other devices were not told.
    RemovedHereOnly,
    /// Still here: something failed, so the client stays (partly present) for a re-run.
    StillPresent,
}

/// A step of the delete that is not one kind of item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteStage {
    /// Removing derived text that names the client (summaries, caches, suggestions, names).
    DerivedText,
    /// Deleting the client's own record.
    ClientRecord,
    /// The final clean-up (unused names, search indexes).
    Cleanup,
    /// Removing meeting files or vault notes owed from the pending record.
    Files,
    /// This device has no sync id, so the meeting files could not be told apart from paths another device
    /// sent and none were removed; they are listed under `FilesNotRemoved`. Task 9 sets the id first.
    NoDeviceId,
}

/// Shown once after a delete. No names, no text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Receipt {
    pub date_unix_ms: i64,
    /// Items deleted by this run, per kind.
    pub counts: Vec<(ItemKind, usize)>,
    /// Of `counts`: deleted here, but other devices were not told (this device has no sync id, or the item
    /// came from another device without an identity to name). "Not sent to other devices".
    pub not_sent: Vec<(ItemKind, usize)>,
    /// Of `counts`: items that only ever existed on this device (Ask conversations, captures never synced).
    pub local_only: Vec<(ItemKind, usize)>,
    pub client: ClientRecord,
    pub left_for_you: Vec<LeftForYou>,
    /// Devices on the sync relay that have not updated to a build that applies these deletes. `None`: not
    /// known yet (sync has not measured it); never shown as 0.
    pub pending_devices: Option<usize>,
    /// Kinds with at least one item that could not be deleted; they are still here. A ticked possible match
    /// the inventory no longer lists but that is still stored counts here too.
    pub failed: Vec<ItemKind>,
    /// Steps that did not finish. For Tasks 9 and 10: `NoDeviceId` is not retryable. Tell the user to set up
    /// this device's sync id and do not offer "try again"; every other stage (and `failed`) is cured by
    /// running the same selection again.
    pub failed_stages: Vec<DeleteStage>,
    /// The encryption upgrade's plaintext rollback copy (`<db>.plain.bak`) was there and this run removed it
    /// (X4 design 3.5 step 8). Shown as one fixed sentence; never a path.
    pub rollback_copy_removed: bool,
}

impl Receipt {
    /// Nothing failed: every selected item, the derived text and the client record are gone.
    pub fn complete(&self) -> bool {
        self.failed.is_empty() && self.failed_stages.is_empty() && self.client != ClientRecord::StillPresent
    }
}

/// kv prefix of the stored receipts.
pub const RECEIPT_PREFIX: &str = "client_delete_receipt:";

/// Deletes client `sel.key` and its material (see the module docs). `media_roots` are the only folders a
/// meeting's audio, transcript and slide files are removed from; a file anywhere else is listed under
/// [`LeftForYou::FilesNotRemoved`].
///
/// `sync` is the sync group key and this device's sync id when sync is set up: the relay clean-up targets
/// are then sealed at once (an own calendar event's target holds its raw UID, such as `kickoff@client.com`,
/// until sealed). Pass `None` only when sync is not set up.
///
/// `Err(ApiError::NotFound)` when `sel.key` is not a live client (unknown, deleted, merged into another):
/// nothing is touched. Other errors only when nothing could be read; per-item failures are in the receipt.
/// The work outside the rows (meeting files, vault notes) is recorded before the rows go and finished by a
/// re-run if a step fails or the app stops part-way.
pub fn delete_client(
    store: &memory::Store,
    sel: &DeleteSelection,
    media_roots: &[PathBuf],
    sync: Option<(&sync::GroupKey, &str)>,
) -> ApiResult<Receipt> {
    use memory::client_delete::{PENDING_ENTITY, PENDING_TOPIC};
    let now = crate::util::now_unix_ms();
    let vault = sel.vault.clone().filter(|_| !sel.vault_off);
    let folders = KnownFolders { media: media_roots.first().cloned(), vault: vault.clone(), vault_off: sel.vault_off };
    let inv = crate::client_inventory::inventory_lean(store, &sel.key, &folders)?;
    // Held only for this call, never stored: what derived text is matched against.
    let terms = store.seed_chain_terms(&sel.key).map_err(db)?;
    let scope = store.client_scope(&sel.key).map_err(db)?;
    let run = memory::Store::client_delete_run_id(&sel.key);
    // What earlier keep-for-N-months expiries removed here only, and what they still owe (their captures,
    // files and notes): the delete adopts the owed work, so it is finished and reaches other devices.
    let expired_here = store.client_expired_count(&sel.key).map_err(db)?;
    store
        .adopt_client_delete_pending(&memory::Store::client_expiry_run_id(&sel.key), &run)
        .map_err(db)?;
    let ids = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>();
    store.add_client_delete_pending(&run, PENDING_TOPIC, &ids(&scope.topic_ids)).map_err(db)?;
    store.add_client_delete_pending(&run, PENDING_ENTITY, &ids(&scope.entity_ids)).map_err(db)?;

    // What the inventory lists as linked, and the possible matches the user ticked that it still finds. A
    // ticked item it no longer lists but that is still stored was not deleted: it counts as failed, so the
    // receipt never reads complete. One already gone (an earlier run took it) is done.
    let mut tally = Tally::default();
    let mut items: Vec<&ClientItem> = inv.linked.iter().collect();
    for item in &sel.include_possible {
        let listed = inv.possible.iter().any(|p| p.kind == item.kind && p.id == item.id);
        if listed {
            if !items.iter().any(|x| x.kind == item.kind && x.id == item.id) {
                items.push(item);
            }
            continue;
        }
        let still_here = match item.kind {
            ItemKind::VaultNote | ItemKind::ExportFolder => Ok(false),
            ItemKind::Meeting => item
                .id
                .parse::<i64>()
                .map_err(|_| rusqlite::Error::InvalidQuery)
                .and_then(|id| store.get_meeting(id).map(|m| m.is_some())),
            ItemKind::Capture => store.client_item_exists(None, &item.id),
            ItemKind::Email => store.client_item_exists(Some(ClientItemKind::Email), &item.id),
            ItemKind::Thread => store.client_item_exists(Some(ClientItemKind::Thread), &item.id),
            ItemKind::CalendarEvent => store.client_item_exists(Some(ClientItemKind::CalendarEvent), &item.id),
            ItemKind::Episode => store.client_item_exists(Some(ClientItemKind::Episode), &item.id),
            ItemKind::Conversation => store.client_item_exists(Some(ClientItemKind::Conversation), &item.id),
            ItemKind::Step => store.client_item_exists(Some(ClientItemKind::Step), &item.id),
        };
        if !matches!(still_here, Ok(false)) {
            tally.fail(item.kind);
        }
    }
    // Steps first, so the ones filed from a deleted email are counted as steps; episodes before captures,
    // since deleting a capture also takes the episodes built over it.
    items.sort_by_key(|i| match i.kind {
        ItemKind::Step => 0,
        ItemKind::Episode => 1,
        _ => 2,
    });

    let secure_before = store.set_secure_delete(true).ok();
    let mut stages: Vec<DeleteStage> = Vec::new();
    let mut left_files: Vec<PathBuf> = Vec::new();
    for item in items {
        let row_id = item.id.parse::<i64>().map_err(|_| rusqlite::Error::InvalidQuery);
        let done = match item.kind {
            ItemKind::VaultNote | ItemKind::ExportFolder => continue,
            ItemKind::Step => store.delete_client_item(ClientItemKind::Step, &item.id),
            ItemKind::Meeting => row_id.and_then(|id| delete_meeting(store, &run, id)),
            ItemKind::Capture => match item.id.strip_prefix("i:") {
                // A capture held only in `items`.
                Some(items_id) => items_id
                    .parse::<i64>()
                    .map_err(|_| rusqlite::Error::InvalidQuery)
                    .and_then(|_| store.delete_client_item(ClientItemKind::Capture, items_id)),
                None => row_id.and_then(|id| store.delete_client_capture(id)),
            },
            ItemKind::Email => row_id.and_then(|_| store.delete_client_item(ClientItemKind::Email, &item.id)),
            ItemKind::Thread => row_id.and_then(|_| store.delete_client_item(ClientItemKind::Thread, &item.id)),
            ItemKind::CalendarEvent => {
                row_id.and_then(|_| store.delete_client_item(ClientItemKind::CalendarEvent, &item.id))
            }
            ItemKind::Episode => row_id.and_then(|_| store.delete_client_episode(&run, &item.id)),
            ItemKind::Conversation => {
                row_id.and_then(|_| store.delete_client_item(ClientItemKind::Conversation, &item.id))
            }
        };
        match done {
            Ok(Some(outcome)) => tally.deleted(item.kind, outcome),
            Ok(None) => {}
            Err(_) => tally.fail(item.kind),
        }
    }

    // Captures of episodes deleted (now or by an earlier run) that the inventory no longer lists.
    match store.client_delete_pending(&run) {
        Ok(pending) => {
            let mut done = Vec::new();
            for (kind, value) in pending.into_iter().filter(|(k, _)| k == memory::client_delete::PENDING_CAPTURE) {
                match store.delete_client_item(ClientItemKind::Capture, &value) {
                    Ok(Some(outcome)) => {
                        tally.deleted(ItemKind::Capture, outcome);
                        done.push((kind, value));
                    }
                    Ok(None) => done.push((kind, value)),
                    Err(_) => tally.fail(ItemKind::Capture),
                }
            }
            if store.clear_client_delete_pending(&run, &done).is_err() {
                stages.push(DeleteStage::Files);
            }
        }
        Err(_) => tally.fail(ItemKind::Capture),
    }

    // Meeting files, from the pending record (this run's meetings, and any an earlier run left).
    let roots: Vec<&Path> = media_roots.iter().map(PathBuf::as_path).collect();
    let files = store.client_delete_pending(&run).and_then(|pending| finish_meeting_files(store, &pending, &roots));
    match files {
        Ok(files) => {
            left_files.extend(files.left);
            if files.stuck || store.clear_client_delete_pending(&run, &files.done).is_err() {
                stages.push(DeleteStage::Files);
            }
        }
        Err(_) => stages.push(DeleteStage::Files),
    }

    if !terms.is_empty() {
        let set = llm::guard::ExclusionSet::new(terms.clone());
        if store.purge_client_text(Some(&run), &|t: &str| set.find_in(t)).is_err() {
            stages.push(DeleteStage::DerivedText);
        }
        // Kept word fingerprints of questions and windows that named the client (fix round 5, N1).
        if store.purge_input_words_naming(&terms).is_err() {
            stages.push(DeleteStage::DerivedText);
        }
    }
    if store.finish_client_delete().is_err() {
        stages.push(DeleteStage::Cleanup);
    }

    // Vault notes of what is gone: deleted meetings, entities no longer here, and the client's own topics
    // once nothing has failed (while the client stays, the export keeps its notes).
    let with_topics = tally.failed.is_empty() && stages.is_empty();
    let owners = store.client_delete_pending(&run).and_then(|pending| vault_owners(store, &pending, with_topics));
    match owners {
        Ok((owners, used)) => {
            // Vault sync off: the notes are the user's; the owed note record is cleared all the same.
            let removed = match &vault {
                Some(vault) => crate::topics_export::delete_vault_notes(vault, &owners).map(|r| {
                    left_files.extend(r.unchecked);
                    r.removed
                }),
                None => Ok(0),
            };
            match removed {
                Ok(n) => {
                    tally.add_count(ItemKind::VaultNote, n);
                    if store.clear_client_delete_pending(&run, &used).is_err() {
                        stages.push(DeleteStage::Files);
                    }
                }
                Err(_) => tally.fail(ItemKind::VaultNote),
            }
        }
        Err(_) => tally.fail(ItemKind::VaultNote),
    }

    // Export folders, when asked: only paths recorded for this client, each checked (fail closed). One that
    // no longer passes the checks is left and listed. One that could not be removed, or whose parent cannot be
    // reached (a drive unplugged), keeps its record, is listed and fails the kind (a re-run finishes it).
    // Kept apart from `left_files`, which decides the no-device-id reason (that is about meeting files only).
    let mut exports_not_removed: Vec<PathBuf> = Vec::new();
    if sel.delete_exports {
        use crate::client_export::FolderRemoval;
        match store.client_exports(&sel.key) {
            Ok(paths) => {
                for p in paths {
                    match crate::client_export::remove_export_folder(store, &p) {
                        Ok(Ok(FolderRemoval::Refused)) => exports_not_removed.push(PathBuf::from(&p)),
                        Ok(Ok(FolderRemoval::Unreachable)) => {
                            exports_not_removed.push(PathBuf::from(&p));
                            tally.fail(ItemKind::ExportFolder);
                        }
                        Ok(Ok(found)) => {
                            if found == FolderRemoval::Removed {
                                tally.add_count(ItemKind::ExportFolder, 1);
                            }
                            if store.forget_client_export(&p).is_err() {
                                tally.fail(ItemKind::ExportFolder);
                            }
                        }
                        Ok(Err(_)) | Err(_) => tally.fail(ItemKind::ExportFolder),
                    }
                }
            }
            Err(_) => tally.fail(ItemKind::ExportFolder),
        }
    }
    // What is left of them, read before the client record (and with it these records) goes.
    let exports_left = store.client_exports(&sel.key);

    // Without a sync id the forget can't tell this device's meeting files from paths another device sent, so
    // it removes none (fail closed). They are listed as left, and the receipt says why.
    if store.own_device_id().is_none() && !left_files.is_empty() {
        stages.push(DeleteStage::NoDeviceId);
    }

    left_files.extend(exports_not_removed);

    // X4 design 3.5 step 8: the encryption upgrade's plaintext rollback copy holds this client's material in the
    // clear. Removed before the client record goes, so a copy that would not go keeps the client for a re-run.
    let mut rollback_copy_removed = false;
    if let Some(db) = store.db_path() {
        let paths = crate::encrypt_migration::MigrationPaths::beside(&db);
        match crate::encrypt_migration::remove_plain_backup_for_client_delete(&paths) {
            Ok(removed) => rollback_copy_removed = removed,
            Err(_) => stages.push(DeleteStage::Files),
        }
    }

    let client = if tally.failed.is_empty() && stages.iter().all(|s| *s == DeleteStage::NoDeviceId) {
        // Nothing is owed any more: the pending record goes before the client record does.
        let cleared = store
            .client_delete_pending(&run)
            .and_then(|rest| store.clear_client_delete_pending(&run, &rest));
        match cleared.and_then(|_| store.forget_seed_topic(&sel.key)) {
            Ok(TombstoneOutcome::Queued) => ClientRecord::Removed,
            Ok(_) => ClientRecord::RemovedHereOnly,
            Err(_) => {
                stages.push(DeleteStage::ClientRecord);
                ClientRecord::StillPresent
            }
        }
    } else {
        ClientRecord::StillPresent
    };
    if client != ClientRecord::StillPresent && store.finish_client_delete().is_err() {
        stages.push(DeleteStage::Cleanup);
    }
    if let Some((group_key, own_device_id)) = sync {
        sync_engine::seal_client_cleanup(store, group_key, own_device_id);
    }
    if let Some(before) = secure_before {
        let _ = store.set_secure_delete(before);
    }

    let mut left_for_you = inv.left_for_you;
    for l in left_for_you.iter_mut() {
        if let (LeftForYou::ExportFolders(listed), Ok(left)) = (l, &exports_left) {
            *listed = left.iter().map(PathBuf::from).collect();
        }
    }
    left_for_you.push(LeftForYou::CopiesOnOtherDevices);
    if expired_here > 0 {
        left_for_you.push(LeftForYou::ExpiredHereOnly(expired_here));
    }
    if !left_files.is_empty() {
        left_files.sort();
        left_files.dedup();
        left_for_you.push(LeftForYou::FilesNotRemoved(left_files));
    }
    let receipt = Receipt {
        date_unix_ms: now,
        counts: tally.counts,
        not_sent: tally.not_sent,
        local_only: tally.local_only,
        client,
        left_for_you,
        // Unreadable reads as unknown, never as "every device is current".
        pending_devices: store.pending_tombstone_devices().ok().flatten(),
        failed: tally.failed,
        failed_stages: stages,
        rollback_copy_removed,
    };
    // Shown once; kept as counts and categories only. A receipt that can't be stored is still shown.
    if let Ok(json) = serde_json::to_string(&StoredReceipt::from(&receipt)) {
        let _ = store.set_meta(&format!("{RECEIPT_PREFIX}{now}"), &json);
    }
    Ok(receipt)
}

/// Forgets meeting `id` for client delete `run`: its files and vault owner id are recorded as pending first
/// (so a crash after the rows commit still finds them), then the rows go. `None` when already gone.
pub(crate) fn delete_meeting(store: &memory::Store, run: &str, id: i64) -> rusqlite::Result<Option<TombstoneOutcome>> {
    if !record_meeting_pending(store, run, id)? {
        return Ok(None);
    }
    Ok(store.forget_meeting_reporting(id)?.map(|(_, outcome)| outcome))
}

/// Records meeting `id`'s files and vault owner id as pending work of `run` (`client_delete_pending`
/// rows, returned), before its rows go. `false` (nothing recorded) when there is no such meeting.
pub(crate) fn record_meeting_pending(store: &memory::Store, run: &str, id: i64) -> rusqlite::Result<bool> {
    Ok(!meeting_pending_rows(store, run, id)?.is_empty())
}

/// [`record_meeting_pending`], returning the `(kind, value)` rows it recorded (empty: no such meeting).
pub(crate) fn meeting_pending_rows(
    store: &memory::Store,
    run: &str,
    id: i64,
) -> rusqlite::Result<Vec<(String, String)>> {
    use memory::client_delete::{PENDING_AUDIO, PENDING_MEETING, PENDING_NAMED, PENDING_SLIDES, PENDING_TRANSCRIPT};
    let Some(files) = store.meeting_files_for_client_delete(id)? else {
        return Ok(Vec::new());
    };
    let named = store.meeting_file_paths(id)?;
    let tag = |paths: &[String]| paths.iter().map(|p| format!("{id}\t{p}")).collect::<Vec<_>>();
    let slides: Vec<String> = files.snapshots_dir.iter().cloned().collect();
    let mut rows = vec![(PENDING_MEETING, vec![id.to_string()])];
    rows.push((PENDING_AUDIO, tag(&files.audio_paths)));
    rows.push((PENDING_TRANSCRIPT, tag(&files.transcript_paths)));
    rows.push((PENDING_SLIDES, tag(&slides)));
    rows.push((PENDING_NAMED, tag(&named)));
    let mut out = Vec::new();
    for (kind, values) in rows {
        store.add_client_delete_pending(run, kind, &values)?;
        out.extend(values.into_iter().map(|v| (kind.to_string(), v)));
    }
    Ok(out)
}

/// What [`finish_meeting_files`] did.
pub(crate) struct MeetingFiles {
    /// Pending rows finished.
    pub(crate) done: Vec<(String, String)>,
    /// Paths left for the user: outside the media folders, or never handed back for deletion.
    pub(crate) left: Vec<PathBuf>,
    /// An audio or transcript file inside the media folders that should have gone is still on disk: its
    /// meeting's rows stay pending for a re-run.
    pub(crate) stuck: bool,
}

/// `path` resolves inside one of `roots` (both canonicalised; `..` and symlinked folders resolved).
fn inside_roots(path: &Path, roots: &[&Path]) -> bool {
    let Ok(real) = path.canonicalize() else { return false };
    roots.iter().filter_map(|r| r.canonicalize().ok()).any(|r| real.starts_with(r))
}

/// Removes the pending files of meetings that are gone: audio and transcripts only inside `roots` (the
/// guarded retention delete resolves each path and checks it is inside a root and named as Whimbrel names
/// them), a slide folder only when it resolves inside a root and carries Whimbrel's marker with nothing
/// foreign. A meeting still here keeps its rows pending and its files untouched.
pub(crate) fn finish_meeting_files(
    store: &memory::Store,
    pending: &[(String, String)],
    roots: &[&Path],
) -> rusqlite::Result<MeetingFiles> {
    use memory::client_delete::{PENDING_AUDIO, PENDING_NAMED, PENDING_SLIDES, PENDING_TRANSCRIPT};
    let mut by_meeting: std::collections::BTreeMap<i64, Vec<(String, String)>> = Default::default();
    let mut out = MeetingFiles { done: Vec::new(), left: Vec::new(), stuck: false };
    for (kind, value) in pending {
        if ![PENDING_AUDIO, PENDING_TRANSCRIPT, PENDING_SLIDES, PENDING_NAMED].contains(&kind.as_str()) {
            continue;
        }
        match value.split_once('\t').and_then(|(m, p)| Some((m.parse::<i64>().ok()?, p))) {
            Some((m, path)) => by_meeting.entry(m).or_default().push((kind.clone(), path.to_string())),
            None => out.done.push((kind.clone(), value.clone())),
        }
    }
    for (m, files) in by_meeting {
        if store.get_meeting(m)?.is_some() {
            continue;
        }
        let pick = |k: &str| files.iter().filter(|(kind, _)| kind == k).map(|(_, p)| p.clone()).collect::<Vec<_>>();
        let (audio, transcripts, slides) = (pick(PENDING_AUDIO), pick(PENDING_TRANSCRIPT), pick(PENDING_SLIDES));
        crate::retention::delete_all_meeting_files_within(
            &memory::DeletedMeetingFiles {
                audio_path: None,
                transcript_path: None,
                snapshots_dir: None,
                audio_paths: audio.clone(),
                transcript_paths: transcripts.clone(),
            },
            roots,
        );
        for dir in slides.iter().filter(|d| inside_roots(Path::new(d), roots)) {
            crate::retention::delete_all_meeting_files_within(
                &memory::DeletedMeetingFiles {
                    audio_path: None,
                    transcript_path: None,
                    snapshots_dir: Some(dir.clone()),
                    audio_paths: Vec::new(),
                    transcript_paths: Vec::new(),
                },
                roots,
            );
        }
        let mut handed: Vec<String> = audio;
        handed.extend(transcripts);
        let stuck = files_still_there(&handed).into_iter().any(|p| inside_roots(&p, roots));
        if stuck {
            out.stuck = true;
            continue;
        }
        let mut check = pick(PENDING_NAMED);
        check.extend(handed);
        check.extend(slides);
        out.left.extend(files_still_there(&check));
        for (kind, path) in files {
            out.done.push((kind, format!("{m}\t{path}")));
        }
    }
    Ok(out)
}

/// The vault note owners pending for a run, among what is gone: meetings no longer here, entities no longer
/// here, and (when `with_topics`) the client's topics. Returns them and the pending rows they come from.
pub(crate) fn vault_owners(
    store: &memory::Store,
    pending: &[(String, String)],
    with_topics: bool,
) -> rusqlite::Result<(crate::topics_export::VaultOwners, Vec<(String, String)>)> {
    use memory::client_delete::{PENDING_ENTITY, PENDING_MEETING, PENDING_TOPIC};
    let mut owners = crate::topics_export::VaultOwners::default();
    let mut used = Vec::new();
    let mut entities = Vec::new();
    for (kind, value) in pending {
        let Ok(id) = value.parse::<i64>() else { continue };
        match kind.as_str() {
            PENDING_TOPIC if with_topics => {
                owners.topics.push(id);
                used.push((kind.clone(), value.clone()));
            }
            PENDING_ENTITY => entities.push(id),
            PENDING_MEETING => {
                if store.get_meeting(id)?.is_none() {
                    owners.meetings.push(id);
                    used.push((kind.clone(), value.clone()));
                }
            }
            _ => {}
        }
    }
    for id in store.missing_entities(&entities)? {
        owners.entities.push(id);
        used.push((PENDING_ENTITY.to_string(), id.to_string()));
    }
    Ok((owners, used))
}

/// The paths (and an audio file's mic/system siblings) still on disk.
fn files_still_there(named: &[String]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for a in named {
        if let Some((system, mic)) = crate::retention::sibling_audio_paths(a) {
            paths.extend([system, mic]);
        }
        paths.push(PathBuf::from(a));
    }
    paths.sort();
    paths.dedup();
    paths.retain(|p| p.symlink_metadata().is_ok());
    paths
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

#[derive(Default)]
struct Tally {
    counts: Vec<(ItemKind, usize)>,
    not_sent: Vec<(ItemKind, usize)>,
    local_only: Vec<(ItemKind, usize)>,
    failed: Vec<ItemKind>,
}

pub(crate) fn bump(list: &mut Vec<(ItemKind, usize)>, kind: ItemKind, n: usize) {
    if n == 0 {
        return;
    }
    match list.iter_mut().find(|(k, _)| *k == kind) {
        Some((_, c)) => *c += n,
        None => {
            list.push((kind, n));
            list.sort_by_key(|(k, _)| *k);
        }
    }
}

impl Tally {
    fn deleted(&mut self, kind: ItemKind, outcome: TombstoneOutcome) {
        bump(&mut self.counts, kind, 1);
        match outcome {
            TombstoneOutcome::Queued => {}
            TombstoneOutcome::NoDeviceId => bump(&mut self.not_sent, kind, 1),
            TombstoneOutcome::LocalOnly => match kind {
                // A meeting pulled with no identity to name exists on other devices too: not sent, not local.
                ItemKind::Meeting => bump(&mut self.not_sent, kind, 1),
                _ => bump(&mut self.local_only, kind, 1),
            },
        }
    }

    /// Files: never on other devices through sync, so neither sent nor not sent.
    fn add_count(&mut self, kind: ItemKind, n: usize) {
        bump(&mut self.counts, kind, n);
        bump(&mut self.local_only, kind, n);
    }

    fn fail(&mut self, kind: ItemKind) {
        if !self.failed.contains(&kind) {
            self.failed.push(kind);
            self.failed.sort();
        }
    }
}

/// The stored copy of a [`Receipt`]: counts, outcomes and the categories of "left for you", no paths.
#[derive(Serialize)]
struct StoredReceipt<'a> {
    date_unix_ms: i64,
    counts: &'a [(ItemKind, usize)],
    not_sent: &'a [(ItemKind, usize)],
    local_only: &'a [(ItemKind, usize)],
    client: ClientRecord,
    left_for_you: Vec<&'static str>,
    pending_devices: Option<usize>,
    failed: &'a [ItemKind],
    failed_stages: &'a [DeleteStage],
    rollback_copy_removed: bool,
}

impl<'a> From<&'a Receipt> for StoredReceipt<'a> {
    fn from(r: &'a Receipt) -> Self {
        StoredReceipt {
            date_unix_ms: r.date_unix_ms,
            counts: &r.counts,
            not_sent: &r.not_sent,
            local_only: &r.local_only,
            client: r.client,
            left_for_you: r.left_for_you.iter().map(category).collect(),
            pending_devices: r.pending_devices,
            failed: &r.failed,
            failed_stages: &r.failed_stages,
            rollback_copy_removed: r.rollback_copy_removed,
        }
    }
}

fn category(l: &LeftForYou) -> &'static str {
    match l {
        LeftForYou::MediaFolder(_) => "media_folder",
        LeftForYou::VaultFolder(_) => "vault_folder",
        LeftForYou::ExportFolders(_) => "export_folders",
        LeftForYou::MailboxOriginals => "mailbox_originals",
        LeftForYou::BackupsAndCloud => "backups_and_cloud",
        LeftForYou::RelayCopiesBeforeThisRelease => "relay_copies_before_this_release",
        LeftForYou::FilesNotRemoved(_) => "files_not_removed",
        LeftForYou::CopiesOnOtherDevices => "copies_on_other_devices",
        LeftForYou::ExpiredHereOnly(_) => "expired_here_only",
        LeftForYou::VaultSyncOff => "vault_sync_off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    const NAME: &str = "Zorblax";
    const ALIAS: &str = "Quuxco";

    struct Env {
        dir: tempfile::TempDir,
        store: memory::Store,
        c: Connection,
    }

    fn env(device: Option<&str>) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        // The encrypted store when built with `sqlcipher` (X4 Task 12: X6's suite on it).
        let store = crate::test_store::open(&path, dir.path());
        if let Some(d) = device {
            store.set_own_device_id(d);
        }
        let c = crate::test_store::raw(&path);
        Env { dir, store, c }
    }

    fn count(e: &Env, sql: &str) -> i64 {
        e.c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Files source `kind`/`r` in topic `topic`.
    fn file(e: &Env, topic: i64, kind: &str, r: i64, ts: i64) -> i64 {
        let sid: i64 = match e.c.query_row(
            "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
            params![kind, r],
            |x| x.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                e.c.execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 1)",
                    params![kind, r, ts],
                )
                .unwrap();
                e.c.last_insert_rowid()
            }
        };
        e.c.execute(
            "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')",
            params![topic, sid],
        )
        .unwrap();
        sid
    }

    fn meeting(e: &Env, title: &str, start: i64, audio: Option<String>) -> i64 {
        let id = e
            .store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1000),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: audio,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "A".into(),
                text: format!("{title}: we talked"),
            }])
            .unwrap();
        id
    }

    fn email(e: &Env, ext: &str, subject: &str, body: &str, thread: &str) -> i64 {
        e.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-a".into(),
                external_id: ext.into(),
                ts_unix_ms: 1_000,
                from: "a@x.com".into(),
                to: "me@x.com".into(),
                subject: subject.into(),
                body_text: body.into(),
                gmail_thread_id: thread.into(),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    fn calendar(e: &Env, ext: &str, title: &str) -> i64 {
        e.store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".into(),
                source: "google_calendar".into(),
                external_id: ext.into(),
                ts_unix_ms: 2_000,
                title: title.into(),
                actors: vec![],
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'calendar_event' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    /// `(capture_item.id, items.id)`.
    fn capture(e: &Env, ts: i64, text: &str) -> (i64, i64) {
        let cap = e
            .store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev-a".into(),
                ts_unix_ms: ts,
                app_bundle_id: "com.apple.TextEdit".into(),
                window_title: "Doc".into(),
                url: None,
                text: text.into(),
                hash: ts,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap();
        let item = e
            .c
            .query_row(
                "SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1",
                [cap],
                |r| r.get(0),
            )
            .unwrap();
        (cap, item)
    }

    struct World {
        a: memory::topics::seeds::SeedTopic,
        b: memory::topics::seeds::SeedTopic,
        m_a: i64,
        m_shared: i64,
        m_b: i64,
        e_a: i64,
        e_b: i64,
        loose_email: i64,
        cal_a: i64,
        loose_capture: i64,
        conv: i64,
        episode: i64,
        org: i64,
        /// Retention stubs: A's own, shared with B naming A, shared with B not naming A.
        stubs: [i64; 3],
        /// A's stub also in an auto topic.
        stub_auto: i64,
        audio_in: PathBuf,
        audio_out: PathBuf,
        media: PathBuf,
    }

    /// Client A (Zorblax, alias Quuxco) and client B (Globex), one meeting they share, A's own meeting,
    /// email, calendar event, episode and step, derived text naming A in every cache, and possible matches
    /// (an email, a capture, a conversation) that only name A.
    fn world(e: &Env) -> World {
        let a = e.store.create_seed_topic(NAME, &[ALIAS.to_string()], None).unwrap();
        let b = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let media = e.dir.path().join("media");
        let outside = e.dir.path().join("elsewhere");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let audio_in = media.join("system-audio-1.m4a");
        let audio_out = outside.join("system-audio-2.m4a");
        std::fs::write(&audio_in, b"x").unwrap();
        std::fs::write(media.join("mic-audio-1.m4a"), b"x").unwrap();
        std::fs::write(&audio_out, b"x").unwrap();

        let m_a = meeting(e, &format!("{NAME} kickoff"), 10_000, Some(audio_in.to_string_lossy().into()));
        let m_shared = meeting(e, &format!("{NAME} and Globex joint"), 20_000, Some(audio_out.to_string_lossy().into()));
        let m_b = meeting(e, "Globex review", 30_000, None);
        file(e, a.topic_id, "meeting", m_a, 10_000);
        let shared_src = file(e, a.topic_id, "meeting", m_shared, 20_000);
        file(e, b.topic_id, "meeting", m_shared, 20_000);
        file(e, b.topic_id, "meeting", m_b, 30_000);
        // A person only A's meeting knows, by voice.
        e.store.upsert_person("p-1", "Pat", Some("pat@example.com")).unwrap();
        e.c.execute(
            "INSERT INTO meeting_speaker (meeting_id, speaker_id, embedding, person_key, status, updated_at_unix_ms) \
             VALUES (?1, 0, x'00000000', 'p-1', 'confirmed', 1)",
            [m_a],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO meeting_followup (meeting_id, text, created_at_unix_ms) VALUES (?1, ?2, 1)",
            params![m_a, format!("Send {NAME} the notes")],
        )
        .unwrap();

        let e_a = email(e, "g-a", &format!("{NAME} contract"), "see attached", "t-a");
        let e_b = email(e, "g-b", "Globex invoice", "due friday", "t-b");
        let loose_email = email(e, "g-loose", "Notes", &format!("call with {ALIAS} went well"), "t-loose");
        for id in [e_a, e_b] {
            e.c.execute(
                "INSERT OR REPLACE INTO items_vec (rowid, embedding) VALUES (?1, zeroblob(?2))",
                params![id, (memory::EMBEDDING_OUTPUT_DIM * 4) as i64],
            )
            .unwrap();
        }
        let src_e_a = file(e, a.topic_id, "email", e_a, 1_000);
        file(e, b.topic_id, "email", e_b, 1_000);
        e.c.execute(
            "INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, \
             last_message_from, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('gmail', 't-a', 'me@x.com', ?1, 1, 1, 'a@x.com', ?2, 'open', 1, 1)",
            params![format!("{NAME} contract"), format!("{NAME} wants changes")],
        )
        .unwrap();

        let cal_a = calendar(e, "kickoff-1@zorblax.com#100", &format!("{NAME} kickoff"));
        file(e, a.topic_id, "calendar_event", cal_a, 2_000);
        e.c.execute(
            "INSERT INTO calendar_presence (source, external_id, account, start_unix_ms, present) \
             VALUES ('google_calendar', 'kickoff-1@zorblax.com#100', 'me@x.com', 100000, 1)",
            [],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO meeting_prep (event_id, title, start_unix_ms, generated_at_unix_ms, brief_json, provider, model) \
             VALUES ('kickoff-1@zorblax.com@100000', ?1, 100000, 1, ?2, 'p', 'm')",
            params![format!("{NAME} kickoff"), format!("{{\"goal\":\"win {NAME}\"}}")],
        )
        .unwrap();

        // An episode A holds, over one capture; and a loose capture naming A.
        let (_, ep_item) = capture(e, 5_000, &format!("{NAME} spreadsheet"));
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, \
             end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', 'com.apple.TextEdit', 'Doc', 5000, 5000, ?1, ?1, 1, ?2, 'summarized', 1, 1)",
            params![ep_item, format!("Worked on the {NAME} budget")],
        )
        .unwrap();
        let episode = e.c.last_insert_rowid();
        file(e, a.topic_id, "episode", episode, 5_000);
        let (loose_capture, _) = capture(e, 900_000, &format!("draft for {ALIAS}"));
        let conv = e.store.insert_conversation(&format!("what did {ALIAS} say?"), 1).unwrap();
        e.store
            .append_conversation_exchange(conv, &format!("what did {ALIAS} say?"), "they agreed", "[]", None, 2)
            .unwrap();
        // Main's per-answer hash of that Ask answer (a later client delete must clear it).
        e.store.record_answer_provenance("they agreed", false, 2).unwrap();

        // Steps: A's, one filed on B from A's email, and B's own.
        for (key, topic, source_key, sid, text) in [
            ("s-a", &a.key, "email:g-a", Some(src_e_a), format!("Send {NAME} the SOW")),
            ("s-from-a-on-b", &b.key, "email:g-a", Some(src_e_a), format!("Ask {NAME} legal")),
            ("s-b", &b.key, "email:g-b", None, "Pay Globex".to_string()),
        ] {
            e.c.execute(
                "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
                 asked_at_unix_ms, updated_at_unix_ms, source_id) VALUES (?1, ?2, ?3, 'email', 't', ?4, 'mine', 1, 1, ?5)",
                params![key, topic, source_key, text, sid],
            )
            .unwrap();
        }
        let _ = shared_src;

        // Meetings past meeting retention: their sources are stubs (`ref_id` NULL, title, text, vector and
        // mentions kept; `topics::schema::stub_sources_for_retention`). A's own, one shared with B that names A,
        // and one shared with B that does not.
        let stub_a = stub(e, &format!("{NAME} planning call"), &format!("Planning with {ALIAS} on the rollout"), &[a.topic_id]);
        let stub_shared_named =
            stub(e, &format!("{NAME} and Globex sync"), "Both teams on the rollout", &[a.topic_id, b.topic_id]);
        let stub_shared_plain = stub(e, "Quarterly partners call", "Budget for the year", &[a.topic_id, b.topic_id]);
        // Filed under A by Jev (no name in it) and auto-joined to a topic no client holds: A's, it goes.
        e.c.execute("INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('Rollout', 1, 1)", []).unwrap();
        let auto_topic = e.c.last_insert_rowid();
        let stub_auto = stub(e, "Rollout call", "Pricing for the rollout", &[a.topic_id, auto_topic]);

        // An organization extracted from A's email, and derived text naming A everywhere.
        e.c.execute(
            "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('org', 'zorblax', ?1, 1)",
            [NAME],
        )
        .unwrap();
        let org = e.c.last_insert_rowid();
        e.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![org, src_e_a]).unwrap();
        e.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![org, stub_a]).unwrap();
        e.c.execute(
            "INSERT INTO routine (name, template, schedule_hhmm, created_at_unix_ms) VALUES ('Daily', 't', '09:00', 1)",
            [],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, 1, 'ok', ?1, '{}')",
            [format!("{NAME} needs a reply; Globex paid")],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, 2, 'ok', 'Globex paid', '{}')",
            [],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO mcp_daily_digest_cache (day, digest_json, generated_at_unix_ms) VALUES ('2026-11-12', ?1, 1)",
            [format!("[\"{ALIAS} call\"]")],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO jobs (job_type, payload_json, created_at_unix_ms, updated_at_unix_ms) VALUES ('x', ?1, 1, 1)",
            [format!("{{\"q\":\"{NAME}\"}}")],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO profile_suggestion (text, status, created_at_unix_ms) VALUES (?1, 'pending', 1)",
            [format!("You work with {NAME}")],
        )
        .unwrap();
        // A failed summary's model reply, quoted in the job's error note (X6 final review, Major 1); and one
        // that does not name A (a thread summary of A's mail can fail without a name in the cut).
        e.store.record_summarize_error("episode_summarize", &format!("no JSON object: \"Worked on the {NAME} budget\""), 1);
        e.store.record_summarize_error("thread_summarize", "no JSON object: \"the contract changes\"", 1);
        // Built from A's meeting without naming A: a synced suggestion, a follow-up's drafted run, a digest day.
        e.store.add_profile_suggestion("You run weekly kickoffs", Some(m_a), 1).unwrap();
        e.c.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, 3, 'ok', 'Draft: thanks for today', '{}')",
            [],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO meeting_followup (meeting_id, text, routine_run_id, created_at_unix_ms) VALUES (?1, 'Reply', ?2, 1)",
            params![m_a, e.c.last_insert_rowid()],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO mcp_daily_digest_cache (day, digest_json, generated_at_unix_ms) VALUES ('2026-11-11', '[\"kickoff at 10\"]', 1)",
            [],
        )
        .unwrap();
        World {
            a,
            b,
            m_a,
            m_shared,
            m_b,
            e_a,
            e_b,
            loose_email,
            cal_a,
            loose_capture,
            conv,
            episode,
            org,
            stubs: [stub_a, stub_shared_named, stub_shared_plain],
            stub_auto,
            audio_in,
            audio_out,
            media,
        }
    }

    /// A retention stub filed in `topics`, with its text and a vector. Returns its source id.
    fn stub(e: &Env, title: &str, text: &str, topics: &[i64]) -> i64 {
        e.c.execute(
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, stub, indexed_at_unix_ms) VALUES ('meeting', NULL, 500, ?1, 1, 1)",
            [title],
        )
        .unwrap();
        let sid = e.c.last_insert_rowid();
        e.c.execute("INSERT INTO topic_source_text (source_id, text) VALUES (?1, ?2)", params![sid, text]).unwrap();
        e.c.execute(
            "INSERT INTO topic_source_vec (rowid, embedding) VALUES (?1, zeroblob(?2))",
            params![sid, (memory::EMBEDDING_OUTPUT_DIM * 4) as i64],
        )
        .unwrap();
        for t in topics {
            e.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![t, sid])
                .unwrap();
        }
        sid
    }

    /// A delete on a device with sync set up (the shell passes its group key).
    fn run(e: &Env, sel: &DeleteSelection, roots: &[PathBuf]) -> ApiResult<Receipt> {
        delete_client(&e.store, sel, roots, Some((&sync::GroupKey::from_bytes([7; 32]), "dev-a")))
    }

    fn select_all(e: &Env, w: &World) -> DeleteSelection {
        let inv = crate::client_inventory::inventory(&e.store, &w.a.key).unwrap();
        DeleteSelection {
            key: w.a.key.clone(),
            include_possible: inv.possible.clone(),
            delete_exports: false,
            vault: None,
            vault_off: false,
        }
    }

    /// Every (table, row) whose text, cast column by column, holds the client's name or alias in any case.
    /// FTS and vec shadow tables are binary index pages, checked by MATCH below instead.
    fn rows_naming_client(e: &Env) -> Vec<String> {
        rows_naming(e, &["zorblax", "quuxco"])
    }

    /// [`rows_naming_client`] for any lower-case single-word `words`.
    fn rows_naming(e: &Env, words: &[&str]) -> Vec<String> {
        let tables: Vec<String> = e
            .c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut hits = Vec::new();
        for t in tables {
            if t.contains("_fts") || t.contains("_vec") {
                continue;
            }
            let cols: Vec<String> = e
                .c
                .prepare(&format!("SELECT name FROM pragma_table_info('{t}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            if cols.is_empty() {
                continue;
            }
            let concat = cols.iter().map(|c| format!("coalesce(CAST(\"{c}\" AS TEXT), '')")).collect::<Vec<_>>().join(" || '|' || ");
            let n: i64 = e
                .c
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM \"{t}\" WHERE {}",
                        words.iter().map(|w| format!("lower({concat}) LIKE '%{w}%'")).collect::<Vec<_>>().join(" OR ")
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if n > 0 {
                hits.push(format!("{t}: {n}"));
            }
        }
        for fts in ["items_fts", "transcript_segment_fts", "capture_item_fts"] {
            for term in words {
                let n: i64 =
                    count(e, &format!("SELECT COUNT(*) FROM {fts} WHERE {fts} MATCH '\"{}\"'", term.to_lowercase()));
                if n > 0 {
                    hits.push(format!("{fts} MATCH {term}: {n}"));
                }
            }
            // The index vocabulary itself: an optimized index keeps no deleted term.
            e.c.execute_batch(&format!(
                "DROP TABLE IF EXISTS temp.v; CREATE VIRTUAL TABLE temp.v USING fts5vocab(main, {fts}, 'row');"
            ))
            .unwrap();
            let list = words.iter().map(|w| format!("'{w}'")).collect::<Vec<_>>().join(", ");
            let n = count(e, &format!("SELECT COUNT(*) FROM temp.v WHERE term IN ({list})"));
            if n > 0 {
                hits.push(format!("{fts} vocabulary"));
            }
            // And the stored index pages: a plain delete leaves the term in its segment until it is merged.
            let n = count(
                e,
                &format!(
                    "SELECT COUNT(*) FROM {fts}_data WHERE {}",
                    words.iter().map(|w| format!("instr(block, CAST('{w}' AS BLOB)) > 0")).collect::<Vec<_>>().join(" OR ")
                ),
            );
            if n > 0 {
                hits.push(format!("{fts} index pages: {n}"));
            }
        }
        hits
    }

    #[test]
    fn delete_removes_every_table_row_for_client() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        assert!(!rows_naming_client(&e).is_empty(), "the world names the client before");
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(rows_naming_client(&e), Vec::<String>::new());
        // Row-level: every kind of A's material is gone.
        for (sql, what) in [
            (format!("SELECT COUNT(*) FROM meeting WHERE id IN ({}, {})", w.m_a, w.m_shared), "meetings"),
            (format!("SELECT COUNT(*) FROM items WHERE id IN ({}, {}, {})", w.e_a, w.cal_a, w.loose_email), "items"),
            (format!("SELECT COUNT(*) FROM episodes WHERE id = {}", w.episode), "episode"),
            (format!("SELECT COUNT(*) FROM capture_item WHERE id = {}", w.loose_capture), "capture"),
            ("SELECT COUNT(*) FROM items WHERE kind = 'capture'".to_string(), "episode's captures"),
            (format!("SELECT COUNT(*) FROM conversation WHERE id = {}", w.conv), "conversation"),
            ("SELECT COUNT(*) FROM threads WHERE external_thread_id = 't-a'".to_string(), "thread"),
            ("SELECT COUNT(*) FROM topic_step WHERE key IN ('s-a', 's-from-a-on-b')".to_string(), "steps"),
            ("SELECT COUNT(*) FROM meeting_speaker".to_string(), "voice prints"),
            ("SELECT COUNT(*) FROM person".to_string(), "orphan person"),
            ("SELECT COUNT(*) FROM meeting_followup".to_string(), "follow-up"),
            ("SELECT COUNT(*) FROM calendar_presence".to_string(), "presence"),
            ("SELECT COUNT(*) FROM meeting_prep".to_string(), "brief"),
            (format!("SELECT COUNT(*) FROM entity WHERE id = {}", w.org), "org"),
            ("SELECT COUNT(*) FROM items_vec WHERE rowid NOT IN (SELECT id FROM items)".to_string(), "vectors"),
            (format!("SELECT COUNT(*) FROM items_vec WHERE rowid = {}", w.e_a), "A's vector"),
            ("SELECT COUNT(*) FROM profile_suggestion".to_string(), "suggestions"),
            ("SELECT COUNT(*) FROM routine_run WHERE output_summary LIKE 'Draft%'".to_string(), "follow-up run"),
            ("SELECT COUNT(*) FROM mcp_daily_digest_cache".to_string(), "digest cache"),
            ("SELECT COUNT(*) FROM kv_meta WHERE key LIKE 'last_error:%'".to_string(), "model error notes"),
            ("SELECT COUNT(*) FROM answer_provenance".to_string(), "answer hashes"),
        ] {
            assert_eq!(count(&e, &sql), 0, "{what}");
        }
        // Retention stubs of A's meetings go with their text, vector and mentions; a stub shared with B goes only
        // when it names A (B keeps the other, still filed).
        let [stub_a, stub_named, stub_plain] = w.stubs;
        for sid in [stub_a, stub_named, w.stub_auto] {
            for (table, col) in
                [("topic_source", "id"), ("topic_source_text", "source_id"), ("topic_source_vec", "rowid"), ("entity_mention", "source_id")]
            {
                assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM {table} WHERE {col} = {sid}")), 0, "stub {sid} in {table}");
            }
        }
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM topic_source_vec WHERE rowid = {stub_plain}")), 1);
        assert_eq!(
            count(&e, &format!("SELECT COUNT(*) FROM topic_member WHERE source_id = {stub_plain} AND topic_id = {}", w.b.topic_id)),
            1,
            "B keeps its shared stub"
        );
        // B is intact: its seed, its own meeting, email, step and the run that only named B.
        assert!(e.store.list_seed_topics().unwrap().iter().any(|s| s.key == w.b.key && s.name == "Globex"));
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.m_b)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.e_b)), 1);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM topic_step WHERE key = 's-b'"), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items_vec WHERE rowid = {}", w.e_b)), 1, "B's vector");
        assert_eq!(count(&e, "SELECT COUNT(*) FROM routine_run WHERE output_summary = 'Globex paid'"), 1);
        // B's email and its own meeting's transcript; the shared meeting's went.
        assert_eq!(count(&e, "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'globex'"), 2);
        assert!(!w.audio_in.exists() && !w.media.join("mic-audio-1.m4a").exists(), "audio inside the media folder goes");
    }

    #[test]
    fn shared_meeting_deleted_for_both() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.m_shared)), 0);
        let b = crate::client_inventory::inventory(&e.store, &w.b.key).unwrap();
        assert!(!b.linked.iter().any(|i| i.id == w.m_shared.to_string()));
        assert!(b.linked.iter().any(|i| i.id == w.m_b.to_string()));
        assert_eq!(r.counts.iter().find(|(k, _)| *k == ItemKind::Meeting).map(|x| x.1), Some(2));
    }

    #[test]
    fn unticked_possible_match_survives() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let mut sel = select_all(&e, &w);
        sel.include_possible.clear();
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.loose_email)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM capture_item WHERE id = {}", w.loose_capture)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM conversation WHERE id = {}", w.conv)), 1);
        // The email and the capture's search copy.
        assert_eq!(count(&e, "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'quuxco'"), 2);
    }

    #[test]
    fn ticked_possible_match_deleted() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let mut sel = select_all(&e, &w);
        sel.include_possible.retain(|i| i.kind == ItemKind::Email);
        assert_eq!(sel.include_possible.len(), 1);
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.loose_email)), 0);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM conversation WHERE id = {}", w.conv)), 1);
        // The email's tombstone went, so other devices delete their copy too.
        assert_eq!(
            count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'client_item_forgotten' AND aggregate_id = 'email:g-loose'"),
            1
        );
    }

    #[test]
    fn receipt_contains_no_client_text() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let mut sel = select_all(&e, &w);
        // A shared item: the inventory names B as the other holder; the receipt must not carry it either.
        sel.include_possible.push(ClientItem {
            kind: ItemKind::Email,
            id: w.e_b.to_string(),
            also_client: Some("Globex".into()),
            date_unix_ms: 0,
            removed: false,
            label: None,
            excerpt: None,
            app: None,
        });
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        // Not a possible match of A: a ticked item the inventory does not list is never deleted, and the
        // receipt says it was not.
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.e_b)), 1);
        assert_eq!(r.failed, vec![ItemKind::Email]);
        assert!(!r.complete());
        let mut texts = vec![serde_json::to_string(&r).unwrap(), format!("{r:?}")];
        texts.extend(
            e.c.prepare("SELECT value FROM kv_meta WHERE key LIKE 'client_delete_receipt:%'")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
        assert!(texts.len() >= 3, "the receipt is stored");
        texts.extend(
            e.c.prepare("SELECT aggregate_id || payload_json FROM sync_outbox WHERE operation_kind = 'tombstone'")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
        for (i, t) in texts.iter().enumerate() {
            let l = t.to_lowercase();
            for term in ["zorblax", "quuxco", "globex"] {
                assert!(!l.contains(term), "{term} in {t}");
            }
            // The receipt does not even hold the client's opaque key (a tombstone does: it is the id).
            if i < 3 {
                assert!(!l.contains(&w.a.key.to_lowercase()), "key in {t}");
            }
        }
        // The stored copy keeps no path either (a folder name can carry the client's name).
        let stored: String = e
            .c
            .query_row("SELECT value FROM kv_meta WHERE key LIKE 'client_delete_receipt:%'", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains(&*e.dir.path().to_string_lossy()), "{stored}");
    }

    fn stored_receipts(e: &Env) -> Vec<String> {
        e.c.prepare("SELECT value FROM kv_meta WHERE key LIKE 'client_delete_receipt:%' ORDER BY key")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// X4 design 3.5 step 8: while the encryption upgrade's plaintext rollback copy `<db>.plain.bak` exists, a
    /// client delete removes it (and its sidecars) as part of the delete, and the receipt says so with a flag, never
    /// with client text or a path. The upgrade's finished record goes with it, so the next launch reads nothing as
    /// interrupted.
    #[test]
    fn delete_client_removes_plain_backup_and_receipt_says_so_without_client_text() {
        use crate::encrypt_migration::{self as mig, MigrationPaths, MigrationStatus};
        let e = env(Some("dev-a"));
        let w = world(&e);
        let paths = MigrationPaths::beside(&e.dir.path().join("m.db"));
        let bak = paths.plain_backup();
        // A plaintext copy that names the client, as the pre-encryption database would.
        std::fs::write(&bak, format!("SQLite format 3\0 {NAME} {ALIAS}")).unwrap();
        std::fs::write(format!("{}-wal", bak.display()), NAME).unwrap();
        std::fs::write(&paths.state_file, br#"{"version":2,"phase":"done","started_unix_ms":1,"updated_unix_ms":1,"failed_attempts":0}"#)
            .unwrap();
        assert_eq!(mig::status(&paths), MigrationStatus::Done);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert!(r.rollback_copy_removed);
        assert!(!bak.exists() && !std::path::Path::new(&format!("{}-wal", bak.display())).exists());
        assert_eq!(mig::status(&paths), MigrationStatus::None, "the finished record goes with the copy");
        let stored = stored_receipts(&e);
        assert_eq!(stored.len(), 1);
        let stored_json: serde_json::Value = serde_json::from_str(&stored[0]).unwrap();
        assert_eq!(stored_json["rollback_copy_removed"], true, "{}", stored[0]);
        for t in [serde_json::to_string(&r).unwrap(), format!("{r:?}"), stored[0].clone()] {
            let l = t.to_lowercase();
            for term in ["zorblax", "quuxco", "plain.bak"] {
                assert!(!l.contains(term), "{term} in {t}");
            }
        }
        // The stored copy keeps no path (the shown receipt lists the places the user must clear).
        assert!(!stored[0].contains(&*e.dir.path().to_string_lossy()), "{}", stored[0]);

        // No rollback copy: nothing to say.
        let e2 = env(Some("dev-a"));
        let w2 = world(&e2);
        let r2 = run(&e2, &select_all(&e2, &w2), &[w2.media.clone()]).unwrap();
        assert!(r2.complete() && !r2.rollback_copy_removed, "{r2:?}");
    }

    /// A rollback copy that would not go keeps the client (a re-run finishes it) and never reads as complete.
    #[cfg(unix)]
    #[test]
    fn a_plain_backup_that_will_not_go_keeps_the_client_for_a_rerun() {
        use std::os::unix::fs::PermissionsExt;
        let e = env(Some("dev-a"));
        let w = world(&e);
        let bak = crate::encrypt_migration::MigrationPaths::beside(&e.dir.path().join("m.db")).plain_backup();
        // A non-empty folder at the copy's name: removing a file there fails.
        std::fs::create_dir(&bak).unwrap();
        std::fs::write(bak.join("x"), b"x").unwrap();
        std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!r.complete() && !r.rollback_copy_removed, "{r:?}");
        assert!(r.failed_stages.contains(&DeleteStage::Files), "{r:?}");
        assert_eq!(r.client, ClientRecord::StillPresent);
        std::fs::remove_dir_all(&bak).unwrap();
        std::fs::write(&bak, NAME).unwrap();
        let again = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(again.complete() && again.rollback_copy_removed, "{again:?}");
        assert!(!bak.exists());
    }

    /// X4 design 9: X6's index clean-up (`optimize`, secure delete) still removes the client's terms from the
    /// stored full-text pages when the store is the encrypted one (the pages are read decrypted through SQL).
    #[cfg(feature = "sqlcipher")]
    #[test]
    fn delete_client_still_clears_fts_pages_with_optimize_on_encrypted_store() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let db = e.dir.path().join("m.db");
        assert_eq!(memory::db_file_state(&db).unwrap(), memory::DbFileState::NotPlain, "the encrypted store");
        assert!(e.store.secure_delete_enabled().unwrap(), "secure delete is on for the encrypted store");
        // A plain delete would leave the terms in their segments; they are there before.
        let terms = [NAME.to_lowercase(), ALIAS.to_lowercase()];
        assert!(terms.iter().map(|t| e.store.fts_pages_naming(t).unwrap()).sum::<i64>() > 0);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        for t in &terms {
            assert_eq!(e.store.fts_pages_naming(t).unwrap(), 0, "{t} still in the index pages");
        }
        assert!(e.store.secure_delete_enabled().unwrap(), "the delete restores secure delete on");
        assert_eq!(memory::db_file_state(&db).unwrap(), memory::DbFileState::NotPlain);
        // And the file on disk never holds the client's name in the clear.
        e.c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let raw = std::fs::read(&db).unwrap();
        for t in [NAME, ALIAS, "Globex"] {
            assert!(!raw.windows(t.len()).any(|win| win == t.as_bytes()), "{t} in clear on disk");
        }
    }

    #[test]
    fn seed_name_is_gone_after_delete() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        // A seed merged into A, with its own name, alias and member.
        let old = e.store.create_seed_topic("Zorblax Labs", &["ZLabs".to_string()], None).unwrap();
        e.c.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", params![w.a.key, old.key]).unwrap();
        // Names only the merged seed (not a possible match of A): found through the merge chain.
        let labs = email(&e, "g-labs", "Roadmap", "ZLabs roadmap", "t-labs");
        file(&e, old.topic_id, "email", labs, 1_000);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert_eq!(r.client, ClientRecord::Removed);
        for key in [&w.a.key, &old.key] {
            assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM topic_seed WHERE key = '{key}'")), 0);
        }
        assert_eq!(count(&e, "SELECT COUNT(*) FROM topic WHERE lower(name) LIKE '%zorblax%' OR lower(name) LIKE '%zlabs%'"), 0);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM topic_seed WHERE aliases LIKE '%ZLabs%' OR aliases LIKE '%Quuxco%'"), 0);
        assert_eq!(rows_naming_client(&e), Vec::<String>::new());
        assert!(matches!(crate::client_inventory::inventory(&e.store, &w.a.key), Err(ApiError::NotFound)));
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {labs}")), 0, "the merged seed's member goes too");
        // A purge record goes to other devices for both keys, nameless.
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'topic_seed_purged'"), 2);
    }

    #[test]
    fn delivered_payloads_go_and_their_relay_rows_are_recorded() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        // As if every op the world queued had been sent: the payloads (text) stay in the outbox.
        e.c.execute(
            "UPDATE sync_outbox SET state = 'delivered', delivered_at_unix_ms = 5, remote_receipt = 'sync_row:x'",
            [],
        )
        .unwrap();
        assert!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'profile_suggestion'") > 0);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(rows_naming_client(&e), Vec::<String>::new());
        assert_eq!(
            count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE row_type IN ('profile_suggestion', 'person', 'meeting', 'meeting_speaker')"),
            0
        );
        // Their relay rows are this device's to delete: recorded by key (suggestion, person) or by meeting.
        assert!(count(&e, "SELECT COUNT(*) FROM client_relay_cleanup WHERE target LIKE 'seq:profile_suggestion:%'") > 0);
        assert!(count(&e, "SELECT COUNT(*) FROM client_relay_cleanup WHERE target LIKE 'seq:person:%'") > 0);
        assert!(count(&e, &format!("SELECT COUNT(*) FROM pushed_meeting_row WHERE meeting_id = {}", w.m_a)) > 0);
    }

    #[test]
    fn an_unknown_or_merged_away_client_is_not_found_and_nothing_is_touched() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let unknown = DeleteSelection { key: "no-such-key".into(), ..Default::default() };
        assert!(matches!(run(&e, &unknown, &[w.media.clone()]), Err(ApiError::NotFound)));
        // A seed merged into A is not a client of its own any more.
        let old = e.store.create_seed_topic("Zorblax Labs", &[], None).unwrap();
        e.c.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", params![w.a.key, old.key]).unwrap();
        let merged = DeleteSelection { key: old.key.clone(), ..Default::default() };
        let ops = count(&e, "SELECT COUNT(*) FROM sync_outbox");
        let before = rows_naming_client(&e);
        assert!(matches!(run(&e, &merged, &[w.media.clone()]), Err(ApiError::NotFound)));
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox"), ops, "no purge row, no tombstone");
        assert_eq!(count(&e, "SELECT COUNT(*) FROM client_delete_pending"), 0);
        assert_eq!(rows_naming_client(&e), before, "no sweep");
        assert!(w.audio_in.exists());
    }

    #[test]
    fn the_receipt_counts_what_the_inventory_listed() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        // A seed merged into A, with its own member; and a calendar event of A's marked removed.
        let old = e.store.create_seed_topic("Zorblax Labs", &[], None).unwrap();
        e.c.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", params![w.a.key, old.key]).unwrap();
        let labs = email(&e, "g-labs", "Roadmap", "ZLabs roadmap", "t-labs");
        file(&e, old.topic_id, "email", labs, 1_000);
        let gone = calendar(&e, "gone-1#200", "Frobnitz retro");
        file(&e, w.a.topic_id, "calendar_event", gone, 3_000);
        e.c.execute("UPDATE items SET flags = '{\"removed\":true,\"changed_at\":1}' WHERE id = ?1", [gone]).unwrap();
        let inv = crate::client_inventory::inventory(&e.store, &w.a.key).unwrap();
        assert!(inv.linked.iter().any(|i| i.id == labs.to_string()), "the merged seed's member is listed");
        let listed = inv.linked.iter().find(|i| i.kind == ItemKind::CalendarEvent && i.id == gone.to_string());
        assert!(listed.is_some_and(|i| i.removed), "a removed event is listed, flagged");
        // The episode's capture is listed as a capture of its own.
        assert!(inv.linked.iter().any(|i| i.kind == ItemKind::Capture));
        assert!(!rows_naming(&e, &["frobnitz"]).is_empty());
        let sel = DeleteSelection { key: w.a.key.clone(), ..Default::default() };
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(r.counts, inv.counts, "the receipt counts exactly what the review screen listed");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {labs}")), 0);
        assert_eq!(rows_naming(&e, &["frobnitz"]), Vec::<String>::new(), "the removed event's title is gone");
        assert_eq!(
            count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'client_item_forgotten' AND aggregate_id LIKE 'calendar_event:%'"),
            2,
            "both events, removed or not, get a tombstone"
        );
    }

    #[test]
    fn an_episode_with_the_unknown_app_sentinel_goes_with_its_capture() {
        let e = env(Some("dev-a"));
        let a = e.store.create_seed_topic(NAME, &[], None).unwrap();
        // A capture with no app: its episode is stored with the "(unknown)" sentinel.
        let (cap, item) = capture(&e, 7_000, &format!("{NAME} notes"));
        e.c.execute("UPDATE items SET app_bundle_id = NULL WHERE id = ?1", [item]).unwrap();
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, \
             end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', '(unknown)', '(unknown)', 7000, 7000, ?1, ?1, 1, ?2, 'summarized', 1, 1)",
            params![item, format!("Read {NAME} notes")],
        )
        .unwrap();
        let ep = e.c.last_insert_rowid();
        let inv = crate::client_inventory::inventory(&e.store, &a.key).unwrap();
        let ticked: Vec<ClientItem> = inv.possible.iter().filter(|i| i.id == cap.to_string()).cloned().collect();
        assert_eq!(ticked.len(), 1);
        let sel = DeleteSelection { key: a.key.clone(), include_possible: ticked, ..Default::default() };
        let r = run(&e, &sel, &[]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {ep}")), 0, "matched by the sentinel");
        assert_eq!(rows_naming_client(&e), Vec::<String>::new());
    }

    /// Vault sync off: the delete engine itself leaves the vault alone even when a path comes along, and says so
    /// under Left for you (the API layer drops the path first; this guards the engine on its own).
    #[test]
    fn with_vault_sync_off_the_engine_never_touches_the_vault() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let vault = e.dir.path().join("vault");
        std::fs::create_dir_all(vault.join("Whimbrel/Meetings")).unwrap();
        let note = vault.join("Whimbrel/Meetings/Kickoff.md");
        std::fs::write(&note, format!("---\nwhimbrel_id: meeting-{}\n---\n# Kickoff\n", w.m_a)).unwrap();
        let mut sel = select_all(&e, &w);
        sel.vault = Some(vault.clone());
        sel.vault_off = true;
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.m_a)), 0, "the meeting goes");
        assert!(note.exists(), "its vault note stays");
        assert!(r.left_for_you.contains(&LeftForYou::VaultSyncOff), "{r:?}");
        assert!(!r.left_for_you.contains(&LeftForYou::VaultFolder(vault)), "{r:?}");
        assert!(!r.counts.iter().any(|(k, _)| *k == ItemKind::VaultNote), "{r:?}");
    }

    #[test]
    fn a_crash_after_the_meeting_rows_commit_is_finished_by_the_rerun() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let vault = e.dir.path().join("vault");
        std::fs::create_dir_all(vault.join("Whimbrel/Meetings")).unwrap();
        let note = vault.join("Whimbrel/Meetings/Kickoff.md");
        std::fs::write(&note, format!("---\nwhimbrel_id: meeting-{}\n---\n# Kickoff\n", w.m_a)).unwrap();
        // The app stops right after the meeting's rows commit: its files and note are still there.
        let run_id = memory::Store::client_delete_run_id(&w.a.key);
        assert!(delete_meeting(&e.store, &run_id, w.m_a).unwrap().is_some());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.m_a)), 0);
        assert!(w.audio_in.exists() && note.exists());
        // The meeting is no longer in the inventory; the pending record still knows its files and note.
        let mut sel = select_all(&e, &w);
        sel.vault = Some(vault);
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert!(!w.audio_in.exists() && !w.media.join("mic-audio-1.m4a").exists());
        assert!(!note.exists());
        assert_eq!(count(&e, "SELECT COUNT(*) FROM client_delete_pending"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_vault_failure_keeps_the_client_and_the_rerun_removes_the_notes() {
        use std::os::unix::fs::PermissionsExt;
        let e = env(Some("dev-a"));
        let w = world(&e);
        let vault = e.dir.path().join("vault");
        let meetings = vault.join("Whimbrel/Meetings");
        std::fs::create_dir_all(&meetings).unwrap();
        let note = meetings.join("Kickoff.md");
        std::fs::write(&note, format!("---\nwhimbrel_id: meeting-{}\n---\n# Kickoff\n", w.m_a)).unwrap();
        std::fs::set_permissions(&meetings, std::fs::Permissions::from_mode(0o555)).unwrap();
        let mut sel = select_all(&e, &w);
        sel.vault = Some(vault);
        let first = run(&e, &sel, &[w.media.clone()]).unwrap();
        std::fs::set_permissions(&meetings, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(first.failed, vec![ItemKind::VaultNote], "{first:?}");
        assert_eq!(first.client, ClientRecord::StillPresent);
        assert!(note.exists());
        let second = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(second.complete(), "{second:?}");
        assert!(!note.exists(), "the deleted meeting's note goes on the re-run");
        assert_eq!(count(&e, "SELECT COUNT(*) FROM client_delete_pending"), 0);
    }

    #[test]
    fn paths_another_device_sent_or_outside_the_roots_are_never_deleted() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        // A meeting pulled from a peer naming a file inside the media folder, and an own meeting whose
        // transcript path climbs out of it.
        let peer_audio = w.media.join("system-audio-9.m4a");
        std::fs::write(&peer_audio, b"x").unwrap();
        let pulled = meeting(&e, &format!("{NAME} peer call"), 40_000, Some(peer_audio.to_string_lossy().into()));
        e.c.execute("UPDATE meeting SET device_id = 'dev-b' WHERE id = ?1", [pulled]).unwrap();
        e.c.execute(
            "INSERT INTO sync_meeting_map (peer_device_id, source_meeting_id, local_meeting_id) VALUES ('dev-b', 3, ?1)",
            [pulled],
        )
        .unwrap();
        file(&e, w.a.topic_id, "meeting", pulled, 40_000);
        let escape = e.dir.path().join("elsewhere").join("transcript-7.txt");
        std::fs::write(&escape, b"x").unwrap();
        let climbing = w.media.join("..").join("elsewhere").join("transcript-7.txt");
        e.c.execute(
            "UPDATE meeting SET transcript_path = ?1 WHERE id = ?2",
            params![climbing.to_string_lossy(), w.m_a],
        )
        .unwrap();
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(peer_audio.exists(), "a path another device sent is never deleted");
        assert!(escape.exists(), "a path resolving outside the media folder is never deleted");
        let left: Vec<PathBuf> = r
            .left_for_you
            .iter()
            .filter_map(|l| match l {
                LeftForYou::FilesNotRemoved(p) => Some(p.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(left.contains(&peer_audio) && left.contains(&climbing), "{left:?}");
    }

    #[test]
    fn the_client_delete_removes_a_kept_slide_folder_with_our_marker() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let slides = w.media.join("slides-a");
        std::fs::create_dir_all(&slides).unwrap();
        std::fs::write(slides.join(crate::retention::SNAPSHOT_MARKER), b"").unwrap();
        std::fs::write(slides.join("10.00.00.png"), b"x").unwrap();
        e.c.execute(
            "UPDATE meeting SET snapshots_dir = ?1, keep_snapshots = 1 WHERE id = ?2",
            params![slides.to_string_lossy(), w.m_a],
        )
        .unwrap();
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        assert!(!slides.exists(), "a client delete overrides keep-slides");
    }

    #[test]
    fn an_episodes_captures_are_finished_by_the_rerun_after_the_episode_went() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let ep_capture: i64 = count(
            &e,
            &format!(
                "SELECT i.id FROM items i JOIN episodes ep ON i.id BETWEEN ep.start_item_id AND ep.end_item_id \
                 WHERE ep.id = {} AND i.kind = 'capture'",
                w.episode
            ),
        );
        // Only what is linked: the episode's capture is not ticked as a possible match.
        let sel = DeleteSelection { key: w.a.key.clone(), ..Default::default() };
        e.c.execute_batch(
            "CREATE TRIGGER fail_capture BEFORE DELETE ON items WHEN old.kind = 'capture' \
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();
        let first = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {}", w.episode)), 0);
        assert!(first.failed.contains(&ItemKind::Capture), "{first:?}");
        assert_eq!(first.client, ClientRecord::StillPresent);
        e.c.execute_batch("DROP TRIGGER fail_capture;").unwrap();
        // The episode is gone, so the inventory no longer lists its capture; the pending record does.
        let inv = crate::client_inventory::inventory(&e.store, &w.a.key).unwrap();
        assert!(!inv.linked.iter().any(|i| i.kind == ItemKind::Capture));
        let second = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {ep_capture}")), 0);
        assert_eq!(second.client, ClientRecord::Removed, "{second:?}");
        assert!(second.complete());
        assert_eq!(second.counts, vec![(ItemKind::Capture, 1)]);
    }

    #[cfg(unix)]
    #[test]
    fn an_audio_file_in_the_media_folder_that_would_not_go_keeps_the_client() {
        use std::os::unix::fs::PermissionsExt;
        let e = env(Some("dev-a"));
        let w = world(&e);
        std::fs::set_permissions(&w.media, std::fs::Permissions::from_mode(0o555)).unwrap();
        let first = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        std::fs::set_permissions(&w.media, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(w.audio_in.exists());
        assert!(first.failed_stages.contains(&DeleteStage::Files), "{first:?}");
        assert_eq!(first.client, ClientRecord::StillPresent);
        let second = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(second.complete(), "{second:?}");
        assert!(!w.audio_in.exists() && !w.media.join("mic-audio-1.m4a").exists());
    }

    #[test]
    fn a_marked_slide_folder_outside_the_media_folders_is_left() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let slides = e.dir.path().join("elsewhere").join("slides-a");
        std::fs::create_dir_all(&slides).unwrap();
        std::fs::write(slides.join(crate::retention::SNAPSHOT_MARKER), b"").unwrap();
        std::fs::write(slides.join("10.00.00.png"), b"x").unwrap();
        e.c.execute(
            "UPDATE meeting SET snapshots_dir = ?1 WHERE id = ?2",
            params![slides.to_string_lossy(), w.m_a],
        )
        .unwrap();
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(slides.join("10.00.00.png").exists(), "a slide folder outside the media folders is never removed");
        assert!(r.left_for_you.iter().any(|l| matches!(l, LeftForYou::FilesNotRemoved(p) if p.contains(&slides))), "{r:?}");
    }

    #[test]
    fn purging_the_client_clears_its_pending_record() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let run_id = memory::Store::client_delete_run_id(&w.a.key);
        e.store.add_client_delete_pending(&run_id, memory::client_delete::PENDING_TOPIC, &["1".into()]).unwrap();
        e.store.forget_seed_topic(&w.a.key).unwrap();
        assert!(e.store.client_delete_pending(&run_id).unwrap().is_empty());
    }

    /// `SELECT RAISE` in a trigger makes one kind's delete fail, as a locked or damaged table would.
    fn break_deletes_of(e: &Env, table: &str) {
        e.c.execute_batch(&format!(
            "CREATE TRIGGER fail_{table} BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT, 'injected'); END;"
        ))
        .unwrap();
    }

    #[test]
    fn rerun_after_partial_failure_completes() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let sel = select_all(&e, &w);
        break_deletes_of(&e, "conversation");
        let first = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert_eq!(first.failed, vec![ItemKind::Conversation]);
        assert_eq!(first.client, ClientRecord::StillPresent);
        assert!(!first.complete());
        // The client stays, partly present, so the review screen and a re-run still find it.
        assert!(crate::client_inventory::inventory(&e.store, &w.a.key).is_ok());
        e.c.execute_batch("DROP TRIGGER fail_conversation;").unwrap();
        let second = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(second.failed.is_empty() && second.failed_stages.is_empty(), "{second:?}");
        assert_eq!(second.client, ClientRecord::Removed);
        assert_eq!(second.counts, vec![(ItemKind::Conversation, 1)], "only what was left");
        assert_eq!(rows_naming_client(&e), Vec::<String>::new());
    }

    #[test]
    fn files_outside_media_roots_are_not_deleted() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(!w.audio_in.exists());
        assert!(w.audio_out.exists(), "a file outside the media folder is never removed");
        let left: Vec<&PathBuf> = r
            .left_for_you
            .iter()
            .filter_map(|l| match l {
                LeftForYou::FilesNotRemoved(p) => Some(p),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(left, vec![&w.audio_out], "and the receipt says it is left");
    }

    #[test]
    fn reports_failure_never_silent_success() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        break_deletes_of(&e, "episodes");
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert!(r.failed.contains(&ItemKind::Episode), "{r:?}");
        assert!(!r.counts.iter().any(|(k, _)| *k == ItemKind::Episode), "a failed kind is never counted");
        assert_eq!(r.client, ClientRecord::StillPresent);
        assert!(!r.complete());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {}", w.episode)), 1);
        let stored: String = e
            .c
            .query_row("SELECT value FROM kv_meta WHERE key LIKE 'client_delete_receipt:%'", [], |r| r.get(0))
            .unwrap();
        assert!(stored.contains("episode") && stored.contains("still_present"), "{stored}");
    }

    #[test]
    fn without_a_sync_id_nothing_is_reported_as_sent() {
        let e = env(None);
        let w = world(&e);
        let r = delete_client(&e.store, &select_all(&e, &w), &[w.media.clone()], None).unwrap();
        assert!(r.failed.is_empty(), "{r:?}");
        assert_eq!(r.client, ClientRecord::RemovedHereOnly);
        // Never a silent success: the receipt says the files were kept for want of a device id.
        assert_eq!(r.failed_stages, vec![DeleteStage::NoDeviceId]);
        assert!(!r.complete());
        assert_eq!(r.pending_devices, None, "unknown, never 0");
        for (kind, n) in &r.counts {
            let held = r.not_sent.iter().chain(&r.local_only).filter(|(k, _)| k == kind).map(|x| x.1).sum::<usize>();
            assert_eq!(held, *n, "{kind:?}: every item is reported not sent or local only");
        }
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE operation_kind = 'tombstone'"), 0);
        // With no sync id the forget can't tell this device's audio from a peer's, so it hands none back:
        // the file stays, and the receipt says so.
        let left: Vec<PathBuf> = r
            .left_for_you
            .iter()
            .filter_map(|l| match l {
                LeftForYou::FilesNotRemoved(p) => Some(p.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(w.audio_in.exists(), "fail closed: no file is removed without a device id");
        assert!(left.contains(&w.audio_in), "{left:?}");
    }

    #[test]
    fn local_only_and_pending_devices_are_reported() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        e.store.set_pending_tombstone_device_ids(&["old-1".into(), "old-2".into()]).unwrap();
        let r = run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        assert_eq!(r.pending_devices, Some(2));
        assert!(r.local_only.contains(&(ItemKind::Conversation, 1)), "{r:?}");
        assert!(!r.not_sent.iter().any(|(k, _)| *k == ItemKind::Email), "{r:?}");
    }

    #[test]
    fn a_deleted_event_or_email_is_not_brought_back_by_ingestion() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        run(&e, &select_all(&e, &w), &[w.media.clone()]).unwrap();
        calendar_again(&e);
        email(&e, "g-new", "Hi", "new mail", "t-new");
        e.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-a".into(),
                external_id: "g-a".into(),
                ts_unix_ms: 1_000,
                from: "a@x.com".into(),
                to: "me@x.com".into(),
                subject: "again".into(),
                body_text: "again".into(),
                gmail_thread_id: "t-a".into(),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        assert_eq!(count(&e, "SELECT COUNT(*) FROM items WHERE external_id IN ('g-a', 'kickoff-1@zorblax.com#100')"), 0);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM items WHERE external_id = 'g-new'"), 1);
    }

    fn calendar_again(e: &Env) {
        e.store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".into(),
                source: "google_calendar".into(),
                external_id: "kickoff-1@zorblax.com#100".into(),
                ts_unix_ms: 2_000,
                title: "again".into(),
                actors: vec![],
            }])
            .unwrap();
    }

    #[test]
    fn vault_notes_of_the_client_go_and_others_stay() {
        let e = env(Some("dev-a"));
        let w = world(&e);
        let vault = e.dir.path().join("vault");
        let wh = vault.join("Whimbrel");
        for d in ["Topics", "Organizations", "Meetings", "Archive"] {
            std::fs::create_dir_all(wh.join(d)).unwrap();
        }
        let note = |dir: &str, name: &str, id: &str| {
            let p = wh.join(dir).join(format!("{name}.md"));
            std::fs::write(&p, format!("---\nwhimbrel_id: {id}\n---\n# {name}\n")).unwrap();
            p
        };
        let ours = [
            note("Topics", NAME, &format!("topic-{}", w.a.topic_id)),
            note("Organizations", "Zorblax Inc", &format!("org-{}", w.org)),
            note("Meetings", "Kickoff", &format!("meeting-{}", w.m_a)),
            note("Archive", "Zorblax old", &format!("topic-{}", w.a.topic_id)),
        ];
        let theirs = [note("Topics", "Globex", &format!("topic-{}", w.b.topic_id)), note("Meetings", "Review", &format!("meeting-{}", w.m_b))];
        let foreign = wh.join("Topics").join("mine.md");
        std::fs::write(&foreign, "my own note about Zorblax\n").unwrap();
        let mut sel = select_all(&e, &w);
        sel.vault = Some(vault.clone());
        let r = run(&e, &sel, &[w.media.clone()]).unwrap();
        assert!(r.complete(), "{r:?}");
        for p in &ours {
            assert!(!p.exists(), "{p:?}");
        }
        for p in &theirs {
            assert!(p.exists(), "{p:?}");
        }
        assert!(foreign.exists(), "a file without our frontmatter is the user's");
        assert_eq!(r.counts.iter().find(|(k, _)| *k == ItemKind::VaultNote).map(|x| x.1), Some(4));
        assert!(r.left_for_you.contains(&LeftForYou::VaultFolder(vault)));
    }
}
