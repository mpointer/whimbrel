//! The seeded corpus: one synthetic month written into a `Store` through the same
//! insert paths the app uses, plus the answer key of every fact that was planted.
//!
//! Content is built from templates with slots. A planted fact's value is a token that
//! appears nowhere else in the corpus, so it is findable by keyword; filler sentences
//! use the same projects, people and attributes with no unique value, so retrieval has
//! to discriminate. Generation is a pure function of the seed; timestamps are relative
//! to `now`, which defaults to the current hour so date-scoped questions ("yesterday")
//! work through the real Ask path.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use memory::{
    MeetingStatus, MeetingTrigger, NewCalendarEventItem, NewCaptureItem, NewEmailItem, NewMeeting,
    NewTranscriptSegment, Store,
};

pub const DEFAULT_SEED: u64 = 20_261_104;
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;
const DEVICE: &str = "eval-device";
const SELF_EMAIL: &str = "me@example.test";

pub const PEOPLE: [&str; 8] = [
    "Priya", "Tomas", "Ingrid", "Marcus", "Yuki", "Dolores", "Henrik", "Amara",
];
pub const ORGS: [&str; 3] = ["Larkspur", "Bramblewood", "Northgate"];
pub const PROJECTS: [&str; 4] = ["Falcon", "Heron", "Osprey", "Merlin"];
pub const ATTRIBUTES: [&str; 8] = [
    "budget",
    "deadline",
    "headcount",
    "vendor",
    "venue",
    "release",
    "milestone",
    "forecast",
];
pub const APPS: [&str; 12] = [
    "com.apple.Safari",
    "com.google.Chrome",
    "com.microsoft.VSCode",
    "com.tinyspeck.slackmacgap",
    "com.apple.Notes",
    "com.apple.mail",
    "com.figma.Desktop",
    "com.apple.Terminal",
    "com.linear",
    "com.notion.id",
    "com.apple.Preview",
    "com.apple.iCal",
];

/// How much of a month to generate. `full` is the CI and baseline size; `quick` is for
/// a developer loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorpusProfile {
    pub name: String,
    pub days: i64,
    pub meetings: usize,
    pub emails: usize,
    pub captures: usize,
    pub events: usize,
    /// Planted facts per kind (single, pair, dated, changed, absent).
    pub facts_per_kind: usize,
}

impl CorpusProfile {
    pub fn full() -> Self {
        CorpusProfile {
            name: "full".into(),
            days: 30,
            meetings: 40,
            emails: 300,
            captures: 2000,
            events: 60,
            facts_per_kind: 25,
        }
    }
    pub fn quick() -> Self {
        CorpusProfile {
            name: "quick".into(),
            days: 30,
            meetings: 8,
            emails: 40,
            captures: 200,
            events: 12,
            facts_per_kind: 5,
        }
    }
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "full" => Some(Self::full()),
            "quick" => Some(Self::quick()),
            _ => None,
        }
    }
}

/// Where a fact was planted. `Email` names the Gmail thread, because emails reach Ask
/// only through a thread summary, never as items (see `retrieval`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceRef {
    Capture { id: i64 },
    Transcript { id: i64 },
    Email { gmail_thread_id: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// One sentence in one source.
    Single,
    /// Two halves in two sources: the value in one, who owns it in the other.
    Pair,
    /// A sentence whose timestamp falls on a chosen day (today or yesterday).
    Dated,
    /// Version 1 in an early source, version 2 in a later one; v2 is current.
    Changed,
    /// Never planted; the right answer is "not in memory".
    Absent,
}

