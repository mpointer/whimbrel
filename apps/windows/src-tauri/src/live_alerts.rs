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

/// The parts of a stored draft (`llm::ChatAnswer` as JSON) the alerts use.
#[derive(serde::Deserialize)]
struct Draft {
    text: String,
    #[serde(default)]
    citations: Vec<serde_json::Value>,
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
            out.push(QuestionNotice {
                id: format!("live-alert-more-{origin}-{source_meeting_id}"),
                title: alert.meeting_title.clone(),
                subtitle: None,
                body: tr("{count} more questions for you", &[("count", &extra.to_string())]),
                origin,
                source_meeting_id,
                meeting_id: alert.meeting_id,
            });
        }
        out
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
    /// When it was asked, "m:ss" from the meeting's start.
    pub asked_at: String,
    pub created_at_unix_ms: i64,
}

/// "m:ss" for centiseconds from the meeting's start.
pub fn clock(cs: i64) -> String {
    let seconds = cs.max(0) / 100;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn alert_view(alert: &LiveAlert) -> LiveAlertView {
    let draft = draft(alert.draft_json.as_deref());
    LiveAlertView {
        id: alert.id,
        restatement: alert.restatement.clone(),
        quote: alert.quote.clone(),
        asker: asker(alert).map(str::to_string),
        sources_elsewhere: alert.origin_device_id.is_some() && draft.as_ref().is_some_and(|d| !d.citations.is_empty()),
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
        let mut a = alert(Some("mac"), 1, 7);
        let view = alert_view(&a);
        assert_eq!((view.draft.as_deref(), view.sources_elsewhere, view.asked_at.as_str()), (None, false, "2:03"));
        a.draft_json = Some(r#"{"text":"They're in the deck.","citations":[{"kind":"capture","id":3}]}"#.into());
        let view = alert_view(&a);
        assert_eq!(view.draft.as_deref(), Some("They're in the deck."));
        assert!(view.sources_elsewhere);
        a.origin_device_id = None;
        assert!(!alert_view(&a).sources_elsewhere, "this PC's own citations resolve here");
    }
}
