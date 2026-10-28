//! Labeled synthetic fixture for `topic_cluster`: ~60 sources across four labeled topics
//! (three projects and one recurring idea), one weekly meeting shared by two projects, and
//! ten noise items. Inserted into `items` / `meeting` / `episodes` and run through
//! `topic_index`, so clustering sees exactly what the real pipeline produces.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rusqlite::params;

use super::index::run_topic_index_job;
use crate::embedding::{Embedder, EmbeddingError, EMBEDDING_OUTPUT_DIM};
use crate::Store;

/// Deterministic "semantic" embedder: fixture keywords add 1.0 to a fixed concept axis,
/// every other word adds 0.35 to a hashed axis, then the vector is normalized. Texts about
/// the same concept are close; "pricing" is deliberately shared by Acme and the idea topic.
pub(crate) struct SemanticEmbedder;

const AXES: &[&[&str]] = &[
    // Deal vocabulary. Company names are not on an axis: a brand name says nothing about
    // what an email is about (the Acme office move must not look like Acme pricing).
    &[
        "tier",
        "tiers",
        "discount",
        "seat",
        "seats",
        "renewal",
        "procurement",
        "contract",
        "enterprise",
        "q3",
    ],
    &["pricing", "price", "prices", "priced"],
    &[
        "kitchen",
        "remodel",
        "renovation",
        "tile",
        "grout",
        "cabinets",
        "countertop",
        "contractor",
        "permit",
        "plumbing",
        "walkthrough",
        "quartz",
        "granite",
        "inspection",
        "demolition",
    ],
    &[
        "board",
        "deck",
        "investor",
        "investors",
        "agenda",
        "minutes",
        "governance",
        "runway",
        "q4",
        "burn",
        "hiring",
    ],
    &["psychology", "anchoring", "decoy", "behavioral", "framing"],
];
const HASHED_FROM: usize = 8;

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

impl Embedder for SemanticEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut v = vec![0f32; EMBEDDING_OUTPUT_DIM];
        for tok in text
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|t| t.len() >= 3 || t == "q3" || t == "q4")
        {
            match AXES.iter().position(|ws| ws.contains(&tok.as_str())) {
                Some(i) => v[i] += 1.0,
                None => {
                    let i =
                        HASHED_FROM + (fnv(&tok) as usize) % (EMBEDDING_OUTPUT_DIM - HASHED_FROM);
                    v[i] += 0.35;
                }
            }
        }
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        v.iter_mut().for_each(|x| *x /= n);
        Ok(v)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

enum Kind {
    Email {
        from: &'static str,
        to: &'static str,
    },
    Cal(&'static [&'static str]),
    /// Attendees; the row body is the meeting summary.
    Meeting(&'static [&'static str]),
    /// Meeting slide OCR (snapshot item); the row body is the OCR text.
    Slide,
    /// The row title is the window title, the body the episode summary.
    Episode,
}

struct Row {
    labels: &'static [&'static str],
    day: i64,
    kind: Kind,
    title: &'static str,
    body: &'static str,
}

const ME: &str = "me@mycorp.com";
const ME_HOME: &str = "me.p@gmail.com";
const SARAH: &str = "Sarah Lee <sarah@acme.com>";
const TOM: &str = "Tom Park <tom@acme.com>";
const RAJ: &str = "Raj Patel <raj@mycorp.com>";
const PRIYA: &str = "Priya Shah <priya@mycorp.com>";
const DIANE: &str = "Diane Chen <diane@sequelvc.com>";
const MARK: &str = "Mark Ito <mark@harborvc.com>";
const BOB: &str = "Bob Ruiz <bob@buildright.com>";
const JAMIE: &str = "Jamie Pointer <jamie.home@gmail.com>";
const TILE: &str = "TileWorld <orders@tileworld.com>";
const LENA: &str = "Lena Cho <lena@mycorp.com>";
const ALEX: &str = "Alex Kim <alex.k@gmail.com>";