/// Where a planted sentence goes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Transcript,
    Capture,
    Email,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlantedFact {
    pub id: String,
    pub kind: FactKind,
    pub container: Container,
    pub project: String,
    pub attribute: String,
    pub person: String,
    /// The unique answer token (a number or an invented proper noun).
    pub value: String,
    /// For `Pair`: the owner named in the second half. For `Changed`: the superseded value.
    pub secondary: Option<String>,
    pub day_offset: i64,
    pub ts_unix_ms: i64,
    pub sentences: Vec<String>,
    /// Resolved after insertion by looking the unique token up the way Ask does.
    pub sources: Vec<SourceRef>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CorpusCounts {
    pub meetings: usize,
    pub transcript_lines: usize,
    pub emails: usize,
    pub captures: usize,
    pub events: usize,
    pub facts: usize,
    pub unresolved_sources: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Corpus {
    pub seed: u64,
    pub now_unix_ms: i64,
    pub profile: CorpusProfile,
    pub facts: Vec<PlantedFact>,
    pub counts: CorpusCounts,
}

/// The current hour, so "today" and "yesterday" line up with the real clock.
pub fn default_now_unix_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now - now % HOUR_MS
}

struct Gen {
    rng: StdRng,
    used_values: std::collections::HashSet<String>,
}

impl Gen {
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        items.choose(&mut self.rng).expect("non-empty")
    }
    /// A number that appears nowhere else in the corpus.
    fn unique_number(&mut self) -> String {
        loop {
            let n: u32 = self.rng.gen_range(10_000..99_999);
            let s = n.to_string();
            if self.used_values.insert(s.clone()) {
                return s;
            }
        }
    }
    /// An invented proper noun that appears nowhere else in the corpus.
    fn unique_name(&mut self) -> String {
        const STARTS: [&str; 12] = [
            "Var", "Mol", "Tes", "Kar", "Bel", "Dor", "Fen", "Gal", "Hol", "Jur", "Lin", "Ped",
        ];
        const MIDS: [&str; 8] = ["an", "er", "is", "or", "ul", "ev", "ad", "im"];
        const ENDS: [&str; 8] = [
            "holt", "wick", "mere", "dane", "ford", "stad", "brook", "vale",
        ];
        loop {
            let s = format!(
                "{}{}{}",
                self.pick(&STARTS),
                self.pick(&MIDS),
                self.pick(&ENDS)
            );
            if self.used_values.insert(s.clone()) {
                return s;
            }
        }
    }
    fn value_for(&mut self, attribute: &str) -> String {
        match attribute {
            "vendor" | "venue" => self.unique_name(),
            _ => self.unique_number(),
        }
    }
}

fn sentence_for(person: &str, project: &str, attribute: &str, value: &str) -> String {
    match attribute {
        "budget" => format!("{person} said the {project} budget is {value} dollars."),
        "deadline" => {
            format!("{person} confirmed the {project} deadline moved to day {value} of the plan.")
        }
        "headcount" => {
            format!("{person} wants {project} headcount at {value} people next quarter.")
        }
        "vendor" => format!("{person} chose {value} as the {project} vendor."),
        "venue" => format!("{person} booked {value} as the venue for the {project} offsite."),
        "release" => format!("{person} set the {project} release build number to {value}."),
        "milestone" => format!("{person} named the next {project} milestone ticket {value}."),
        _ => format!("{person} put the {project} forecast at {value} units."),
    }
}

fn filler_sentence(g: &mut Gen) -> String {
    const SHAPES: [&str; 8] = [
        "{person} asked for an update on {project} before the review.",
        "The {project} {attribute} is still under discussion with {org}.",
        "{person} will write up the {project} notes for {org}.",
        "Nothing changed on {project} this week, said {person}.",
        "{org} wants a call about the {project} {attribute} next week.",
        "{person} is out on Friday, so {project} waits until Monday.",
        "Reminder from {person}: the {project} {attribute} review is not scheduled yet.",
        "{person} and {person2} disagreed about the {project} scope again.",
    ];
    let shape = *g.pick(&SHAPES);
    let person = *g.pick(&PEOPLE);
    let person2 = *g.pick(&PEOPLE);
    let project = *g.pick(&PROJECTS);
    let attribute = *g.pick(&ATTRIBUTES);
    let org = *g.pick(&ORGS);
    shape
        .replace("{person2}", person2)
        .replace("{person}", person)
        .replace("{project}", project)
        .replace("{attribute}", attribute)
        .replace("{org}", org)
}

