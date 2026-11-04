//! Live question alerts other devices heard (parity item a). Sync applies a peer's `live_alert`
//! rows; after each round this takes the new ones and queues popup notices: at most
//! [`PEER_CEILING`] per (origin device, meeting), then one "N more" notice updated in place,
//! as the Mac's `LiveAlertDelivery.afterPull` does. The popup is `question_alert.rs`; the
//! meeting page's "Questions for you" uses [`alert_view`].
//! See docs/superpowers/plans/2026-11-03-windows-peer-live-alerts.md.

use std::collections::HashMap;
use std::sync::Mutex;

use memory::live_alert::LiveAlert;

use crate::l10n::tr;

/// Peer alerts older than this when pulled only appear on the meeting page.
pub const PEER_MAX_AGE_MS: i64 = 30 * 60 * 1000;
/// Notices per (origin device, meeting) before the "N more" one: the Mac's `LiveQuestionWatcher.maxAlerts`.
pub const PEER_CEILING: usize = 6;

/// One popup notice.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuestionNotice {
    /// `live-alert-<origin>-<source id>`, or `live-alert-more-<origin>-<source meeting>`; a
    /// notice with the same id replaces the queued one.
    pub id: String,
    pub title: String,
    /// The meeting's title: another device's alert isn't about the meeting on screen here.
    pub subtitle: Option<String>,
    pub body: String,
    /// The device that heard it and its own id for the meeting, resolved to this PC's id
    /// when the notice is opened.
    pub origin: String,
    pub source_meeting_id: i64,
    /// This PC's id for the meeting when it had synced at the time of the pull.
    pub meeting_id: Option<i64>,
}

/// The parts of a stored draft (`llm::ChatAnswer` as JSON) the alerts use. Citations stay
/// loose JSON: a peer's newer kind must not make the whole draft unreadable here.
#[derive(serde::Deserialize)]
struct Draft {
    text: String,
    #[serde(default)]
    citations: Vec<serde_json::Value>,
}

/// One cited source under a draft this PC made: its title and, when known, its date.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub label: String,
    pub ts_unix_ms: Option<i64>,
}

/// Each cited item once, as its title and date: the Mac's `LiveAlertDelivery.evidence`. Items
/// gone from the store are skipped. Two copies with the same title in the same minute show
/// once (the Mac dedupes on the "title · date" it prints, to the minute).
pub fn evidence(store: &memory::Store, citations: &[serde_json::Value]) -> Vec<Evidence> {
    use llm::ContextItemRef as R;
    let mut out: Vec<Evidence> = Vec::new();
    for item in citations.iter().filter_map(|c| serde_json::from_value::<R>(c.clone()).ok()) {
        let entry = match item {
            R::Capture { id } => store.get_capture_item(id).ok().flatten().map(|c| Evidence {
                label: if c.window_title.trim().is_empty() { c.app_bundle_id } else { c.window_title },
                ts_unix_ms: Some(c.ts_unix_ms),
            }),
            R::Thread { id } => store.get_thread(id).ok().flatten().map(|t| Evidence {
                label: t.subject.filter(|s| !s.trim().is_empty()).unwrap_or(t.last_message_from),
                ts_unix_ms: Some(t.last_activity_ts_unix_ms),
            }),
            R::Episode { id } => store.get_episode(id).ok().flatten().map(|e| Evidence {
                label: e.window_title.filter(|t| !t.trim().is_empty()).unwrap_or(e.app_bundle_id),
                ts_unix_ms: Some(e.start_ts_unix_ms),
            }),
            R::Transcript { id } => store.get_transcript_segment(id).ok().flatten().map(|s| Evidence {
                ts_unix_ms: store.get_meeting(s.meeting_id).ok().flatten().map(|m| m.start_unix_ms),
                label: s.meeting_title,
            }),
            R::Snapshot { .. } => Some(Evidence { label: tr("A meeting slide", &[]), ts_unix_ms: None }),
            R::RoutineRun { .. } => Some(Evidence { label: tr("A routine result", &[]), ts_unix_ms: None }),
        };
        let Some(entry) = entry else { continue };
        let minute = |e: &Evidence| e.ts_unix_ms.map(|ms| ms.div_euclid(60_000));
        if !out.iter().any(|e| e.label == entry.label && minute(e) == minute(&entry)) {
            out.push(entry);
        }
    }
    out
}