pub(crate) const ACME: &str = "Acme Q3 pricing";
pub(crate) const HOME: &str = "Home renovation";
pub(crate) const BOARD: &str = "Board prep";
pub(crate) const IDEA: &str = "pricing psychology";
pub(crate) const NOISE: &str = "noise";
/// Hard negatives: look like a labeled topic (same customer org, same vocabulary, same
/// spouse) but belong to none of them.
pub(crate) const HARD: &str = "hard negative";
pub(crate) const LABELS: [&str; 4] = [ACME, HOME, BOARD, IDEA];
const WEEKLY: &str = "Weekly leadership sync";

use Kind::*;

const ROWS: &[Row] = &[
    // ---- Acme Q3 pricing: emails, meetings, calendar, slides, an Excel episode.
    Row { labels: &[ACME], day: 1, kind: Email { from: SARAH, to: ME }, title: "Acme Q3 Pricing proposal",
        body: "Hi, attached is our draft of the Acme Q3 Pricing tiers. We need a discount on the enterprise seats before renewal. Sarah" },
    Row { labels: &[ACME], day: 2, kind: Email { from: ME, to: "Sarah Lee <sarah@acme.com>, Raj Patel <raj@mycorp.com>" }, title: "Re: Acme Q3 Pricing proposal",
        body: "Thanks Sarah, we will model the tiers and the seat discount this week." },
    Row { labels: &[ACME], day: 3, kind: Cal(&[SARAH, TOM, RAJ]), title: "Acme Q3 Pricing call", body: "" },
    Row { labels: &[ACME], day: 5, kind: Meeting(&[SARAH, TOM, RAJ]), title: "Acme Q3 Pricing review",
        body: "Sarah pushed for a deeper discount on enterprise seats. We agreed to model three tiers before the renewal." },
    Row { labels: &[ACME], day: 5, kind: Slide, title: "Slide",
        body: "Acme Q3 Pricing - proposed tiers for enterprise seats with a 12% renewal discount" },
    Row { labels: &[ACME], day: 6, kind: Episode, title: "Q3 Pricing.xlsx",
        body: "Worked on the Acme Q3 Pricing model with tier discount scenarios." },
    Row { labels: &[ACME], day: 8, kind: Email { from: TOM, to: ME }, title: "Seat counts for renewal",
        body: "Here are the enterprise seat counts for the Acme renewal and the discount we discussed. Tom" },
    Row { labels: &[ACME], day: 10, kind: Email { from: RAJ, to: ME }, title: "Acme discount approval",
        body: "Can you approve the Acme Q3 Pricing discount before the renewal call? Raj" },
    Row { labels: &[ACME], day: 12, kind: Meeting(&[SARAH, TOM, RAJ]), title: "Acme Q3 Pricing follow-up",
        body: "Walked through the revised tiers and the renewal contract; Sarah will take the discount to procurement." },
    Row { labels: &[ACME], day: 13, kind: Slide, title: "Slide",
        body: "Acme Q3 Pricing - revised tiers and renewal contract terms for enterprise seats" },
    Row { labels: &[ACME], day: 15, kind: Email { from: SARAH, to: ME }, title: "Procurement feedback on tiers",
        body: "Procurement accepted the tier structure for Acme Q3 Pricing but wants the discount extended to all seats." },
    Row { labels: &[ACME], day: 18, kind: Cal(&[SARAH, TOM, RAJ]), title: "Acme Q3 Pricing sign-off", body: "" },
    Row { labels: &[ACME], day: 20, kind: Email { from: ME, to: "Sarah Lee <sarah@acme.com>, Tom Park <tom@acme.com>" }, title: "Final Acme Q3 Pricing contract",
        body: "Attached is the final renewal contract with the agreed discount and seat tiers." },
    Row { labels: &[ACME], day: 22, kind: Email { from: SARAH, to: ME }, title: "Signed renewal contract",
        body: "Signed renewal contract attached. Thanks for the work on the Acme Q3 Pricing tiers!" },
    Row { labels: &[ACME], day: 26, kind: Meeting(&[SARAH, TOM, RAJ]), title: "Acme renewal kickoff",
        body: "Kickoff for the renewal rollout of the new Acme Q3 Pricing tiers and enterprise seats." },
    // ---- Acme people not otherwise in the topic, no shared phrase: same deal content, linked
    // only by the acme.com org and their wording.
    Row { labels: &[ACME], day: 17, kind: Email { from: "Nina Gold <nina@acme.com>", to: ME }, title: "Contract redlines",
        body: "Legal finished the redlines on the renewal contract; the discount on enterprise seats and the pricing tiers are unchanged." },
    Row { labels: &[ACME], day: 19, kind: Email { from: "Omar Diaz <omar@acme.com>", to: ME }, title: "Invoice schedule for renewal",
        body: "Finance needs the invoice schedule for the renewal: seats, tiers, and the agreed discount on pricing." },
    Row { labels: &[ACME], day: 21, kind: Email { from: "Lucy Hale <lucy@acme.com>", to: ME }, title: "Procurement checklist",
        body: "Procurement checklist for the renewal contract: confirm seat counts, tier pricing and the discount." },
    // ---- Shared weekly meeting: discusses both Acme and the board deck.
    Row { labels: &[ACME, BOARD], day: 14, kind: Meeting(&[RAJ, PRIYA]), title: WEEKLY,
        body: "Raj gave an update on the Acme Q3 Pricing renewal discount and seat tiers; Priya walked through the Q4 Board Deck runway and investor agenda." },
    // ---- Board prep: meetings, slides, emails with investors.
    Row { labels: &[BOARD], day: 3, kind: Email { from: DIANE, to: ME }, title: "Q4 Board Deck draft",
        body: "Please share the Q4 Board Deck draft by next week, including runway and the hiring plan. Diane" },
    Row { labels: &[BOARD], day: 4, kind: Email { from: ME, to: "Diane Chen <diane@sequelvc.com>, Mark Ito <mark@harborvc.com>" }, title: "Re: Q4 Board Deck draft",
        body: "Draft of the Q4 Board Deck attached: runway, hiring plan and investor update." },
    Row { labels: &[BOARD], day: 6, kind: Meeting(&[PRIYA]), title: "Q4 Board Deck prep",
        body: "Reviewed the runway slide and the agenda for the board meeting with Priya." },
    Row { labels: &[BOARD], day: 6, kind: Slide, title: "Slide",
        body: "Q4 Board Deck - runway of 26 months at current burn" },
    Row { labels: &[BOARD], day: 9, kind: Email { from: MARK, to: ME }, title: "Board meeting agenda",
        body: "Suggested agenda items for the board meeting: governance, runway, and the investor update for the Q4 Board Deck. Mark" },
    Row { labels: &[BOARD], day: 11, kind: Cal(&[DIANE, MARK, PRIYA]), title: "Q4 Board Meeting", body: "" },
    Row { labels: &[BOARD], day: 13, kind: Meeting(&[PRIYA]), title: "Q4 Board Deck agenda",
        body: "Finalised the board meeting agenda and the appendix with hiring and runway." },
    Row { labels: &[BOARD], day: 14, kind: Slide, title: "Slide",
        body: "Q4 Board Deck - hiring plan and investor update" },
    Row { labels: &[BOARD], day: 16, kind: Email { from: PRIYA, to: ME }, title: "Runway numbers",
        body: "Updated runway numbers for the Q4 Board Deck are in the finance folder. Priya" },
    Row { labels: &[BOARD], day: 18, kind: Meeting(&[DIANE, MARK, PRIYA]), title: "Q4 Board Meeting",
        body: "Investors reviewed the deck; governance and runway approved. Minutes to follow." },
    Row { labels: &[BOARD], day: 19, kind: Email { from: DIANE, to: ME }, title: "Q4 Board Meeting minutes",
        body: "Minutes from the board meeting attached, with follow-ups on runway and hiring." },
    // ---- Home renovation: personal Gmail, contractor, tile shop, calendar.
    Row { labels: &[HOME], day: 0, kind: Email { from: JAMIE, to: ME_HOME }, title: "Kitchen Remodel ideas",
        body: "Found some tile ideas for the Kitchen Remodel: white subway tile and oak cabinets?" },
    Row { labels: &[HOME], day: 2, kind: Email { from: BOB, to: ME_HOME }, title: "Kitchen Remodel quote",
        body: "Attached is the quote for the Kitchen Remodel: demolition, plumbing, cabinets and countertop. Bob" },
    Row { labels: &[HOME], day: 4, kind: Cal(&["bob@buildright.com", "jamie.home@gmail.com"]), title: "Kitchen Remodel walkthrough", body: "" },
    Row { labels: &[HOME], day: 7, kind: Email { from: ME_HOME, to: "Bob Ruiz <bob@buildright.com>, Jamie Pointer <jamie.home@gmail.com>" }, title: "Re: Kitchen Remodel quote",
        body: "Looks good, can we start the permit application next week?" },
    Row { labels: &[HOME], day: 9, kind: Email { from: BOB, to: ME_HOME }, title: "Permit submitted",
        body: "The city permit for your kitchen remodel is submitted. Plumbing rough-in starts after approval." },
    Row { labels: &[HOME], day: 12, kind: Email { from: TILE, to: ME_HOME }, title: "Your TileWorld order",
        body: "Your order of white subway tile for the kitchen has shipped." },
    Row { labels: &[HOME], day: 15, kind: Cal(&["bob@buildright.com"]), title: "Kitchen Remodel demolition", body: "" },
    Row { labels: &[HOME], day: 20, kind: Email { from: JAMIE, to: ME_HOME }, title: "Countertop samples",
        body: "Bob dropped off countertop samples for the Kitchen Remodel. Quartz or granite?" },
    Row { labels: &[HOME], day: 25, kind: Episode, title: "Kitchen Remodel budget",
        body: "Updated the renovation budget with the contractor quote and tile order." },
    Row { labels: &[HOME], day: 28, kind: Email { from: TILE, to: ME_HOME }, title: "Your TileWorld order has shipped",
        body: "Grout and trim pieces for your tile order have shipped." },
    Row { labels: &[HOME], day: 33, kind: Email { from: BOB, to: ME_HOME }, title: "Final invoice",
        body: "Final invoice for the Kitchen Remodel; the cabinets and countertop are installed." },
    Row { labels: &[HOME], day: 38, kind: Cal(&["bob@buildright.com", "jamie.home@gmail.com"]), title: "Kitchen Remodel inspection", body: "" },
    // ---- Idea: "Pricing Psychology" recurs over six weeks in otherwise unrelated sources.
    Row { labels: &[IDEA], day: 2, kind: Email { from: ALEX, to: ME_HOME }, title: "Book rec",
        body: "Just finished Pricing Psychology by a former consultant; the anchoring chapter is great." },
    Row { labels: &[IDEA], day: 10, kind: Episode, title: "Pricing Psychology - Kindle",
        body: "Read two chapters about anchoring and decoy effects." },
    Row { labels: &[IDEA], day: 19, kind: Slide, title: "Slide",
        body: "Pricing Psychology for founders: anchoring beats discounting" },
    Row { labels: &[IDEA], day: 27, kind: Meeting(&[LENA]), title: "Coffee with Lena",
        body: "Lena recommended a podcast on Pricing Psychology and behavioral framing." },
    Row { labels: &[IDEA], day: 35, kind: Email { from: "Kim Novak <kim@growthletter.io>", to: ME }, title: "Notes on Pricing Psychology",
        body: "This week: why the decoy option works, and framing effects in checkout flows." },
    Row { labels: &[IDEA], day: 42, kind: Episode, title: "Decoy effect - Wikipedia",
        body: "Read about the decoy effect mentioned in Pricing Psychology." },
    // ---- Hard negatives.
    // Same customer org as Acme, unrelated subject.
    Row { labels: &[HARD], day: 9, kind: Email { from: "Facilities Team <facilities@acme.com>", to: ME }, title: "Acme HQ office move",
        body: "Acme is moving its HQ office to Oakland on June 3. Please update the mailing address and badge access." },
    Row { labels: &[HARD], day: 11, kind: Email { from: "Facilities Team <facilities@acme.com>", to: ME }, title: "Re: Acme HQ office move",
        body: "Parking passes for the new Oakland office will be mailed next week." },
    // Another customer's pricing: same vocabulary as Acme, no shared people.
    Row { labels: &[HARD], day: 7, kind: Email { from: "Bill Lumbergh <bill@globex.com>", to: ME }, title: "Globex Q3 Pricing request",
        body: "Can you send Globex Q3 Pricing for enterprise seats? We want the same tiers and discount as last year." },
    Row { labels: &[HARD], day: 16, kind: Email { from: ME, to: "Bill Lumbergh <bill@globex.com>" }, title: "Re: Globex Q3 Pricing request",
        body: "Attached are the Globex tiers and the seat discount for the renewal." },
    // A second home project with the same spouse.
    Row { labels: &[HARD], day: 22, kind: Email { from: "Ana Ortiz <ana@greenyard.com>", to: ME_HOME }, title: "Backyard Deck quote",
        body: "Quote for the Backyard Deck: cedar boards, railing and the city permit." },
    Row { labels: &[HARD], day: 24, kind: Email { from: JAMIE, to: ME_HOME }, title: "Backyard Deck colors",
        body: "Ana sent stain colors for the Backyard Deck. I like the dark walnut." },
    // ---- Noise: unrelated one-offs, several from the user's own company domain.
    Row { labels: &[NOISE], day: 1, kind: Email { from: LENA, to: ME }, title: "Team offsite lunch",
        body: "Pizza or tacos for the offsite lunch on Thursday?" },
    Row { labels: &[NOISE], day: 4, kind: Email { from: "IT Desk <it@mycorp.com>", to: ME }, title: "Laptop refresh",
        body: "Your laptop refresh is scheduled; please back up your files." },
    Row { labels: &[NOISE], day: 7, kind: Cal(&[]), title: "Haircut", body: "" },
    Row { labels: &[NOISE], day: 11, kind: Email { from: ALEX, to: ME_HOME }, title: "Ski trip?",
        body: "Want to go skiing in Tahoe in January? The cabin has room for six." },
    Row { labels: &[NOISE], day: 14, kind: Episode, title: "Spotify",
        body: "Listened to a jazz playlist while answering mail." },
    Row { labels: &[NOISE], day: 17, kind: Email { from: "Dan Ortiz <dan@mycorp.com>", to: ME }, title: "Interview loop",
        body: "Can you join the interview loop on Tuesday for the backend candidate?" },
    Row { labels: &[NOISE], day: 21, kind: Meeting(&["Kevin Ng <kevin@mycorp.com>"]), title: "1:1 with Kevin",
        body: "Talked about the on-call rotation and flaky deploys." },
    Row { labels: &[NOISE], day: 24, kind: Email { from: "Smile Dental <frontdesk@smiledental.com>", to: ME_HOME }, title: "Appointment reminder",
        body: "Reminder: your dental cleaning is tomorrow at 9am." },
    Row { labels: &[NOISE], day: 30, kind: Email { from: "PG&E <billing@pge.com>", to: ME_HOME }, title: "Your electricity bill",
        body: "Your statement for this month is ready to view." },
    Row { labels: &[NOISE], day: 36, kind: Episode, title: "Terminal",
        body: "Ran cargo build and fixed a lint warning." },
];