/// Plans the facts, writes the month, then resolves each fact's sources by looking its
/// unique token up through the store's own search, which is how Ask would find it.
pub fn generate(
    store: &Store,
    seed: u64,
    profile: &CorpusProfile,
    now_unix_ms: i64,
) -> rusqlite::Result<Corpus> {
    let mut g = Gen {
        rng: StdRng::seed_from_u64(seed),
        used_values: Default::default(),
    };
    let start = now_unix_ms - profile.days * DAY_MS;

    // Plan the facts and the container each one lands in.
    let mut facts: Vec<PlantedFact> = Vec::new();
    let containers = [Container::Transcript, Container::Capture, Container::Email];
    let mut n = 0usize;
    let mut plan = |g: &mut Gen, kind: FactKind, facts: &mut Vec<PlantedFact>| {
        let project = g.pick(&PROJECTS).to_string();
        let attribute = g.pick(&ATTRIBUTES).to_string();
        let person = g.pick(&PEOPLE).to_string();
        let container = *g.pick(&containers);
        let value = g.value_for(&attribute);
        let day_offset = match kind {
            // Today or yesterday, so the date-scoped window finds it.
            FactKind::Dated => -(g.rng.gen_range(0..2) as i64),
            _ => -(g.rng.gen_range(2..profile.days.max(3)) as i64),
        };
        let hour = g.rng.gen_range(9..18) as i64;
        let ts = now_unix_ms + day_offset * DAY_MS - (now_unix_ms % DAY_MS) + hour * HOUR_MS;
        let ts = ts.min(now_unix_ms - 60_000);
        n += 1;
        let id = format!("f{n:03}");
        let mut secondary = None;
        let mut sentences = vec![sentence_for(&person, &project, &attribute, &value)];
        match kind {
            FactKind::Pair => {
                // The second half names the owner. People's names recur everywhere, so
                // the half also carries its own unique code for source resolution.
                let owner = g.pick(&PEOPLE).to_string();
                let code = g.unique_number();
                sentences = vec![
                    format!("The {project} {attribute} is {value}, {person} wrote."),
                    format!("{owner} owns the {project} {attribute} from now on, tracked as item {code}."),
                ];
                secondary = Some(format!("{owner} {code}"));
            }
            FactKind::Changed => {
                let old = g.value_for(&attribute);
                sentences = vec![
                    sentence_for(&person, &project, &attribute, &old),
                    sentence_for(&person, &project, &attribute, &value),
                ];
                secondary = Some(old);
            }
            FactKind::Absent => {
                sentences.clear();
            }
            _ => {}
        }
        facts.push(PlantedFact {
            id,
            kind,
            container,
            project,
            attribute,
            person,
            value,
            secondary,
            day_offset,
            ts_unix_ms: ts,
            sentences,
            sources: Vec::new(),
            supersedes: None,
        });
    };
    for kind in [
        FactKind::Single,
        FactKind::Pair,
        FactKind::Dated,
        FactKind::Changed,
        FactKind::Absent,
    ] {
        for _ in 0..profile.facts_per_kind {
            plan(&mut g, kind, &mut facts);
        }
    }

    // Sentences to plant, keyed by container, with the fact id and which half.
    let mut to_plant: Vec<(Container, String, usize, i64)> = Vec::new();
    for f in &facts {
        for (i, _) in f.sentences.iter().enumerate() {
            // A changed fact's first version lands a week earlier than its second.
            let ts = if f.kind == FactKind::Changed && i == 0 {
                f.ts_unix_ms - 7 * DAY_MS
            } else {
                f.ts_unix_ms
            };
            to_plant.push((f.container, f.id.clone(), i, ts));
        }
    }
    let sentence_of = |facts: &[PlantedFact], id: &str, i: usize| -> String {
        facts
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.sentences[i].clone())
            .unwrap_or_default()
    };

    let mut counts = CorpusCounts::default();

    // Meetings with transcripts. Planted transcript lines go into the meeting nearest
    // their timestamp.
    let mut meeting_starts: Vec<i64> = (0..profile.meetings)
        .map(|i| {
            start
                + (i as i64 * profile.days * DAY_MS) / profile.meetings.max(1) as i64
                + 10 * HOUR_MS
        })
        .collect();
    meeting_starts.sort_unstable();
    for (mi, m_start) in meeting_starts.iter().enumerate() {
        let project = *g.pick(&PROJECTS);
        let attendees: Vec<String> = {
            let mut p: Vec<&str> = PEOPLE.to_vec();
            p.shuffle(&mut g.rng);
            p.into_iter()
                .take(g.rng.gen_range(2..5))
                .map(|s| format!("{s} <{}@example.test>", s.to_lowercase()))
                .collect()
        };
        let meeting_id = store.insert_meeting(&NewMeeting {
            device_id: DEVICE.into(),
            start_unix_ms: *m_start,
            end_unix_ms: Some(*m_start + 45 * 60_000),
            title: format!("{project} sync {}", mi + 1),
            attendees: attendees.clone(),
            trigger: MeetingTrigger::Calendar,
            audio_path: None,
            status: MeetingStatus::Completed,
        })?;
        counts.meetings += 1;
        let lines = g.rng.gen_range(12..40);
        let mut segs: Vec<NewTranscriptSegment> = (0..lines)
            .map(|li| NewTranscriptSegment {
                meeting_id,
                start_cs: (li as i64) * 1500,
                speaker_label: format!("SPEAKER_{:02}", g.rng.gen_range(0..attendees.len())),
                text: filler_sentence(&mut g),
            })
            .collect();
        // Planted lines: facts whose nearest meeting is this one.
        let planted: Vec<(String, usize)> = to_plant
            .iter()
            .filter(|(c, _, _, ts)| {
                *c == Container::Transcript
                    && meeting_starts
                        .iter()
                        .min_by_key(|s| (*s - ts).abs())
                        .map(|s| s == m_start)
                        .unwrap_or(false)
            })
            .map(|(_, id, i, _)| (id.clone(), *i))
            .collect();
        // Distinct line positions, so two facts in one meeting never overwrite each other.
        let mut slots: Vec<usize> = (0..segs.len()).collect();
        slots.shuffle(&mut g.rng);
        for (id, i) in planted {
            let at = match slots.pop() {
                Some(at) => at,
                None => {
                    segs.push(NewTranscriptSegment {
                        meeting_id,
                        start_cs: segs.len() as i64 * 1500,
                        speaker_label: "SPEAKER_00".into(),
                        text: String::new(),
                    });
                    segs.len() - 1
                }
            };
            segs[at].text = sentence_of(&facts, &id, i);
        }
        counts.transcript_lines += segs.len();
        store.insert_transcript_segments(&segs)?;
    }

    // Captures: screen text across apps, one unique hash each so none is deduped.
    for ci in 0..profile.captures {
        let ts = start
            + (ci as i64 * profile.days * DAY_MS) / profile.captures.max(1) as i64
            + 8 * HOUR_MS
            + g.rng.gen_range(0..9 * HOUR_MS);
        let app = *g.pick(&APPS);
        let project = *g.pick(&PROJECTS);
        let text = format!("{}\n{}", filler_sentence(&mut g), filler_sentence(&mut g));
        store.insert_capture_record(&[NewCaptureItem {
            device_id: DEVICE.into(),
            ts_unix_ms: ts.min(now_unix_ms - 60_000),
            app_bundle_id: app.into(),
            window_title: format!("{project} notes"),
            url: None,
            text,
            hash: ci as i64 + 1,
            source: "ax".into(),
        }])?;
        counts.captures += 1;
    }
    for (c, id, i, ts) in &to_plant {
        if *c != Container::Capture {
            continue;
        }
        let app = *g.pick(&APPS);
        let f = facts.iter().find(|f| &f.id == id).expect("planned fact");
        store.insert_capture_record(&[NewCaptureItem {
            device_id: DEVICE.into(),
            ts_unix_ms: *ts,
            app_bundle_id: app.into(),
            window_title: format!("{} notes", f.project),
            url: None,
            text: format!(
                "{}\n{}",
                sentence_of(&facts, id, *i),
                filler_sentence(&mut g)
            ),
            hash: 1_000_000 + counts.captures as i64,
            source: "ax".into(),
        }])?;
        counts.captures += 1;
    }

    // Emails in threads. A planted email sentence is the body of its own thread.
    let threads = (profile.emails / 3).max(1);
    let mut emails: Vec<NewEmailItem> = Vec::new();
    for ei in 0..profile.emails {
        let thread = ei % threads;
        let ts = start
            + (ei as i64 * profile.days * DAY_MS) / profile.emails.max(1) as i64
            + 7 * HOUR_MS
            + g.rng.gen_range(0..12 * HOUR_MS);
        let from = *g.pick(&PEOPLE);
        let project = *g.pick(&PROJECTS);
        emails.push(NewEmailItem {
            device_id: DEVICE.into(),
            external_id: format!("msg-{ei}"),
            ts_unix_ms: ts.min(now_unix_ms - 60_000),
            from: format!("{from} <{}@example.test>", from.to_lowercase()),
            to: SELF_EMAIL.into(),
            subject: format!("Re: {project} thread {thread}"),
            body_text: format!("{}\n\n{}", filler_sentence(&mut g), filler_sentence(&mut g)),
            gmail_thread_id: format!("thread-{thread}"),
            account_email: SELF_EMAIL.into(),
            gmail_labels: vec!["INBOX".into()],
            list_unsubscribe: false,
        });
    }
    let mut planted_thread = 0usize;
    for (c, id, i, ts) in &to_plant {
        if *c != Container::Email {
            continue;
        }
        let f = facts.iter().find(|f| &f.id == id).expect("planned fact");
        planted_thread += 1;
        emails.push(NewEmailItem {
            device_id: DEVICE.into(),
            external_id: format!("msg-planted-{id}-{i}"),
            ts_unix_ms: *ts,
            from: format!("{} <{}@example.test>", f.person, f.person.to_lowercase()),
            to: SELF_EMAIL.into(),
            subject: format!("{} {}", f.project, f.attribute),
            body_text: sentence_of(&facts, id, *i),
            gmail_thread_id: format!("thread-planted-{planted_thread}"),
            account_email: SELF_EMAIL.into(),
            gmail_labels: vec!["INBOX".into()],
            list_unsubscribe: false,
        });
    }
    counts.emails = emails.len();
    store.upsert_email_items(&emails)?;
    // The app folds synced emails into `threads` in a background job; do the same here.
    while store.run_thread_state_sync_job(now_unix_ms)? {}

    // Calendar events, some in the future for "up next".
    let events: Vec<NewCalendarEventItem> = (0..profile.events)
        .map(|ei| {
            let ts = start
                + (ei as i64 * (profile.days + 7) * DAY_MS) / profile.events.max(1) as i64
                + 9 * HOUR_MS;
            let project = *g.pick(&PROJECTS);
            NewCalendarEventItem {
                device_id: DEVICE.into(),
                source: "google_calendar".into(),
                external_id: format!("evt-{ei}"),
                ts_unix_ms: ts,
                title: format!("{project} planning"),
                actors: vec![format!("{}@example.test", g.pick(&PEOPLE).to_lowercase())],
            }
        })
        .collect();
    counts.events = events.len();
    store.upsert_calendar_event_items(&events)?;

    // Episodes from the captures, the way the app's job runner builds them.
    let _ = store.run_episode_segment_batch(10_000, now_unix_ms);

    // Resolve sources by unique token through the store's own search.
    for f in facts.iter_mut() {
        if f.kind == FactKind::Absent {
            continue;
        }
        let tokens: Vec<String> = match f.kind {
            // The owner half resolves by its code, the last word of `secondary`.
            FactKind::Pair => vec![
                f.value.clone(),
                f.secondary
                    .as_deref()
                    .and_then(|s| s.split_whitespace().last())
                    .unwrap_or_default()
                    .to_string(),
            ],
            FactKind::Changed => vec![f.secondary.clone().unwrap_or_default(), f.value.clone()],
            _ => vec![f.value.clone()],
        };
        for (i, token) in tokens.iter().enumerate() {
            let found = match f.container {
                Container::Transcript => store
                    .search_transcript_keyword_with_meeting_ts(token, 3)?
                    .into_iter()
                    .map(|(t, _)| SourceRef::Transcript { id: t.id })
                    .next(),
                Container::Capture => store
                    .search_keyword(token, 3)?
                    .into_iter()
                    .map(|c| SourceRef::Capture { id: c.id })
                    .next(),
                Container::Email => Some(SourceRef::Email {
                    gmail_thread_id: format!(
                        "thread-planted-{}",
                        email_thread_index(&to_plant, &f.id, i)
                    ),
                }),
            };
            match found {
                Some(s) => f.sources.push(s),
                None => counts.unresolved_sources += 1,
            }
        }
    }
    // A changed fact's second version supersedes its first.
    for f in facts.iter_mut() {
        if f.kind == FactKind::Changed {
            f.supersedes = Some(format!("{}#v1", f.id));
        }
    }
    counts.facts = facts.len();

    Ok(Corpus {
        seed,
        now_unix_ms,
        profile: profile.clone(),
        facts,
        counts,
    })
}