fn draft(json: Option<&str>) -> Option<Draft> {
    json.and_then(|j| serde_json::from_str(j).ok())
}

fn asker(alert: &LiveAlert) -> Option<&str> {
    alert.asker.as_deref().map(str::trim).filter(|a| !a.is_empty())
}

/// The notice's title and body: who asked, then the restatement and the draft's first line.
pub fn content(alert: &LiveAlert) -> (String, String) {
    let title = match asker(alert) {
        Some(name) => tr("{asker} asked you", &[("asker", name)]),
        None => tr("Question for you", &[]),
    };
    let first_line = draft(alert.draft_json.as_deref()).and_then(|d| d.text.lines().find(|l| !l.trim().is_empty()).map(str::to_string));
    let body = std::iter::once(alert.restatement.clone()).chain(first_line).collect::<Vec<_>>().join("\n");
    (title, body)
}

/// How many notices each peer meeting has had. In memory, as on the Mac, so a relaunch
/// restarts the count.
#[derive(Debug, Default)]
pub struct PeerNotices {
    shown: HashMap<(String, i64), usize>,
    extra: HashMap<(String, i64), usize>,
}

impl PeerNotices {
    /// The notices for alerts just taken (oldest first). Rows this PC made are skipped.
    pub fn plan(&mut self, alerts: &[LiveAlert]) -> Vec<QuestionNotice> {
        let mut out = Vec::new();
        let mut summaries: Vec<((String, i64), &LiveAlert)> = Vec::new();
        for alert in alerts {
            let Some(origin) = alert.origin_device_id.clone() else { continue };
            let key = (origin.clone(), alert.source_meeting_id);
            let shown = self.shown.entry(key.clone()).or_default();
            if *shown < PEER_CEILING {
                *shown += 1;
                let (title, body) = content(alert);
                out.push(QuestionNotice {
                    id: format!("live-alert-{origin}-{}", alert.source_id),
                    title,
                    subtitle: Some(alert.meeting_title.clone()),
                    body,
                    origin,
                    source_meeting_id: alert.source_meeting_id,
                    meeting_id: alert.meeting_id,
                });
            } else {
                *self.extra.entry(key.clone()).or_default() += 1;
                match summaries.iter_mut().find(|(k, _)| *k == key) {
                    Some(entry) => entry.1 = alert,
                    None => summaries.push((key, alert)),
                }
            }
        }
        summaries.sort_by(|a, b| a.0.cmp(&b.0));
        for ((origin, source_meeting_id), alert) in summaries {
            let extra = self.extra[&(origin.clone(), source_meeting_id)];
            out.push(more_notice(&origin, source_meeting_id, alert.meeting_id, &alert.meeting_title, extra));
        }
        out
    }
}

/// The one "N more questions for you" notice per meeting, updated in place.
pub fn more_notice(origin: &str, source_meeting_id: i64, meeting_id: Option<i64>, meeting_title: &str, extra: usize) -> QuestionNotice {
    QuestionNotice {
        id: format!("live-alert-more-{origin}-{source_meeting_id}"),
        title: meeting_title.to_string(),
        subtitle: None,
        body: tr("{count} more questions for you", &[("count", &extra.to_string())]),
        origin: origin.to_string(),
        source_meeting_id,
        meeting_id,
    }
}

/// The notice for an alert this PC found (parity item b). Its origin is this PC's device id,
/// so its id matches the one a peer would build, and opening it falls back to `meeting_id`
/// (a meeting is never in its own device's sync map). No subtitle: it's the meeting being
/// recorded here.
pub fn local_notice(alert: &LiveAlert, device_id: &str) -> QuestionNotice {
    let (title, body) = content(alert);
    QuestionNotice {
        id: format!("live-alert-{device_id}-{}", alert.source_id),
        title,
        subtitle: None,
        body,
        origin: device_id.to_string(),
        source_meeting_id: alert.source_meeting_id,
        meeting_id: alert.meeting_id,
    }
}

/// The popup's queue, shared by the sync thread (which fills it), the watcher (which opens
/// the popup while it isn't empty) and the popup's commands.
#[derive(Debug, Default)]
pub struct QuestionAlerts {
    queue: Mutex<Vec<QuestionNotice>>,
    planner: Mutex<PeerNotices>,
    /// A meeting the main window should open when it next asks: `Some(None)` opens Meetings
    /// with nothing selected (the peer's meeting hasn't synced here yet).
    pending_open: Mutex<Option<Option<i64>>>,
}