const BASE_MS: i64 = 1_780_000_000_000; // 2026-05-28, far older than the index's 1h hold
const DAY_MS: i64 = 86_400_000;

pub(crate) struct Fixture {
    _dir: Option<tempfile::TempDir>,
    pub store: Store,
    /// Label -> topic_source ids. The weekly meeting is in both ACME and BOARD.
    pub labels: BTreeMap<&'static str, BTreeSet<i64>>,
    pub weekly: i64,
}

/// Builds the fixture and runs `topic_index` to completion. With `vectors = false` the
/// store has no working embedder, so no source gets a vector (entity-only clustering).
pub(crate) fn build(vectors: bool) -> Fixture {
    build_with(vectors, |s| while run_topic_index_job(s, 200).unwrap() {})
}

/// Like [`build`], but `drive` runs the indexing (and may interleave clustering).
pub(crate) fn build_with(vectors: bool, drive: impl FnOnce(&Store)) -> Fixture {
    let (dir, store) = if vectors {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_with_embedder(&dir.path().join("t.db"), Arc::new(SemanticEmbedder))
            .unwrap();
        (Some(dir), s)
    } else {
        (None, Store::open_in_memory().unwrap())
    };
    let c = &store.conn;
    for (i, email) in [ME, ME_HOME].iter().enumerate() {
        c.execute(
            "INSERT INTO connector_account (connector, email, is_default, position, connected_at_unix_ms) VALUES ('gmail', ?1, 0, ?2, 0)",
            params![email, i as i64],
        )
        .unwrap();
    }
    let mut refs: Vec<(&'static str, i64, &Row)> = Vec::new();
    for (i, r) in ROWS.iter().enumerate() {
        let ts = BASE_MS + r.day * DAY_MS + i as i64 * 60_000;
        let attendees = |a: &[&str]| serde_json::to_string(a).unwrap();
        let (kind, id) = match &r.kind {
            Email { from, to } => {
                c.execute(
                    "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                     VALUES ('d', 'email', 'gmail', ?1, json_array(?2, ?3), ?4, ?5, '{}', ?6)",
                    params![ts, from, to, r.title, r.body, format!("fx{i}")],
                )
                .unwrap();
                ("email", c.last_insert_rowid())
            }
            Cal(a) => {
                c.execute(
                    "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                     VALUES ('d', 'calendar_event', 'gcal', ?1, ?2, ?3, '', ?4)",
                    params![ts, attendees(a), r.title, format!("fx{i}")],
                )
                .unwrap();
                ("calendar_event", c.last_insert_rowid())
            }
            Slide => {
                c.execute(
                    "INSERT INTO items (device_id, kind, ts_unix_ms, title, body_text, source_table) \
                     VALUES ('d', 'snapshot', ?1, ?2, ?3, 'meeting_snapshot')",
                    params![ts, r.title, r.body],
                )
                .unwrap();
                ("snapshot", c.last_insert_rowid())
            }
            Meeting(a) => {
                c.execute(
                    "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, status, summary_tldr) \
                     VALUES ('d', ?1, ?2, ?3, ?4, 'manual', 'completed', ?5)",
                    params![ts, ts + 1_800_000, r.title, attendees(a), r.body],
                )
                .unwrap();
                ("meeting", c.last_insert_rowid())
            }
            Episode => {
                c.execute(
                    "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('d', 'app', ?1, ?2, ?3, 0, 0, 0, ?4, 'summarized', ?2, ?2)",
                    params![r.title, ts, ts + 600_000, r.body],
                )
                .unwrap();
                ("episode", c.last_insert_rowid())
            }
        };
        refs.push((kind, id, r));
    }
    drive(&store);
    let mut labels: BTreeMap<&'static str, BTreeSet<i64>> = BTreeMap::new();
    let mut weekly = 0;
    for (kind, id, r) in refs {
        let sid: i64 = c
            .query_row(
                "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
                params![kind, id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| panic!("not indexed: {kind} {}", r.title));
        for l in r.labels {
            labels.entry(l).or_default().insert(sid);
        }
        if r.title == WEEKLY {
            weekly = sid;
        }
    }
    Fixture {
        _dir: dir,
        store,
        labels,
        weekly,
    }
}