/// The planted-thread index assigned to (`fact_id`, `half`) above, recomputed in the
/// same order so the source reference names the thread the email went into.
fn email_thread_index(
    to_plant: &[(Container, String, usize, i64)],
    fact_id: &str,
    half: usize,
) -> usize {
    let mut n = 0;
    for (c, id, i, _) in to_plant {
        if *c != Container::Email {
            continue;
        }
        n += 1;
        if id == fact_id && *i == half {
            return n;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{open_store, EmbedderChoice};

    #[test]
    fn the_same_seed_plants_the_same_facts() {
        let (a, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let (b, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let now = 1_800_000_000_000;
        let ca = generate(&a, 7, &CorpusProfile::quick(), now).unwrap();
        let cb = generate(&b, 7, &CorpusProfile::quick(), now).unwrap();
        assert_eq!(ca.facts, cb.facts);
        assert_eq!(ca.counts, cb.counts);
    }

    #[test]
    fn every_planted_value_is_unique_and_every_non_email_source_resolves() {
        let (store, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let c = generate(
            &store,
            DEFAULT_SEED,
            &CorpusProfile::quick(),
            1_800_000_000_000,
        )
        .unwrap();
        let mut seen = std::collections::HashSet::new();
        for f in &c.facts {
            assert!(seen.insert(f.value.clone()), "duplicate value {}", f.value);
            if let Some(s) = &f.secondary {
                if f.kind == FactKind::Changed {
                    assert!(seen.insert(s.clone()), "duplicate old value {s}");
                }
                if f.kind == FactKind::Pair {
                    let code = s.split_whitespace().last().unwrap().to_string();
                    assert!(seen.insert(code.clone()), "duplicate owner code {code}");
                }
            }
            if f.kind != FactKind::Absent {
                let expected = match f.kind {
                    FactKind::Pair | FactKind::Changed => 2,
                    _ => 1,
                };
                assert_eq!(
                    f.sources.len(),
                    expected,
                    "{} ({:?}) resolved {:?}",
                    f.id,
                    f.container,
                    f.sources
                );
            }
        }
        assert_eq!(c.counts.unresolved_sources, 0);
        assert_eq!(c.counts.meetings, 8);
        assert!(c.counts.captures >= 200);
    }

    #[test]
    fn the_full_profile_resolves_every_source() {
        let (store, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let c = generate(
            &store,
            DEFAULT_SEED,
            &CorpusProfile::full(),
            default_now_unix_ms(),
        )
        .unwrap();
        let unresolved: Vec<String> = c
            .facts
            .iter()
            .filter(|f| f.kind != FactKind::Absent)
            .filter(|f| {
                f.sources.len()
                    < if matches!(f.kind, FactKind::Pair | FactKind::Changed) {
                        2
                    } else {
                        1
                    }
            })
            .map(|f| {
                format!(
                    "{} {:?} {:?} value={} secondary={:?} sources={:?}",
                    f.id, f.kind, f.container, f.value, f.secondary, f.sources
                )
            })
            .collect();
        assert!(unresolved.is_empty(), "unresolved: {unresolved:#?}");
    }

    #[test]
    fn dated_facts_fall_on_today_or_yesterday() {
        let (store, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let now = 1_800_000_000_000;
        let c = generate(&store, 3, &CorpusProfile::quick(), now).unwrap();
        for f in c.facts.iter().filter(|f| f.kind == FactKind::Dated) {
            assert!(f.day_offset == 0 || f.day_offset == -1);
            assert!(f.ts_unix_ms < now && f.ts_unix_ms > now - 2 * DAY_MS);
        }
    }
}