impl QuestionAlerts {
    /// After a sync round: takes the new peer alerts (marking them notified even when alerts
    /// are off here, so they never fire later) and queues their notices. Returns how many.
    pub fn after_round(&self, store: &memory::Store, enabled: bool, now_ms: i64) -> usize {
        let alerts = match store.take_unnotified_peer_alerts(now_ms, PEER_MAX_AGE_MS) {
            Ok(alerts) => alerts,
            Err(e) => {
                tracing::warn!(error = %e, "taking peer question alerts failed");
                return 0;
            }
        };
        if !enabled || alerts.is_empty() {
            return 0;
        }
        let notices = self.planner.lock().unwrap().plan(&alerts);
        let count = notices.len();
        self.push(notices);
        tracing::info!(count, "queued question alerts from other devices");
        count
    }

    /// Newest first; a notice replaces a queued one with the same id.
    pub fn push(&self, notices: Vec<QuestionNotice>) {
        let mut queue = self.queue.lock().unwrap();
        for notice in notices {
            queue.retain(|n| n.id != notice.id);
            queue.insert(0, notice);
        }
    }

    /// Updates a queued notice in place (a draft arrived) and reports whether it was there. A
    /// notice already opened or dismissed stays gone: a draft doesn't pop it up again.
    pub fn replace(&self, notice: QuestionNotice) -> bool {
        let mut queue = self.queue.lock().unwrap();
        match queue.iter_mut().find(|n| n.id == notice.id) {
            Some(queued) => {
                *queued = notice;
                true
            }
            None => false,
        }
    }

    pub fn list(&self) -> Vec<QuestionNotice> {
        self.queue.lock().unwrap().clone()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    pub fn remove(&self, id: &str) -> Option<QuestionNotice> {
        let mut queue = self.queue.lock().unwrap();
        let at = queue.iter().position(|n| n.id == id)?;
        Some(queue.remove(at))
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
    }

    pub fn set_pending_open(&self, meeting_id: Option<i64>) {
        *self.pending_open.lock().unwrap() = Some(meeting_id);
    }

    pub fn take_pending_open(&self) -> Option<Option<i64>> {
        self.pending_open.lock().unwrap().take()
    }
}

/// This PC's id for the notice's meeting, resolved now through the sync meeting map (it may
/// have synced since the pull), else the id known at the pull, else `None`.
pub fn local_meeting(store: &memory::Store, notice: &QuestionNotice) -> Option<i64> {
    store
        .resolve_sync_meeting_map(&notice.origin, notice.source_meeting_id)
        .ok()
        .flatten()
        .filter(|id| store.get_meeting(*id).ok().flatten().is_some())
        .or(notice.meeting_id)
}

/// One "Questions for you" card.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LiveAlertView {
    pub id: i64,
    pub restatement: String,
    pub quote: String,
    pub asker: Option<String>,
    /// The draft answer's markdown; model output, rendered without links or images.
    pub draft: Option<String>,
    /// The draft cites sources this PC can't resolve: another device recorded the meeting.
    pub sources_elsewhere: bool,
    /// What the draft cites, for an alert this PC made (empty for a peer's).
    pub evidence: Vec<Evidence>,
    /// When it was asked, "m:ss" from the meeting's start.
    pub asked_at: String,
    pub created_at_unix_ms: i64,
}

/// "m:ss" for centiseconds from the meeting's start.
pub fn clock(cs: i64) -> String {
    let seconds = cs.max(0) / 100;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// One card. A peer's citations are its own ids, so only this PC's alerts resolve theirs.
pub fn alert_view(store: &memory::Store, alert: &LiveAlert) -> LiveAlertView {
    let draft = draft(alert.draft_json.as_deref());
    let peer = alert.origin_device_id.is_some();
    LiveAlertView {
        id: alert.id,
        restatement: alert.restatement.clone(),
        quote: alert.quote.clone(),
        asker: asker(alert).map(str::to_string),
        sources_elsewhere: peer && draft.as_ref().is_some_and(|d| !d.citations.is_empty()),
        evidence: match (&draft, peer) {
            (Some(d), false) => evidence(store, &d.citations),
            _ => Vec::new(),
        },
        draft: draft.map(|d| d.text).filter(|t| !t.trim().is_empty()),
        asked_at: clock(alert.at_cs),
        created_at_unix_ms: alert.created_at_unix_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::live_alert::AppliedLiveAlert;

    fn alert(origin: Option<&str>, source_id: i64, source_meeting_id: i64) -> LiveAlert {
        LiveAlert {
            id: source_id,
            meeting_id: None,
            origin_device_id: origin.map(str::to_string),
            source_id,
            source_meeting_id,
            meeting_title: "Pricing review".into(),
            quote: "Mike, can you send the numbers?".into(),
            restatement: "Send the Q3 numbers".into(),
            asker: Some("Dana".into()),
            at_cs: 12_345,
            created_at_unix_ms: 1_000,
            draft_json: None,
            email_status: None,
            notified_at_unix_ms: None,
        }
    }

    #[test]
    fn six_notices_per_meeting_then_one_summary_that_keeps_counting() {
        let mut planner = PeerNotices::default();
        let first: Vec<_> = (1..=8).map(|i| alert(Some("mac"), i, 7)).collect();
        let notices = planner.plan(&first);
        assert_eq!(notices.len(), 7);
        assert!(notices[..6].iter().all(|n| n.id.starts_with("live-alert-mac-")));
        assert_eq!(notices[6].id, "live-alert-more-mac-7");
        assert_eq!(notices[6].body, "2 more questions for you");
        let later = planner.plan(&[alert(Some("mac"), 9, 7)]);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].body, "3 more questions for you", "the summary is updated in place");
    }

    #[test]
    fn each_meeting_and_device_has_its_own_count() {
        let mut planner = PeerNotices::default();
        let mut alerts: Vec<_> = (1..=6).map(|i| alert(Some("mac"), i, 7)).collect();
        alerts.push(alert(Some("mac"), 20, 8));
        alerts.push(alert(Some("mini"), 30, 7));
        let notices = planner.plan(&alerts);
        assert_eq!(notices.len(), 8);
        assert!(notices.iter().all(|n| !n.id.starts_with("live-alert-more-")));
    }

    #[test]
    fn this_pcs_own_alerts_are_never_notified() {
        assert!(PeerNotices::default().plan(&[alert(None, 1, 7)]).is_empty());
    }

    #[test]
    fn the_title_names_the_asker_and_the_body_adds_the_drafts_first_line() {
        let mut a = alert(Some("mac"), 1, 7);
        assert_eq!(content(&a), ("Dana asked you".to_string(), "Send the Q3 numbers".to_string()));
        a.asker = Some("  ".into());
        a.draft_json = Some(r#"{"text":"\nThey're in the deck.\nMore detail","citations":[]}"#.into());
        assert_eq!(content(&a), ("Question for you".to_string(), "Send the Q3 numbers\nThey're in the deck.".to_string()));
        a.draft_json = Some("not json".into());
        assert_eq!(content(&a).1, "Send the Q3 numbers", "a bad draft is ignored");
    }

    #[test]
    fn a_notice_with_the_same_id_replaces_the_queued_one_and_goes_first() {
        let queue = QuestionAlerts::default();
        let mut planner = PeerNotices::default();
        queue.push(planner.plan(&[alert(Some("mac"), 1, 7), alert(Some("mac"), 2, 7)]));
        assert_eq!(queue.list().iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["live-alert-mac-2", "live-alert-mac-1"]);
        let mut again = queue.list()[1].clone();
        again.body = "drafted".into();
        queue.push(vec![again]);
        assert_eq!(queue.list().len(), 2);
        assert_eq!(queue.list()[0].body, "drafted");
        assert!(queue.remove("live-alert-mac-1").is_some());
        assert!(queue.remove("live-alert-mac-1").is_none());
    }

    fn applied(source_id: i64, created_at_unix_ms: i64) -> AppliedLiveAlert {
        AppliedLiveAlert {
            source_id,
            source_meeting_id: 42,
            meeting_title: "Pricing review".into(),
            quote: "Mike, can you send the numbers?".into(),
            restatement: "Send the Q3 numbers".into(),
            asker: Some("Dana".into()),
            at_cs: 6_000,
            created_at_unix_ms,
            draft_json: None,
        }
    }

    #[test]
    fn after_a_round_fresh_peer_alerts_are_queued_and_old_ones_only_marked() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = 100 * 60_000;
        store.apply_synced_live_alert("mac", &applied(1, now - 60_000)).unwrap();
        store.apply_synced_live_alert("mac", &applied(2, now - PEER_MAX_AGE_MS - 1)).unwrap();
        let alerts = QuestionAlerts::default();
        assert_eq!(alerts.after_round(&store, true, now), 1);
        assert_eq!(alerts.list()[0].id, "live-alert-mac-1");
        assert_eq!(alerts.after_round(&store, true, now), 0, "both were marked notified");
    }

    #[test]
    fn with_alerts_off_nothing_is_queued_but_alerts_are_still_marked() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = 100 * 60_000;
        store.apply_synced_live_alert("mac", &applied(1, now)).unwrap();
        let alerts = QuestionAlerts::default();
        assert_eq!(alerts.after_round(&store, false, now), 0);
        assert!(alerts.is_empty());
        assert_eq!(alerts.after_round(&store, true, now), 0, "it won't fire later");
    }

    #[test]
    fn opening_resolves_the_peer_meeting_once_it_has_synced() {
        let store = memory::Store::open_in_memory().unwrap();
        store.apply_synced_live_alert("mac", &applied(1, 1_000)).unwrap();
        let notice = PeerNotices::default().plan(&store.take_unnotified_peer_alerts(1_000, PEER_MAX_AGE_MS).unwrap()).remove(0);
        assert_eq!(local_meeting(&store, &notice), None, "not synced here yet");
        let local = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "mac".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Pricing review".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        store.record_sync_meeting_map("mac", 42, local).unwrap();
        assert_eq!(local_meeting(&store, &notice), Some(local));
    }

    #[test]
    fn a_card_shows_the_draft_and_says_when_its_sources_are_elsewhere() {
        let store = memory::Store::open_in_memory().unwrap();
        let mut a = alert(Some("mac"), 1, 7);
        let view = alert_view(&store, &a);
        assert_eq!((view.draft.as_deref(), view.sources_elsewhere, view.asked_at.as_str()), (None, false, "2:03"));
        a.draft_json = Some(r#"{"text":"They're in the deck.","citations":[{"kind":"snapshot","id":3}]}"#.into());
        let view = alert_view(&store, &a);
        assert_eq!(view.draft.as_deref(), Some("They're in the deck."));
        assert!(view.sources_elsewhere);
        assert!(view.evidence.is_empty(), "a peer's ids mean nothing here");
        a.origin_device_id = None;
        let view = alert_view(&store, &a);
        assert!(!view.sources_elsewhere, "this PC's own citations resolve here");
        assert_eq!(view.evidence, [Evidence { label: "A meeting slide".into(), ts_unix_ms: None }]);
    }

    #[test]
    fn evidence_skips_what_is_gone_and_shows_each_source_once() {
        let store = memory::Store::open_in_memory().unwrap();
        let cited: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"kind":"capture","id":99},{"kind":"snapshot","id":1},{"kind":"routine_run","id":2},{"kind":"snapshot","id":4},{"kind":"someday","id":5}]"#,
        )
        .unwrap();
        let labels: Vec<String> = evidence(&store, &cited).into_iter().map(|e| e.label).collect();
        assert_eq!(labels, ["A meeting slide", "A routine result"], "a missing capture and an unknown kind are skipped");
    }

    #[test]
    fn a_local_notice_keeps_its_id_when_the_draft_arrives_and_opens_its_own_meeting() {
        let store = memory::Store::open_in_memory().unwrap();
        let mut a = alert(None, 5, 9);
        a.meeting_id = Some(9);
        let notice = local_notice(&a, "pc");
        assert_eq!((notice.id.as_str(), notice.subtitle.as_deref(), notice.title.as_str()), ("live-alert-pc-5", None, "Dana asked you"));
        assert_eq!(local_meeting(&store, &notice), Some(9), "never in its own sync map; the id it carries");

        let queue = QuestionAlerts::default();
        queue.push(vec![notice.clone(), more_notice("mac", 1, None, "Other", 2)]);
        a.draft_json = Some(r#"{"text":"Friday works.","citations":[]}"#.into());
        assert!(queue.replace(local_notice(&a, "pc")));
        assert_eq!(queue.list()[1].body, "Send the Q3 numbers\nFriday works.", "updated where it was, not moved");
        queue.remove(&notice.id);
        assert!(!queue.replace(local_notice(&a, "pc")), "a dismissed notice doesn't come back with its draft");
        assert_eq!(queue.list().len(), 1);
    }
}
