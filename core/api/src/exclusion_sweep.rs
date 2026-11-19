//! Task 8 (X6 client controls, spec 7.1-7.4, 10): one test per hosted caller, each with a
//! recording hosted transport behind the real gate, asserting that no request carries an
//! excluded client's material. The canary is text filed under the client that never names it
//! ([`CANARY`]): the gate alone (Task 1) lets it through, so only the retrieval filter, the
//! request classification and classify-first keep it on this computer. The sweep at the end
//! lists every hosted caller in the code, so a new one fails the build until it is covered.

use std::sync::{Arc, Mutex};

use memory::topics::client_controls::ClientControl;

use crate::exclusion::{ExclusionOutcome, RequestOrigin};
use crate::routing::RoutingConfig;

/// Words filed under the excluded client that never name it.
const CANARY: &str = "zebracanary";
/// Words of another client's meeting, which a hosted answer may carry.
const OTHER: &str = "giraffe";

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn prompts(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// A model transport that keeps every prompt it is sent.
struct Recorder {
    log: Log,
    reply: String,
}

impl llm::LlmClient for Recorder {
    fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
        self.log.0.lock().unwrap().push(prompt.to_string());
        Ok(llm::Completion { text: self.reply.clone(), token_usage: llm::TokenUsage::default() })
    }
}

/// A hosted link recording into `log`, behind the gate production uses.
fn hosted(store: &memory::Store, log: &Log, reply: &str) -> llm::ChainLink {
    llm::guard_hosted(
        llm::ChainLink {
            provider: llm::Provider::Anthropic,
            model: "claude-test".into(),
            client: Box::new(Recorder { log: log.clone(), reply: reply.into() }),
        },
        crate::exclusion::current_gate(store),
    )
}

/// An on-device link recording into `log`.
fn local(log: &Log, reply: &str) -> llm::ChainLink {
    llm::ChainLink {
        provider: llm::Provider::Ollama,
        model: "gemma-test".into(),
        client: Box::new(Recorder { log: log.clone(), reply: reply.into() }),
    }
}

/// No hosted request carried the client's material, its name, an alias, or why.
fn assert_clean(log: &Log) {
    for p in log.prompts() {
        let lower = p.to_lowercase();
        for banned in [CANARY, "acme", "roadrunner", "exclu", "left out"] {
            assert!(!lower.contains(banned), "a hosted request carried {banned:?}:\n{p}");
        }
    }
}

/// Fix round 2, M3: the control request went through the real caller, which logged a hosted
/// call under its own route (`usage_log`), not only through a test helper.
fn assert_reached(c: &rusqlite::Connection, route: &str) {
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM usage_log WHERE route = ?1 AND provider NOT IN ('ollama', 'local')",
            [route],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n > 0, "no hosted call was logged under route {route}: the test never reached the caller");
}

fn hosted_routing() -> RoutingConfig {
    RoutingConfig { preferred_provider: "anthropic".into(), anthropic_key: Some("k".into()), ..RoutingConfig::default() }
}

struct Fx {
    _dir: tempfile::TempDir,
    s: memory::Store,
    c: rusqlite::Connection,
    acme_key: String,
    acme_topic: i64,
    /// Filed under Acme; its title, attendees and words never name it.
    acme_meeting: i64,
    other_meeting: i64,
}

fn meeting(s: &memory::Store, title: &str, attendees: &[&str], start: i64, lines: &[&str], tldr: &str) -> i64 {
    let id = s
        .insert_meeting(&memory::NewMeeting {
            device_id: "d".into(),
            start_unix_ms: start,
            end_unix_ms: Some(start + 60_000),
            title: title.into(),
            attendees: attendees.iter().map(|a| a.to_string()).collect(),
            trigger: memory::MeetingTrigger::Calendar,
            audio_path: None,
            status: memory::MeetingStatus::Completed,
        })
        .unwrap();
    // Not from Google Calendar (Limited Use would hold it from some links): these tests check the
    // exclusion gate alone; Limited Use has its own tests.
    assert!(s.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap());
    let segments: Vec<memory::NewTranscriptSegment> = lines
        .iter()
        .enumerate()
        .map(|(i, t)| memory::NewTranscriptSegment { meeting_id: id, start_cs: i as i64 * 100, speaker_label: "Speaker 1".into(), text: t.to_string() })
        .collect();
    s.insert_transcript_segments(&segments).unwrap();
    s.update_meeting_summary(id, tldr, &[], &[], &[], "anthropic", "m").unwrap();
    id
}

fn file(c: &rusqlite::Connection, topic: i64, kind: &str, ref_id: i64) -> i64 {
    c.execute(
        "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1, 1)",
        rusqlite::params![kind, ref_id],
    )
    .unwrap();
    let sid = c.last_insert_rowid();
    c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", rusqlite::params![topic, sid])
        .unwrap();
    sid
}

/// Acme (alias Roadrunner Co) with one meeting filed under it, and another client's meeting.
/// `excluded`: Acme is excluded from hosted models.
fn fixture(excluded: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    let s = crate::test_store::open(&path, dir.path());
    let c = crate::test_store::raw(&path);
    let acme = s.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
    let globex = s.create_seed_topic("Globex", &[], None).unwrap();
    let now = crate::util::now_unix_ms();
    let acme_meeting = meeting(
        &s,
        "Quarterly sync",
        &["Bob <bob@example.com>"],
        now - 3_600_000,
        &[&format!("We agreed the {CANARY} pricing for the launch.")],
        &format!("Agreed the {CANARY} pricing."),
    );
    let other_meeting = meeting(
        &s,
        "Team standup",
        &["Carol <carol@example.com>"],
        now - 7_200_000,
        &[&format!("The {OTHER} pricing review is next week.")],
        &format!("{OTHER} pricing review next week."),
    );
    file(&c, acme.topic_id, "meeting", acme_meeting);
    file(&c, globex.topic_id, "meeting", other_meeting);
    if excluded {
        s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    }
    Fx { _dir: dir, s, c, acme_key: acme.key, acme_topic: acme.topic_id, acme_meeting, other_meeting }
}

fn ask(fx: &Fx, query: &str, origin: RequestOrigin, hosted_log: &Log, local_log: Option<&Log>) -> crate::ApiResult<(llm::ChatAnswer, ExclusionOutcome)> {
    crate::ask::answer_with(
        &fx.s,
        query,
        &origin,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, hosted_log, "An answer.")],
        || local_log.map(|l| local(l, "A local answer.")).into_iter().collect(),
    )
}

// ---- Ask, window Ask, hotkey -------------------------------------------------------------

#[test]
fn ask_never_sends_excluded_text() {
    // Control: with nothing excluded, the hosted prompt does carry the canary.
    let open = fixture(false);
    let log = Log::default();
    ask(&open, "what about the pricing?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts().iter().any(|p| p.contains(CANARY)), "the fixture reaches retrieval");

    let fx = fixture(true);
    let log = Log::default();
    let (_, outcome) = ask(&fx, "what about the pricing?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_eq!(log.count(), 1);
    assert!(log.prompts()[0].contains(OTHER), "other material still answers");
    assert_clean(&log);
    assert_reached(&fx.c, "ask");
    assert!(matches!(outcome, ExclusionOutcome::FilteredHosted { left_out } if left_out >= 1), "{outcome:?}");

    // The streaming prepare builds the same prompt for a hosted stream.
    let prepared = crate::ask::prepare_ask(&fx.s, "what about the pricing?", 10, &[], &hosted_routing()).unwrap();
    assert!(!prepared.prompt.contains(CANARY));
    assert!(matches!(prepared.exclusion, ExclusionOutcome::FilteredHosted { .. }));
    crate::ask::discard_ask(&prepared.prepare_id);

    // The Mac's entry point (main's Limited Use settings per call) goes through the same gate,
    // and a question naming the client is refused for a hosted stream.
    let call = crate::routing::LimitedUseSettings::default();
    let prepared = crate::ask::prepare_ask_for(&fx.s, "what about the pricing?", 10, &[], &hosted_routing(), &call).unwrap();
    assert!(!prepared.prompt.contains(CANARY));
    assert!(prepared.prompt.contains(OTHER));
    crate::ask::discard_ask(&prepared.prepare_id);
    assert!(crate::ask::prepare_ask_for(&fx.s, "what did Acme say?", 10, &[], &hosted_routing(), &call).is_err());
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
}

#[test]
fn ask_reports_the_model_that_really_answered_and_whether_a_named_client_forced_it_local() {
    let fx = fixture(true);
    // The cheap pre-check the Ask bar uses: true for a question naming the client or an alias,
    // false for one that does not, and never the name itself.
    assert!(crate::ask::question_names_excluded_client(&fx.s, "What do I know about Acme?", &[]));
    assert!(crate::ask::question_names_excluded_client(&fx.s, "tell me about roadrunner co", &[]));
    assert!(!crate::ask::question_names_excluded_client(&fx.s, "what about the pricing?", &[]));
    let named = [llm::ChatTurn { role: llm::ChatRole::User, text: "What did Acme say?".into() }];
    assert!(crate::ask::question_names_excluded_client(&fx.s, "and the follow up?", &named));

    // A hosted picker, a question naming the client: the local link answers and says so.
    let hosted_log = Log::default();
    let local_log = Log::default();
    let (answer, outcome, by) = crate::ask::answer_with_filter_by(
        &fx.s,
        crate::exclusion::context_filter(&fx.s),
        "What do I know about Acme?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, &hosted_log, "Hosted.")],
        || vec![local(&local_log, "Local.")],
    )
    .unwrap();
    assert_eq!(answer.text, "Local.");
    assert_eq!(outcome, ExclusionOutcome::LocalOnly);
    assert_eq!(by, crate::ask::AnsweredBy { provider: "ollama".into(), model: "gemma-test".into(), local: true });
    assert_eq!(hosted_log.count(), 0);

    // A question that does not name the client is answered by the hosted link, reported as such.
    let hosted_log = Log::default();
    let (_, _, by) = crate::ask::answer_with_filter_by(
        &fx.s,
        crate::exclusion::context_filter(&fx.s),
        "what about the pricing?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, &hosted_log, "Hosted.")],
        || vec![],
    )
    .unwrap();
    assert_eq!(by, crate::ask::AnsweredBy { provider: "anthropic".into(), model: "claude-test".into(), local: false });

    // The envelope the shells decode carries the answering link and never the client's name.
    let detailed = crate::ask::DetailedAnswer {
        answer: llm::ChatAnswer { text: "Local.".into(), citations: vec![], token_usage: llm::TokenUsage::default() },
        exclusion: ExclusionOutcome::LocalOnly,
        answered_by: by,
    };
    let json = serde_json::to_value(&detailed).unwrap();
    assert_eq!(json["text"], "Local.");
    assert_eq!(json["exclusion"]["kind"], "local_only");
    assert_eq!(json["answered_by"]["model"], "claude-test");
    assert!(!json.to_string().to_lowercase().contains("acme"));
}

#[test]
fn answer_detailed_never_sends_excluded_text() {
    // `answer_detailed` is the blocking Ask on Windows (commands/chat.rs) and behind the Mac's
    // `whimbrel_llm_answer_detailed`: `answer_with_filter_by` with the chains built from routing.
    let fx = fixture(true);

    // Through the real entry: a question naming the client, or a conversation that named it, with
    // no model on this computer is refused before any chain is built, so nothing is sent.
    let routing = RoutingConfig { use_local_model: false, ..hosted_routing() };
    let err = crate::ask::answer_detailed(&fx.s, "What did Acme decide on pricing?", "gemma-test", 10, &[], &routing).unwrap_err();
    assert!(matches!(err, crate::ApiError::ProviderFailure), "{err:?}");
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
    let named = [llm::ChatTurn { role: llm::ChatRole::User, text: "What did Acme say?".into() }];
    let err = crate::ask::answer_detailed(&fx.s, "and the follow up?", "gemma-test", 10, &named, &routing).unwrap_err();
    assert!(matches!(err, crate::ApiError::ProviderFailure), "{err:?}");
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));

    // The function it wraps, with recording links in place of the routing-built chains: a hosted
    // answer leaves the client's material out, and says it answered on a hosted link.
    let hosted_log = Log::default();
    let (_, outcome, by) = crate::ask::answer_with_filter_by(
        &fx.s,
        crate::exclusion::context_filter(&fx.s),
        "what about the pricing?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, &hosted_log, "Hosted.")],
        || vec![],
    )
    .unwrap();
    assert_eq!(hosted_log.count(), 1);
    assert!(hosted_log.prompts()[0].contains(OTHER), "other material still answers");
    assert_clean(&hosted_log);
    assert_reached(&fx.c, "ask");
    assert!(matches!(outcome, ExclusionOutcome::FilteredHosted { left_out } if left_out >= 1), "{outcome:?}");
    assert!(!by.local);

    // A question naming the client never reaches the hosted link, even when one is offered.
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let (_, outcome, by) = crate::ask::answer_with_filter_by(
        &fx.s,
        crate::exclusion::context_filter(&fx.s),
        "What did Acme decide on pricing?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, &hosted_log, "Hosted.")],
        || vec![local(&local_log, "Local.")],
    )
    .unwrap();
    assert_eq!(outcome, ExclusionOutcome::LocalOnly);
    assert!(by.local);
    assert_eq!(hosted_log.count(), 0);
    assert_eq!(local_log.count(), 1);
}

#[test]
fn window_ask_never_sends_excluded_text() {
    let fx = fixture(true);
    let input = crate::window_ask::WindowInput {
        app_name: "Notes".into(),
        title: "Weekly notes".into(),
        text: "Remember to check the pricing.".into(),
        is_selection: false,
    };
    let prepared = crate::window_ask::prepare_window_ask(&fx.s, &input, "what about the pricing?", &[], &hosted_routing()).unwrap();
    assert!(!prepared.prompt.contains(CANARY), "{}", prepared.prompt);
    assert!(prepared.prompt.contains(OTHER));
    crate::ask::discard_ask(&prepared.prepare_id);

    // A window that names the client is refused for a hosted stream, never sent.
    let named = crate::window_ask::WindowInput { text: "From: Acme legal".into(), ..input.clone() };
    assert!(crate::window_ask::prepare_window_ask(&fx.s, &named, "summarise", &[], &hosted_routing()).is_err());
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));

    // The Mac's entry point (main's Limited Use settings per call): the same gate.
    let call = crate::routing::LimitedUseSettings::default();
    let prepared =
        crate::window_ask::prepare_window_ask_for(&fx.s, &input, "what about the pricing?", &[], &hosted_routing(), &call).unwrap();
    assert!(!prepared.prompt.contains(CANARY), "{}", prepared.prompt);
    assert!(prepared.prompt.contains(OTHER));
    crate::ask::discard_ask(&prepared.prepare_id);
    assert!(crate::window_ask::prepare_window_ask_for(&fx.s, &named, "summarise", &[], &hosted_routing(), &call).is_err());
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
}

#[test]
fn hotkey_destination_line_says_local_only() {
    let fx = fixture(true);
    let window = |text: &str| crate::window_ask::WindowInput {
        app_name: "Mail".into(),
        title: "Inbox".into(),
        text: text.into(),
        is_selection: false,
    };
    let routing = hosted_routing();
    assert_eq!(
        crate::window_ask::destination_for(&fx.s, &routing, "summarise", &[], &window("Roadrunner Co renewal terms")),
        crate::window_ask::Destination::LocalOnly
    );
    assert_eq!(
        crate::window_ask::destination_for(&fx.s, &routing, "what did Acme say?", &[], &window("Inbox")),
        crate::window_ask::Destination::LocalOnly
    );
    assert_eq!(
        crate::window_ask::destination_for(&fx.s, &routing, "summarise", &[], &window("Globex renewal terms")),
        crate::window_ask::Destination::Hosted { provider: "anthropic".into() }
    );
    // An Ollama cloud model leaves the computer: the line says so.
    let cloud = RoutingConfig { preferred_provider: "ollama".into(), preferred_model: "gpt-oss:120b-cloud".into(), ..RoutingConfig::default() };
    assert_eq!(crate::window_ask::destination(&cloud), crate::window_ask::Destination::Hosted { provider: "ollama".into() });
}

/// X6 final review, Major 6: a window that never names the client but carries a known client contact's
/// address (a mail thread from them) is the client's: local only, refused for a hosted stream.
#[test]
fn a_window_with_a_client_contacts_address_stays_local() {
    let fx = fixture(true);
    let sid = fx.c.query_row("SELECT id FROM topic_source WHERE ref_id = ?1 AND kind = 'meeting'", [fx.acme_meeting], |r| r.get::<_, i64>(0)).unwrap();
    fx.c.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
    let wile = fx.c.last_insert_rowid();
    fx.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", rusqlite::params![wile, sid]).unwrap();
    let window = |text: &str| crate::window_ask::WindowInput { app_name: "Mail".into(), title: "Inbox".into(), text: text.into(), is_selection: false };
    let mail = window("From: Wile E. <Wile@Coyote.example>\nSubject: renewal terms\n\nCan we talk about the renewal?");
    let routing = hosted_routing();
    assert_eq!(
        crate::window_ask::destination_for(&fx.s, &routing, "summarise", &[], &mail),
        crate::window_ask::Destination::LocalOnly
    );
    assert!(crate::exclusion::request_carries_client(
        &fx.s,
        "summarise",
        RequestOrigin::HotkeyWindow { window_text: "reply to wile@coyote.example. thanks".into() }
    ));
    assert!(crate::window_ask::prepare_window_ask(&fx.s, &mail, "summarise", &[], &routing).is_err(), "never sent");
    // Control: another sender's mail goes hosted.
    let other = window("From: Carol <carol@example.com>\nSubject: renewal terms");
    assert_eq!(
        crate::window_ask::destination_for(&fx.s, &routing, "summarise", &[], &other),
        crate::window_ask::Destination::Hosted { provider: "anthropic".into() }
    );
}

#[test]
fn request_naming_client_goes_local_only() {
    let fx = fixture(true);
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let (answer, outcome) = ask(&fx, "What did roadrunner co decide on pricing?", RequestOrigin::AskBar, &hosted_log, Some(&local_log)).unwrap();
    assert_eq!(outcome, ExclusionOutcome::LocalOnly);
    assert_eq!(answer.text, "A local answer.");
    assert_eq!(hosted_log.count(), 0);
    assert_eq!(local_log.count(), 1);
    assert!(local_log.prompts()[0].contains(CANARY), "on this computer the answer may use all of memory");
}

#[test]
fn refuses_without_local_model_and_never_falls_back() {
    let fx = fixture(true);
    let hosted_log = Log::default();
    let err = ask(&fx, "What did Acme decide?", RequestOrigin::AskBar, &hosted_log, None).unwrap_err();
    assert!(matches!(err, crate::ApiError::ProviderFailure));
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
    assert_eq!(hosted_log.count(), 0);
    // The blocking meeting Ask, too.
    let local_log = Log::default();
    assert!(crate::exclusion::request_carries_client(&fx.s, "anything", RequestOrigin::Meeting { meeting_id: fx.acme_meeting }));
    assert!(!crate::exclusion::request_carries_client(&fx.s, "anything", RequestOrigin::Meeting { meeting_id: fx.other_meeting }));
    let _ = local_log;
}

#[test]
fn question_from_client_page_is_local_only() {
    let fx = fixture(true);
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let (_, outcome) =
        ask(&fx, "what is still open?", RequestOrigin::TopicPage { key: fx.acme_key.clone() }, &hosted_log, Some(&local_log)).unwrap();
    assert_eq!(outcome, ExclusionOutcome::LocalOnly);
    assert_eq!(hosted_log.count(), 0);
    assert_eq!(local_log.count(), 1);
    // Another client's page goes hosted, without the canary.
    let other = fx.s.list_seed_topics().unwrap().into_iter().find(|t| t.name == "Globex").unwrap();
    let (_, outcome) = ask(&fx, "what about the pricing?", RequestOrigin::TopicPage { key: other.key }, &hosted_log, Some(&local_log)).unwrap();
    assert_ne!(outcome, ExclusionOutcome::LocalOnly);
    assert_eq!(hosted_log.count(), 1);
    assert_clean(&hosted_log);
}

#[test]
fn hosted_prompt_never_names_client_or_reason() {
    let fx = fixture(true);
    let log = Log::default();
    let (_, outcome) = ask(&fx, "what about the pricing?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(matches!(outcome, ExclusionOutcome::FilteredHosted { .. }));
    assert_eq!(log.count(), 1);
    assert_clean(&log);
    let p = log.prompts()[0].to_lowercase();
    for word in ["client", "withheld", "omitted", "removed"] {
        assert!(!p.contains(word), "the prompt says why: {word}");
    }
}

#[test]
fn substring_term_does_not_exclude_unrelated_text() {
    let dir = tempfile::tempdir().unwrap();
    let s = crate::test_store::open(&dir.path().join("m.db"), dir.path());
    let ann = s.create_seed_topic("Ann", &[], None).unwrap();
    s.set_client_control(&ann.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    let filter = crate::exclusion::context_filter(&s);
    assert!(!filter.names_client("The Annual report is due"));
    assert!(filter.names_client("Lunch with Ann on Friday"));
    assert!(!crate::exclusion::request_carries_client(&s, "Where is the Annual report?", RequestOrigin::AskBar));
    assert!(crate::exclusion::request_carries_client(&s, "What did Ann say?", RequestOrigin::AskBar));
    assert!(!crate::exclusion::classify_before_hosted(&s, "Annual planning, Annabel", &[]));
}

#[test]
fn shared_source_excluded_if_either_client_excluded() {
    let fx = fixture(true);
    let globex = fx.s.list_seed_topics().unwrap().into_iter().find(|t| t.name == "Globex").unwrap();
    let shared = fx.c.query_row("SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1", [fx.acme_meeting], |r| r.get::<_, i64>(0)).unwrap();
    fx.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", rusqlite::params![globex.topic_id, shared])
        .unwrap();
    let ids = crate::exclusion::excluded_source_ids(&fx.s).unwrap();
    assert!(ids.contains(&shared));
    let other = fx.c.query_row("SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1", [fx.other_meeting], |r| r.get::<_, i64>(0)).unwrap();
    assert!(!ids.contains(&other));
    // Asked from the other client's page, the shared meeting still stays out of a hosted prompt.
    let log = Log::default();
    ask(&fx, "what about the pricing?", RequestOrigin::TopicPage { key: globex.key }, &log, None).unwrap();
    assert_clean(&log);
}

#[test]
fn unreadable_setting_fails_closed() {
    let fx = fixture(true);
    fx.c.execute("DROP TABLE client_control", []).unwrap();
    assert!(crate::exclusion::context_filter(&fx.s).is_closed());
    assert!(crate::exclusion::request_carries_client(&fx.s, "the weather", RequestOrigin::AskBar));
    assert!(crate::exclusion::classify_before_hosted(&fx.s, "the weather", &[]));
    let hosted_log = Log::default();
    assert!(ask(&fx, "the weather", RequestOrigin::AskBar, &hosted_log, None).is_err());
    assert_eq!(hosted_log.count(), 0);
    let local_log = Log::default();
    let (_, outcome) = ask(&fx, "the weather", RequestOrigin::AskBar, &hosted_log, Some(&local_log)).unwrap();
    assert_eq!(outcome, ExclusionOutcome::LocalOnly);
    assert_eq!(hosted_log.count(), 0);
    assert!(crate::exclusion::excluded_source_ids(&fx.s).unwrap().len() >= 2, "every source");
}

// ---- Meetings: summary, classify first, speakers, prep ---------------------------------

/// A meeting filed under no topic, whose first line names the client and whose second (the one
/// the question finds) holds the canary: the whole meeting stays out of a hosted prompt.
#[test]
fn a_meeting_naming_the_client_anywhere_is_left_out_whole() {
    let fx = fixture(true);
    meeting(
        &fx.s,
        "Catch-up",
        &[],
        crate::util::now_unix_ms() - 1_800_000,
        &["Thanks for joining from Acme.", &format!("The {CANARY} budget was approved.")],
        "",
    );
    let log = Log::default();
    ask(&fx, "what about the budget?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_eq!(log.count(), 1);
    assert_clean(&log);
}



fn summary_reply() -> &'static str {
    r#"{"tldr": "ok", "decisions": [], "action_items": [], "open_questions": []}"#
}

#[test]
fn summary_never_sends_excluded_text() {
    let fx = fixture(true);
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let transcript = format!("We agreed the {CANARY} pricing for the launch.");
    let out = crate::summary::summarize_meeting_routed(
        &fx.s,
        fx.acme_meeting,
        "",
        &transcript,
        "",
        &RoutingConfig::default(),
        "",
        || vec![hosted(&fx.s, &hosted_log, summary_reply())],
        || vec![local(&local_log, summary_reply())],
    )
    .unwrap();
    assert_eq!(out.provider, llm::Provider::Ollama);
    assert_eq!(hosted_log.count(), 0);
    assert_eq!(local_log.count(), 1);

    // Carried requirement (b): a long transcript naming the client only in a later chunk is
    // never fanned out chunk by chunk to each hosted link in turn.
    let max = llm::max_transcript_chars_per_chunk();
    let filler = "word ".repeat(max / 5 + 10);
    let long = format!("{filler}\n{filler}\nand then Acme came up.");
    let (a, b) = (Log::default(), Log::default());
    let r = crate::summary::summarize_meeting_with(
        &fx.s,
        fx.other_meeting,
        "",
        &long,
        "",
        &RoutingConfig::default(),
        "",
        vec![hosted(&fx.s, &a, summary_reply()), hosted(&fx.s, &b, summary_reply())],
        false,
    );
    assert!(r.is_err());
    assert_eq!((a.count(), b.count()), (0, 0), "no chunk reached either hosted link");
    // The other client's meeting is summarised by the hosted link as before.
    let ok = Log::default();
    crate::summary::summarize_meeting_routed(
        &fx.s,
        fx.other_meeting,
        "",
        &format!("The {OTHER} pricing review is next week."),
        "",
        &RoutingConfig::default(),
        "",
        || vec![hosted(&fx.s, &ok, summary_reply())],
        Vec::new,
    )
    .unwrap();
    assert_eq!(ok.count(), 1);
    assert_clean(&ok);
    assert_reached(&fx.c, "meeting_summary");
}

#[test]
fn classify_first_new_meeting_for_excluded_client_makes_no_hosted_call() {
    let fx = fixture(true);
    // Not filed under any topic yet: only its title names the client.
    let new = meeting(&fx.s, "Acme kickoff", &[], crate::util::now_unix_ms(), &["Hello everyone."], "");
    let (hosted_log, local_log) = (Log::default(), Log::default());
    crate::summary::summarize_meeting_routed(
        &fx.s,
        new,
        "",
        "Hello everyone.",
        "",
        &RoutingConfig::default(),
        "",
        || vec![hosted(&fx.s, &hosted_log, summary_reply())],
        || vec![local(&local_log, summary_reply())],
    )
    .unwrap();
    assert_eq!(hosted_log.count(), 0, "no hosted call before filing");
    assert_eq!(local_log.count(), 1);
    // No local model: not made, and still nothing sent.
    let r = crate::summary::summarize_meeting_routed(
        &fx.s,
        new,
        "",
        "Hello everyone.",
        "",
        &RoutingConfig::default(),
        "",
        || vec![hosted(&fx.s, &hosted_log, summary_reply())],
        Vec::new,
    );
    assert!(r.is_err());
    assert_eq!(hosted_log.count(), 0);

    // By attendee address: a person met only in the client's material.
    let sid = fx.c.query_row("SELECT id FROM topic_source WHERE ref_id = ?1 AND kind = 'meeting'", [fx.acme_meeting], |r| r.get::<_, i64>(0)).unwrap();
    fx.c.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
    let wile = fx.c.last_insert_rowid();
    fx.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", rusqlite::params![wile, sid]).unwrap();
    assert!(crate::exclusion::classify_before_hosted(&fx.s, "Kickoff", &["Wile E <wile@coyote.example>".to_string()]));
    assert!(!crate::exclusion::classify_before_hosted(&fx.s, "Kickoff", &["carol@example.com".to_string()]));
    // Someone also met in another client's material is not the client's contact.
    let other = fx.c.query_row("SELECT id FROM topic_source WHERE ref_id = ?1 AND kind = 'meeting'", [fx.other_meeting], |r| r.get::<_, i64>(0)).unwrap();
    fx.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", rusqlite::params![wile, other]).unwrap();
    assert!(!crate::exclusion::classify_before_hosted(&fx.s, "Kickoff", &["wile@coyote.example".to_string()]));
}

#[test]
fn prep_never_sends_excluded_text() {
    // Control: with nothing excluded, Bob's brief does carry the canary to a hosted model.
    let open = fixture(false);
    open.s
        .insert_routine(&memory::NewRoutine {
            name: "Meeting Prep".into(),
            template: memory::RoutineTemplate::MeetingPrep,
            schedule_hhmm: "00:00".into(),
            enabled: true,
        })
        .unwrap();
    let now = crate::util::now_unix_ms();
    let planning = crate::connectors::calendar::QualifyingMeeting {
        id: "evt-0".into(),
        title: "Planning".into(),
        start_unix_ms: now + 20 * 60_000,
        end_unix_ms: now + 50 * 60_000,
        attendees: vec!["Bob <bob@example.com>".into()],
        attendee_details: Vec::new(),
        conference_link: None,
        source: crate::connectors::calendar::MeetingSource::GoogleCalendar,
        app_bundle_id: None,
        organizer_email: None,
        organizer_name: None,
        self_is_organizer: false,
        self_response: None,
    };
    let seen: std::cell::RefCell<Vec<(String, bool)>> = Default::default();
    crate::meeting_prep::run_gated_with(&open.s, &[planning], now, 30.0, "", "", crate::routine_runs::RunGate::Full, &mut |p, l| {
        seen.borrow_mut().push((p.to_string(), l));
        None
    })
    .unwrap();
    assert!(seen.borrow().iter().any(|(p, l)| !l && p.contains(CANARY)), "the fixture reaches the brief");

    let fx = fixture(true);
    fx.s.insert_routine(&memory::NewRoutine {
        name: "Meeting Prep".into(),
        template: memory::RoutineTemplate::MeetingPrep,
        schedule_hhmm: "00:00".into(),
        enabled: true,
    })
    .unwrap();
    let now = crate::util::now_unix_ms();
    let upcoming = |id: &str, title: &str, who: &str| crate::connectors::calendar::QualifyingMeeting {
        id: id.into(),
        title: title.into(),
        start_unix_ms: now + 20 * 60_000,
        end_unix_ms: now + 50 * 60_000,
        attendees: vec![who.into()],
        attendee_details: Vec::new(),
        conference_link: None,
        source: crate::connectors::calendar::MeetingSource::GoogleCalendar,
        app_bundle_id: None,
        organizer_email: None,
        organizer_name: None,
        self_is_organizer: false,
        self_response: None,
    };
    // Bob attended the client's meeting: his earlier meeting never reaches a hosted brief.
    let meetings = [upcoming("evt-1", "Planning", "Bob <bob@example.com>"), upcoming("evt-2", "Acme renewal", "Carol <carol@example.com>")];
    let calls: std::cell::RefCell<Vec<(String, bool)>> = Default::default();
    let mut complete = |prompt: &str, local_only: bool| {
        calls.borrow_mut().push((prompt.to_string(), local_only));
        None
    };
    crate::meeting_prep::run_gated_with(&fx.s, &meetings, now, 30.0, "", "", crate::routine_runs::RunGate::Full, &mut complete).unwrap();
    let calls = calls.into_inner();
    let hosted_log = Log::default();
    for (prompt, local_only) in &calls {
        if !local_only {
            hosted_log.0.lock().unwrap().push(prompt.clone());
        }
    }
    assert!(calls.iter().any(|(p, l)| *l && p.contains("Acme renewal")), "the client's meeting is prepared on this computer");
    assert_clean(&hosted_log);
}

/// The cloud runner's prep: a substituted transport (`meeting_prep::run_with_chain_transport`) is only
/// the client behind each link of routing's chain, so the gate and the filter still apply.
#[test]
fn prep_through_a_transport_never_sends_excluded_text() {
    let routing = RoutingConfig {
        preferred_provider: "anthropic".into(),
        anthropic_key: Some("k".into()),
        use_local_model: false,
        ..RoutingConfig::default()
    };
    let prep_with = |excluded: bool, log: &Log| {
        let fx = fixture(excluded);
        fx.s.insert_routine(&memory::NewRoutine {
            name: "Meeting Prep".into(),
            template: memory::RoutineTemplate::MeetingPrep,
            schedule_hhmm: "00:00".into(),
            enabled: true,
        })
        .unwrap();
        let now = crate::util::now_unix_ms();
        let upcoming = |id: &str, title: &str, who: &str| crate::connectors::calendar::QualifyingMeeting {
            id: id.into(),
            title: title.into(),
            start_unix_ms: now + 20 * 60_000,
            end_unix_ms: now + 50 * 60_000,
            attendees: vec![who.into()],
            source: crate::connectors::calendar::MeetingSource::Calendar,
            ..Default::default()
        };
        let meetings = [
            upcoming("evt-1", "Planning", "Bob <bob@example.com>"),
            upcoming("evt-2", "Acme renewal", "Carol <carol@example.com>"),
            upcoming("evt-3", "Review", "Carol <carol@example.com>"),
        ];
        let log = log.clone();
        let transport = move |_: llm::Provider, _: &str| -> Box<dyn llm::LlmClient> { Box::new(Recorder { log: log.clone(), reply: "not json".into() }) };
        crate::meeting_prep::run_with_chain_transport(&fx.s, &meetings, now, 30.0, "", &routing, crate::routine_runs::RunGate::Full, Some(&transport))
            .unwrap();
    };
    // Control: with nothing excluded, the canary reaches the hosted transport.
    let open = Log::default();
    prep_with(false, &open);
    assert!(open.prompts().iter().any(|p| p.contains(CANARY)), "the fixture reaches the brief");

    let log = Log::default();
    prep_with(true, &log);
    assert!(log.prompts().iter().any(|p| p.contains(OTHER)), "another client's brief still goes to the hosted model");
    assert!(log.prompts().iter().all(|p| !p.contains("Acme renewal")), "the client's meeting has no hosted brief");
    assert_clean(&log);
}

#[test]
fn meeting_ask_and_titles_never_send_excluded_text() {
    let fx = fixture(true);
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let answer = crate::ask::answer_meeting_with(
        &fx.s,
        fx.acme_meeting,
        "what was agreed?",
        &[],
        &RoutingConfig::default(),
        || vec![hosted(&fx.s, &hosted_log, "Hosted.")],
        || vec![local(&local_log, "Local.")],
    )
    .unwrap();
    assert_eq!(answer.text, "Local.");
    assert_eq!((hosted_log.count(), local_log.count()), (0, 1));
    // No local model: refused, never hosted.
    let r = crate::ask::answer_meeting_with(&fx.s, fx.acme_meeting, "what was agreed?", &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &hosted_log, "Hosted.")], Vec::new);
    assert!(r.is_err());
    // The streaming prepare refuses a hosted stream of the client's meeting.
    assert!(crate::ask::prepare_meeting_ask(&fx.s, fx.acme_meeting, "what was agreed?", &[], &hosted_routing()).is_err());
    // Another client's meeting goes hosted.
    crate::ask::answer_meeting_with(&fx.s, fx.other_meeting, "what was agreed?", &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &hosted_log, "Hosted.")], Vec::new)
        .unwrap();
    assert_eq!(hosted_log.count(), 1);
    assert_clean(&hosted_log);

    // Conversation titles: a first exchange naming the client is titled locally.
    let (h, l) = (Log::default(), Log::default());
    crate::ask::generate_conversation_title_with(&fx.s, "What did Acme decide?", "They agreed.", &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Title")], || vec![local(&l, "Title")]).unwrap();
    assert_eq!((h.count(), l.count()), (0, 1));
    // Control: an answer made clean for the current clients is titled by the hosted chain.
    mark_clean_no_inputs(&fx, "Friday.");
    crate::ask::generate_conversation_title_with(&fx.s, "Lunch plans?", "Friday.", &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Title")], || vec![local(&l, "Title")]).unwrap();
    assert_eq!(h.count(), 1);
    assert_clean(&h);
    assert_reached(&fx.c, "ask");
    assert_reached(&fx.c, "conversation_title");
    // Fix round 2, N7: an answer from before the client was excluded is titled locally.
    let before = l.count();
    crate::ask::generate_conversation_title_with(&fx.s, "Dinner plans?", "Saturday.", &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Title")], || vec![local(&l, "Title")]).unwrap();
    assert_eq!((h.count(), l.count()), (1, before + 1));
}

/// A hosted link of `provider` recording into `log`, behind the gate production uses.
fn hosted_as(store: &memory::Store, provider: llm::Provider, log: &Log, reply: &str) -> llm::ChainLink {
    llm::guard_hosted(
        llm::ChainLink { provider, model: "hosted-test".into(), client: Box::new(Recorder { log: log.clone(), reply: reply.into() }) },
        crate::exclusion::current_gate(store),
    )
}

/// A meeting of `n` lines of about 1,000 characters each, every line carrying `word`, filed
/// under `topic`: over a local model's and OpenRouter's meeting Ask window.
fn long_meeting_under(fx: &Fx, topic: i64, word: &str, n: usize, ago_ms: i64) -> i64 {
    let lines: Vec<String> = (0..n).map(|i| format!("LINE{i} the {word} plan {}", "x".repeat(980))).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let id = meeting(&fx.s, "Planning", &[], crate::util::now_unix_ms() - ago_ms, &refs, "");
    file(&fx.c, topic, "meeting", id);
    id
}

/// Merge 3 (main's #816): a whole-meeting question on a long transcript reads it part by part
/// before answering (`gather_findings`). The parts of an excluded client's meeting are read on
/// this computer only, blocking and streamed; another client's parts go to the hosted link,
/// behind the gate. The streamed path's link is built by `streamed_link`: an on-device chain when
/// the request carries the client.
#[test]
fn meeting_ask_map_step_never_sends_excluded_text() {
    let fx = fixture(true);
    let globex = fx.c.query_row("SELECT id FROM topic WHERE name = 'Globex'", [], |r| r.get::<_, i64>(0)).unwrap();
    let acme_long = long_meeting_under(&fx, fx.acme_topic, CANARY, 120, 20_000_000);
    let other_long = long_meeting_under(&fx, globex, OTHER, 120, 30_000_000);
    let question = "list the action items";

    // Blocking: the client's meeting is read part by part on the local link; nothing hosted.
    let (hosted_log, local_log) = (Log::default(), Log::default());
    crate::ask::answer_meeting_with(
        &fx.s,
        acme_long,
        question,
        &[],
        &RoutingConfig::default(),
        || vec![hosted_as(&fx.s, llm::Provider::OpenRouter, &hosted_log, "Hosted.")],
        || vec![local(&local_log, "Local.")],
    )
    .unwrap();
    assert_eq!(hosted_log.count(), 0);
    assert!(local_log.count() >= 3, "parts then the answer: {}", local_log.count());
    assert!(local_log.prompts()[0].contains("part 1 of"), "the map step ran");

    // Streamed, local stream: the parts are read on the on-device link `link_for` builds for a
    // request that carries the client.
    let parts_log = Log::default();
    let prepared = crate::ask::prepare_meeting_ask_with(&fx.s, acme_long, question, &[], &RoutingConfig::default(), |_, carries| {
        assert!(carries, "the client's meeting is read on this computer");
        Some(local(&parts_log, "Finding."))
    })
    .unwrap_or_else(|_| panic!("a local stream of the client's meeting is prepared"));
    assert!(parts_log.count() >= 2 && parts_log.prompts()[0].contains("part 1 of"));
    crate::ask::discard_ask(&prepared.prepare_id);
    // Streamed, hosted stream: refused before any part is read.
    let openrouter = RoutingConfig { preferred_provider: "openrouter".into(), openrouter_key: Some("k".into()), ..RoutingConfig::default() };
    let refused = crate::ask::prepare_meeting_ask_with(&fx.s, acme_long, question, &[], &openrouter, |_, _| -> Option<llm::ChainLink> {
        panic!("no link is built for a refused stream")
    });
    assert!(refused.is_err());

    // Control: another client's long meeting is read part by part by the hosted link.
    let control = Log::default();
    crate::ask::answer_meeting_with(
        &fx.s,
        other_long,
        question,
        &[],
        &RoutingConfig::default(),
        || vec![hosted_as(&fx.s, llm::Provider::OpenRouter, &control, "Hosted.")],
        Vec::new,
    )
    .unwrap();
    assert!(control.count() >= 3, "parts then the answer: {}", control.count());
    assert!(control.prompts().iter().any(|p| p.contains(OTHER)));
    assert_clean(&control);
    let streamed = Log::default();
    let prepared = crate::ask::prepare_meeting_ask_with(&fx.s, other_long, question, &[], &openrouter, |provider, carries| {
        assert!(!carries);
        Some(hosted_as(&fx.s, provider, &streamed, "Finding."))
    })
    .unwrap_or_else(|_| panic!("another client's meeting streams hosted"));
    assert!(streamed.count() >= 2);
    assert!(prepared.prompt.contains("Finding."), "the stream carries the findings");
    assert_clean(&streamed);
    crate::ask::discard_ask(&prepared.prepare_id);
    assert_reached(&fx.c, "ask");

    // A long meeting filed under no excluded client whose transcript names the client only in its
    // last part: no part is sent to a hosted link, blocking or streamed. (The stored transcript
    // already makes the meeting the client's material, so the request carries it; the part-by-part
    // reading also checks every part before the first on a hosted link: see ask.rs
    // `gather_findings_checks_every_part_before_the_first_hosted_call`.)
    let late_lines: Vec<String> = (0..120)
        .map(|i| if i == 119 { "LINE119 and then Acme came up.".to_string() } else { format!("LINE{i} the {OTHER} plan {}", "x".repeat(980)) })
        .collect();
    let refs: Vec<&str> = late_lines.iter().map(String::as_str).collect();
    let late = meeting(&fx.s, "Planning", &[], crate::util::now_unix_ms() - 40_000_000, &refs, "");
    let (h1, h2) = (Log::default(), Log::default());
    let r = crate::ask::answer_meeting_with(
        &fx.s,
        late,
        question,
        &[],
        &RoutingConfig::default(),
        || vec![hosted_as(&fx.s, llm::Provider::OpenRouter, &h1, "Hosted."), hosted_as(&fx.s, llm::Provider::OpenRouter, &h2, "Hosted.")],
        Vec::new,
    );
    assert!(r.is_err());
    assert_eq!((h1.count(), h2.count()), (0, 0), "no part reached either hosted link");
    let h3 = Log::default();
    let streamed = crate::ask::prepare_meeting_ask_with(&fx.s, late, question, &[], &openrouter, |provider, _| {
        Some(hosted_as(&fx.s, provider, &h3, "Finding."))
    });
    assert_eq!(h3.count(), 0, "no part reached the hosted stream's link");
    if let Ok(p) = streamed {
        assert!(!p.prompt.to_lowercase().contains("acme"));
        crate::ask::discard_ask(&p.prepare_id);
    }
}

/// Merge 3 (main's "Re-summarize with Whimbrel"): a forced re-summary is decided by the
/// exclusion gate first. An excluded client's meeting is refused (never forced onto the hosted
/// link, and no misleading sign-in reason); another meeting goes to the preferred link alone.
#[test]
fn forced_resummary_never_sends_excluded_text() {
    let fx = fixture(true);
    let whimbrel = RoutingConfig {
        preferred_provider: "whimbrel".into(),
        whimbrel_id_token: Some("tok".into()),
        whimbrel_device_key: Some("key".into()),
        ..RoutingConfig::default()
    };
    let transcript = format!("We agreed the {CANARY} pricing for the launch.");
    let (hosted_log, local_log) = (Log::default(), Log::default());
    let r = crate::summary::summarize_meeting_routed_as(
        &fx.s,
        fx.acme_meeting,
        "",
        &transcript,
        "",
        &whimbrel,
        "",
        true,
        || vec![hosted_as(&fx.s, llm::Provider::Whimbrel, &hosted_log, summary_reply())],
        || vec![local(&local_log, summary_reply())],
    );
    assert!(r.is_err());
    assert_eq!((hosted_log.count(), local_log.count()), (0, 0));
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
    assert_eq!(crate::summary::take_last_summary_failure_reason(), None);
    // The public entry point: the same refusal, before any network (no on-device Whimbrel link).
    let r = crate::summary::summarize_meeting_preferred(&fx.s, fx.acme_meeting, "", &transcript, "", "", &whimbrel, "");
    assert!(r.is_err());
    assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));

    // Control: another client's meeting goes to the preferred hosted link, and only to it.
    let (ok, fallback) = (Log::default(), Log::default());
    let out = crate::summary::summarize_meeting_routed_as(
        &fx.s,
        fx.other_meeting,
        "",
        &format!("The {OTHER} pricing review is next week."),
        "",
        &whimbrel,
        "",
        true,
        || vec![hosted_as(&fx.s, llm::Provider::Whimbrel, &ok, summary_reply()), local(&fallback, summary_reply())],
        Vec::new,
    )
    .unwrap();
    assert_eq!(out.provider, llm::Provider::Whimbrel);
    assert_eq!((ok.count(), fallback.count()), (1, 0));
    assert_clean(&ok);
    assert_reached(&fx.c, "meeting_summary");
}

/// An excluded client named only in the title of a Google Calendar meeting (its transcript never
/// names it): the exclusion decision must not depend on Limited Use. A link Limited Use allows
/// (Anthropic) and one it hides the title from (OpenRouter) both get neither the transcript nor
/// the name, streamed and blocking.
#[test]
fn a_google_calendar_title_naming_the_client_keeps_the_meeting_off_every_hosted_link() {
    let build = |excluded: bool| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let s = crate::test_store::open(&path, dir.path());
        let acme = s.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
        let id = meeting(&s, "Acme pricing sync", &["Bob <bob@example.com>"], crate::util::now_unix_ms() - 3_600_000, &[&format!("We agreed the {CANARY} pricing.")], &format!("Agreed the {CANARY} pricing."));
        assert!(s.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_GOOGLE).unwrap());
        if excluded {
            s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        }
        (dir, s)
    };
    let routing = |provider: &str| {
        let mut r = RoutingConfig { preferred_provider: provider.into(), ..RoutingConfig::default() };
        match provider {
            "openrouter" => r.openrouter_key = Some("k".into()),
            _ => r.anthropic_key = Some("k".into()),
        }
        r
    };
    let link = |s: &memory::Store, log: &Log, provider: llm::Provider| {
        let mut l = hosted(s, log, "An answer.");
        l.provider = provider;
        l
    };

    // Control: with nothing excluded, a stream to the link Limited Use allows carries the transcript.
    let (_d, open) = build(false);
    let data = crate::ask::prepare_ask(&open, "what about the pricing?", 10, &[], &routing("anthropic")).unwrap();
    crate::ask::discard_ask(&data.prepare_id);
    assert!(data.prompt.contains(CANARY), "the fixture reaches retrieval:\n{}", data.prompt);

    let (_d, s) = build(true);
    for provider in ["anthropic", "openrouter"] {
        match crate::ask::prepare_ask(&s, "what about the pricing?", 10, &[], &routing(provider)) {
            Ok(data) => {
                crate::ask::discard_ask(&data.prepare_id);
                let lower = data.prompt.to_lowercase();
                assert!(!lower.contains(CANARY) && !lower.contains("acme"), "{provider} stream carried it:\n{}", data.prompt);
            }
            Err(_) => {} // refused: also keeps it off the link
        }
    }
    let (anthropic_log, openrouter_log) = (Log::default(), Log::default());
    let _ = crate::ask::answer_with(
        &s,
        "what about the pricing?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![link(&s, &openrouter_log, llm::Provider::OpenRouter), link(&s, &anthropic_log, llm::Provider::Anthropic)],
        Vec::new,
    );
    assert_clean(&anthropic_log);
    assert_clean(&openrouter_log);
}

#[test]
fn live_question_never_sends_excluded_text() {
    let fx = fixture(true);
    let (h, l) = (Log::default(), Log::default());
    let reply = r#"{"is_for_user":false,"confidence":"low","quote":"q","asker":null,"restatement":"r"}"#;
    crate::live_question::detect_with(&fx.s, "Mike, what does Acme want?", &RequestOrigin::Other, || vec![hosted(&fx.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!((h.count(), l.count()), (0, 1));
    crate::live_question::detect_with(&fx.s, "Mike, can you ship Friday?", &RequestOrigin::Other, || vec![hosted(&fx.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!(h.count(), 1);
    assert_clean(&h);
    assert_reached(&fx.c, "live_question");
}

#[test]
fn speaker_names_never_send_excluded_text() {
    let fx = fixture(true);
    let invitees = [
        memory::speaker_names::Attendee { name: "Ann Lee".into(), email: None },
        memory::speaker_names::Attendee { name: "Bo Diaz".into(), email: None },
    ];
    fx.s.set_meeting_attendee_details(fx.acme_meeting, &invitees).unwrap();
    let (h, l) = (Log::default(), Log::default());
    let reply = r#"{"speakers": []}"#;
    crate::speaker_names::assign_meeting_speakers_with(
        &fx.s,
        fx.acme_meeting,
        &[(0, vec![1.0, 0.0]), (1, vec![0.0, 1.0])],
        &RoutingConfig::default(),
        false,
        || vec![hosted(&fx.s, &h, reply)],
        || vec![local(&l, reply)],
    )
    .unwrap();
    assert_eq!(h.count(), 0, "the client's meeting is never named by a hosted model");
    assert_eq!(l.count(), 1);
}

#[test]
fn title_check_never_sends_excluded_text() {
    struct Matcher(std::cell::Cell<usize>);
    impl llm::typesafe::TitleMatcher for Matcher {
        fn probability(&self, _input: &llm::typesafe::TitleMatchInput) -> Result<f64, llm::LlmError> {
            self.0.set(self.0.get() + 1);
            Ok(0.9)
        }
    }
    let fx = fixture(true);
    let summary = llm::MeetingSummary {
        title: String::new(),
        tldr: format!("Agreed the {CANARY} pricing."),
        decisions: vec![],
        action_items: vec![],
        action_items_detail: vec![],
        open_questions: vec![],
        token_usage: llm::TokenUsage::default(),
    };
    let m = Matcher(std::cell::Cell::new(0));
    crate::title_check::check_calendar_title_with(&fx.s, fx.acme_meeting, &summary, "Pricing", &m);
    assert_eq!(m.0.get(), 0, "TypeSafe never sees the client's meeting");
    // Control: the other client's meeting is checked.
    let other = llm::MeetingSummary { tldr: format!("{OTHER} pricing review."), ..summary };
    crate::title_check::check_calendar_title_with(&fx.s, fx.other_meeting, &other, "Pricing", &m);
    assert_eq!(m.0.get(), 1);
}

#[test]
fn ollama_cloud_models_are_treated_as_hosted() {
    let fx = fixture(true);
    let gate = crate::exclusion::current_gate(&fx.s);
    let routing = RoutingConfig::default();
    // The Ollama link of a cloud model is behind the gate.
    let chain = crate::routing::build_chain("gpt-oss:120b-cloud", &routing, &gate);
    let ollama = chain.iter().find(|l| l.provider == llm::Provider::Ollama).unwrap();
    assert!(matches!(ollama.client.complete("the Acme call"), Err(llm::LlmError::Refused(_))));
    assert!(crate::exclusion::chain_is_hosted(&chain));
    // A cloud model is never a local link, in local-only mode either.
    assert!(crate::routing::build_local_chain("gpt-oss:120b-cloud", &routing).iter().all(|l| l.provider != llm::Provider::Ollama));
    let sensitive = RoutingConfig { sensitive: true, ..RoutingConfig::default() };
    assert!(crate::routing::build_chain("gpt-oss:120b-cloud", &sensitive, &gate).iter().all(|l| l.provider != llm::Provider::Ollama));
    // An on-device model is unchanged.
    assert!(!crate::exclusion::chain_is_hosted(&crate::routing::build_local_chain("gemma4:12b", &routing)));
    // A stream prepared for Ollama with a cloud model counts as hosted.
    let cloud = RoutingConfig { preferred_provider: "ollama".into(), preferred_model: "kimi-k2:1t-cloud".into(), ..RoutingConfig::default() };
    assert!(crate::exclusion::streamed_is_hosted(&cloud));
    assert!(crate::ask::prepare_ask(&fx.s, "What did Acme decide?", 5, &[], &cloud).is_err());
}

// ---- Day-level callers: digest, routines, profile suggestions --------------------------

#[test]
fn digest_never_sends_excluded_text() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let log = Log::default();
    let reply = r#"{"summary": "A day.", "open_loops": []}"#;
    crate::digest::daily_digest_with(&fx.s, now - 86_400_000, now, &RoutingConfig::default(), vec![hosted(&fx.s, &log, reply)]).unwrap();
    assert_eq!(log.count(), 1);
    assert!(log.prompts()[0].contains(OTHER), "the rest of the day is still digested");
    assert_clean(&log);
    assert_reached(&fx.c, "digest");
}

#[test]
fn routines_never_send_excluded_text() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let log = Log::default();
    let chain = vec![hosted(&fx.s, &log, r#"{"summary": "A day.", "open_loops": []}"#)];
    let filter = crate::exclusion::filter_for_chain(&fx.s, &chain);
    assert!(filter.is_active(), "a chain that can reach a hosted model is filtered");
    routines::run_daily_brief(&fx.s, chain[0].client.as_ref(), &filter, now - 86_400_000, now, &[], now, "", "").unwrap();
    routines::run_eod_recap(&fx.s, chain[0].client.as_ref(), &filter, now - 86_400_000, now, now, false, None, "", "").unwrap();
    routines::run_weekly_digest(&fx.s, chain[0].client.as_ref(), &filter, now - 7 * 86_400_000, now, now, false, None, "").unwrap();
    assert_eq!(log.count(), 3);
    assert_clean(&log);
    // An all-local chain may use everything.
    assert!(!crate::exclusion::filter_for_chain(&fx.s, &[local(&Log::default(), "")]).is_active());
}

/// X6 final review, Major 7: the scheduler's own tick, not only the routine functions: each of the daily brief,
/// EOD recap and weekly digest reaches a hosted model without the excluded client's material, and the daily
/// brief stores none of its calendar events. Fails if any of the three `filter_for_chain` calls in
/// `routine_runs::tick` or its `events_without_excluded` is dropped.
#[test]
fn the_routine_tick_never_sends_or_stores_excluded_material() {
    let fx = fixture(true);
    let at = |d: u32, h: u32| {
        use chrono::TimeZone;
        chrono::Local.with_ymd_and_hms(2026, 8, d, h, 0, 0).unwrap().timestamp_millis()
    };
    // Saturday (the daily brief's "yesterday" and the weekly window) and Sunday (the EOD recap's day).
    for day in [1, 2] {
        let acme = meeting(&fx.s, "Pricing call", &[], at(day, 14), &[&format!("The {CANARY} pricing, day {day}.")], &format!("{CANARY} pricing."));
        file(&fx.c, fx.acme_topic, "meeting", acme);
        meeting(&fx.s, "Team call", &[], at(day, 15), &[&format!("The {OTHER} review, day {day}.")], &format!("{OTHER} review."));
    }
    fx.s.seed_builtin_routines().unwrap();
    for r in fx.s.list_routines().unwrap() {
        let on = matches!(
            r.template,
            memory::RoutineTemplate::DailyBrief | memory::RoutineTemplate::EodRecap | memory::RoutineTemplate::WeeklyDigest
        );
        fx.s.set_routine_enabled(r.id, on).unwrap();
    }
    let log = Log::default();
    let recorder = log.clone();
    crate::routine_runs::TEST_CHAIN.with(|c| {
        *c.borrow_mut() = Some(Box::new(move |s: &memory::Store| vec![hosted(s, &recorder, r#"{"summary": "A day.", "open_loops": []}"#)]))
    });
    let events = [
        routines::CalendarEventSummary { id: None, title: "Acme kickoff".into(), start_unix_ms: at(2, 16) },
        routines::CalendarEventSummary { id: None, title: "Dentist".into(), start_unix_ms: at(2, 17) },
    ];
    let runs = crate::routine_runs::tick(&fx.s, at(2, 23), "gemma4:12b", &RoutingConfig::default(), &events, false, None, true);
    crate::routine_runs::TEST_CHAIN.with(|c| *c.borrow_mut() = None);
    let runs = runs.unwrap();
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert_eq!(log.count(), 3);
    for p in log.prompts() {
        assert!(p.contains(OTHER), "each routine gathered its window: {p}");
    }
    assert_clean(&log);
    for run in &runs {
        let stored = format!("{} {}", run.output_summary, run.output_json).to_lowercase();
        assert!(!stored.contains("acme") && !stored.contains(CANARY), "a stored run carries the client: {stored}");
    }
    assert!(runs.iter().any(|r| r.output_summary.contains("Dentist")), "control: the brief lists today's other event");
}

#[test]
fn profile_suggest_never_sends_excluded_text() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let log = Log::default();
    // Both Limited Use forms of the prompt (main's per-link choice) are checked.
    let mut complete = |p: &crate::profile_suggest::SuggestPrompts| {
        log.0.lock().unwrap().push(p.full.clone());
        log.0.lock().unwrap().extend(p.without_google.clone());
        Some(r#"{"suggestions": []}"#.to_string())
    };
    crate::profile_suggest::run_with_prompts(&fx.s, now, "2026-11-12", now - 86_400_000, now + 1, &[], "", "", crate::routine_runs::RunGate::Full, &mut complete)
        .unwrap();
    assert_eq!(log.count(), 2, "the full prompt and the one without Google data");
    assert!(log.prompts().iter().all(|p| p.contains(OTHER)));
    assert_clean(&log);
}

// ---- Caches and the re-summary offer ----------------------------------------------------

#[test]
fn turning_exclusion_on_drops_mixed_caches() {
    let fx = fixture(false);
    fx.s.seed_builtin_routines().unwrap();
    let routine = fx.s.list_routines().unwrap()[0].id;
    let run = |summary: &str| {
        fx.s.insert_routine_run(&memory::NewRoutineRun {
            routine_id: routine,
            ran_at_unix_ms: 1,
            status: memory::RunStatus::Success,
            output_summary: summary.into(),
            output_json: "{}".into(),
        })
        .unwrap()
    };
    let mixed = run("Yesterday: the Acme pricing call and the giraffe review.");
    let clean = run("Yesterday: the giraffe review.");
    fx.s.cache_daily_digest("2026-11-11", r#"{"summary":"x","open_loops":[]}"#, 1).unwrap();
    fx.s.set_client_control(&fx.acme_key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    assert!(fx.s.get_cached_daily_digest("2026-11-11").unwrap().is_none(), "the digest cache is dropped at once");
    // Fix round 1, B3: runs are user data: kept, and kept out of hosted prompts instead.
    let alive = |id: i64| fx.c.query_row("SELECT COUNT(*) FROM routine_run WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(alive(mixed), 1);
    assert_eq!(alive(clean), 1);
    let filter = crate::exclusion::context_filter(&fx.s);
    assert!(filter.material().is_some_and(|m| !m.clean_runs.contains(&mixed) && !m.clean_runs.contains(&clean)), "made before exclusion: not clean");
    // The re-summary offer: the client's meeting was summarised by a hosted model.
    assert_eq!(crate::exclusion::hosted_summaries_to_redo(&fx.s, &fx.acme_key).unwrap(), vec![fx.acme_meeting]);
    assert_eq!(crate::exclusion::ALREADY_LEFT_NOTE, "Material that already left cannot be recalled.");
    let _ = fx.acme_topic;
}

// ---- Fix round 1 -------------------------------------------------------------------------

/// B2: an answer made on this computer from the client's material is marked; it never rides to
/// a hosted model as history or as a title's input, and with no local model the title keeps its
/// placeholder. Controls: unmarked text goes hosted.
#[test]
fn a_local_answer_never_rides_to_a_hosted_model_as_history_or_title() {
    let fx = fixture(true);
    let (h, l) = (Log::default(), Log::default());
    // An on-device link answers first, from all of memory (the canary included).
    let (answer, outcome) = crate::ask::answer_with(
        &fx.s,
        "what about the pricing?",
        &RequestOrigin::AskBar,
        10,
        &[],
        &RoutingConfig::default(),
        || vec![local(&l, "Local pricing answer."), hosted(&fx.s, &h, "Hosted.")],
        Vec::new,
    )
    .unwrap();
    assert_eq!(answer.text, "Local pricing answer.");
    assert_eq!(outcome, ExclusionOutcome::LocalOnly, "the outcome says it holds excluded material");
    assert!(l.prompts()[0].contains(CANARY));
    // The next turn's history carries it: a hosted-only chain is refused, nothing is sent.
    let history = [
        llm::ChatTurn { role: llm::ChatRole::User, text: "what about the pricing?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: "Local pricing answer.".into() },
    ];
    let r = crate::ask::answer_with(&fx.s, "and from when?", &RequestOrigin::AskBar, 10, &history, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Hosted.")], Vec::new);
    assert!(r.is_err());
    assert_eq!(h.count(), 0);
    // Its title stays on this computer, or keeps the placeholder.
    assert!(crate::ask::generate_conversation_title_with(&fx.s, "what about the pricing?", "Local pricing answer.", &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "T")], Vec::new).is_err());
    assert_eq!(h.count(), 0);
    // Control: an earlier answer marked clean for the current clients goes hosted, and so does
    // its title.
    mark_clean_no_inputs(&fx, "Friday.");
    let clean = [
        llm::ChatTurn { role: llm::ChatRole::User, text: "lunch?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: "Friday.".into() },
    ];
    crate::ask::answer_with(&fx.s, "and where?", &RequestOrigin::AskBar, 10, &clean, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Hosted.")], Vec::new).unwrap();
    crate::ask::generate_conversation_title_with(&fx.s, "lunch?", "Friday.", &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "T")], Vec::new).unwrap();
    assert_eq!(h.count(), 2);
    assert_clean(&h);

    // A streamed answer from a local prepare holding the canary is marked at finalize.
    let ollama = RoutingConfig { preferred_provider: "ollama".into(), ..RoutingConfig::default() };
    let prepared = crate::ask::prepare_ask(&fx.s, "what about the pricing?", 10, &[], &ollama).unwrap();
    assert_eq!(prepared.exclusion, ExclusionOutcome::LocalOnly);
    let done = crate::ask::finalize_ask(&fx.s, &prepared.prepare_id, "Streamed pricing answer.", "gemma", llm::TokenUsage::default());
    assert_eq!(done.exclusion, ExclusionOutcome::LocalOnly);
    assert!(fx.s.is_local_only_text("Streamed pricing answer.").unwrap());
    // The exchange stored with it is flagged.
    let conv = fx.s.insert_conversation("what about the pricing?", 1).unwrap();
    fx.s.append_conversation_exchange(conv, "what about the pricing?", "Streamed pricing answer.", "[]", None, 2).unwrap();
    let flag: i64 = fx.c.query_row("SELECT local_only FROM conversation_exchange WHERE conversation_id = ?1", [conv], |r| r.get(0)).unwrap();
    assert_eq!(flag, 1);
}

/// B3: a routine run made before the client was excluded (here: one that mixed the canary in
/// without naming the client) stays out of hosted Ask while the client is excluded, and is not
/// deleted; a run built from filtered material after exclusion still goes.
#[test]
fn routine_runs_without_clean_provenance_stay_out_of_hosted_prompts() {
    let fx = fixture(false);
    fx.s.seed_builtin_routines().unwrap();
    let routine = fx.s.list_routines().unwrap()[0].id;
    let run = |summary: &str, from: i64| {
        let id = fx
            .s
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: routine,
                ran_at_unix_ms: crate::util::now_unix_ms(),
                status: memory::RunStatus::Success,
                output_summary: summary.into(),
                output_json: "{}".into(),
            })
            .unwrap();
        let inputs = [memory::topics::exclusion::input_ref("meeting", from)];
        fx.s.set_routine_run_provenance(id, &crate::exclusion::provenance_for(&fx.s, &crate::exclusion::context_filter(&fx.s), inputs)).unwrap();
        id
    };
    // Built from the Acme meeting before Acme was excluded.
    let mixed = run(&format!("Yesterday: the {CANARY} pricing call."), fx.acme_meeting);
    // Control: with nothing excluded the run rides hosted Ask.
    let log = Log::default();
    ask(&fx, "what about the pricing review?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts()[0].contains(CANARY), "the run reaches retrieval");

    fx.s.set_client_control(&fx.acme_key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    let after = run("Yesterday: the elephant review.", fx.other_meeting);
    let log = Log::default();
    ask(&fx, "what about the pricing review?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(!log.prompts()[0].contains(CANARY));
    assert!(log.prompts()[0].contains("elephant"), "a run built from filtered material still goes");
    assert_clean(&log);
    let alive: i64 = fx.c.query_row("SELECT COUNT(*) FROM routine_run WHERE id IN (?1, ?2)", [mixed, after], |r| r.get(0)).unwrap();
    assert_eq!(alive, 2, "nothing deleted");
}

/// B4: the production shape (display names in `attendees`, addresses in `attendee_details`). A
/// new meeting with only the client's contact is summarised locally, its transcript stays out of
/// hosted Ask, and a calendar event filed under the client is prepared locally. Controls: a
/// colleague's meeting goes hosted.
#[test]
fn a_client_contacts_meeting_and_the_clients_events_stay_local() {
    let fx = fixture(true);
    let sid = fx.c.query_row("SELECT id FROM topic_source WHERE ref_id = ?1 AND kind = 'meeting'", [fx.acme_meeting], |r| r.get::<_, i64>(0)).unwrap();
    fx.c.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
    let wile = fx.c.last_insert_rowid();
    fx.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", rusqlite::params![wile, sid]).unwrap();
    let new = meeting(&fx.s, "Weekly sync", &["Wile E. Coyote"], crate::util::now_unix_ms() - 600_000, &[&format!("The {CANARY} plan for the weekly pricing.")], "");
    fx.s.set_meeting_attendee_details(new, &[memory::speaker_names::Attendee { name: "Wile E. Coyote".into(), email: Some("wile@coyote.example".into()) }]).unwrap();
    let colleague = meeting(&fx.s, "Planning", &["Carol"], crate::util::now_unix_ms() - 700_000, &["The giraffe plan."], "");
    fx.s.set_meeting_attendee_details(colleague, &[memory::speaker_names::Attendee { name: "Carol".into(), email: Some("carol@example.com".into()) }]).unwrap();

    let (h, l) = (Log::default(), Log::default());
    crate::summary::summarize_meeting_routed(&fx.s, new, "", "The weekly plan.", "", &RoutingConfig::default(), "", || vec![hosted(&fx.s, &h, summary_reply())], || vec![local(&l, summary_reply())]).unwrap();
    assert_eq!((h.count(), l.count()), (0, 1), "classified by the stored attendee address");
    crate::summary::summarize_meeting_routed(&fx.s, colleague, "", "The giraffe plan.", "", &RoutingConfig::default(), "", || vec![hosted(&fx.s, &h, summary_reply())], || vec![local(&l, summary_reply())]).unwrap();
    assert_eq!(h.count(), 1, "control: a colleague's meeting goes hosted");

    let log = Log::default();
    ask(&fx, "what is the weekly plan?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_clean(&log);

    // A calendar event filed under the client, whose title never names it.
    fx.c.execute(
        "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
         VALUES ('d', 'calendar_event', 'gcal', 90000000, '[]', 'Budget review', 'Budget review', 'evt-acme')",
        [],
    )
    .unwrap();
    file(&fx.c, fx.acme_topic, "calendar_event", fx.c.last_insert_rowid());
    let filter = crate::exclusion::context_filter(&fx.s);
    assert!(filter.excludes_event("evt-acme", "Budget review", 90_000_000));
    assert!(!filter.excludes_event("evt-other", "Lunch", 90_000_000));
    let events = [
        routines::CalendarEventSummary { id: None, title: "Budget review".into(), start_unix_ms: 90_000_000 },
        routines::CalendarEventSummary { id: None, title: "Lunch".into(), start_unix_ms: 91_000_000 },
    ];
    let kept = crate::exclusion::events_without_excluded(&fx.s, &events, &llm::ContextFilter::none());
    assert_eq!(kept.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["Lunch"]);
}

/// B5: the client's live meeting is judged locally even when no line names the client; a
/// caller that cannot say which meeting fails closed while a client meeting is recording.
#[test]
fn live_questions_in_the_clients_meeting_stay_local() {
    let fx = fixture(true);
    let reply = r#"{"is_for_user":false,"confidence":"low","quote":"q","asker":null,"restatement":"r"}"#;
    let (h, l) = (Log::default(), Log::default());
    let origin = crate::live_question::live_origin(&fx.s, Some(fx.acme_meeting));
    crate::live_question::detect_with(&fx.s, "Mike, can you ship it Friday?", &origin, || vec![hosted(&fx.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!((h.count(), l.count()), (0, 1));
    // Control: another client's meeting goes hosted.
    let other = crate::live_question::live_origin(&fx.s, Some(fx.other_meeting));
    crate::live_question::detect_with(&fx.s, "Mike, can you ship it Friday?", &other, || vec![hosted(&fx.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!(h.count(), 1);
    // No meeting id: while the client's meeting records, fail closed.
    assert_eq!(crate::live_question::live_origin(&fx.s, None), RequestOrigin::Other);
    fx.c.execute("UPDATE meeting SET status = 'recording' WHERE id = ?1", [fx.acme_meeting]).unwrap();
    assert_eq!(crate::live_question::live_origin(&fx.s, None), RequestOrigin::Meeting { meeting_id: fx.acme_meeting });
}

/// M4: local-only mode with an Ollama cloud model never streams (the prepare refuses), its line
/// still says local only; a cloud model outside local-only shows as leaving; history decides
/// the line too.
#[test]
fn the_destination_line_is_truthful() {
    let fx = fixture(true);
    let local_only_cloud = RoutingConfig { sensitive: true, ollama_model: "gpt-oss:120b-cloud".into(), ..RoutingConfig::default() };
    assert!(crate::ask::prepare_ask(&fx.s, "lunch?", 5, &[], &local_only_cloud).is_err());
    assert_eq!(crate::window_ask::destination(&local_only_cloud), crate::window_ask::Destination::LocalOnly);
    let cloud = RoutingConfig { preferred_provider: "ollama".into(), ollama_model: "gpt-oss:120b-cloud".into(), ..RoutingConfig::default() };
    assert_eq!(crate::window_ask::destination(&cloud), crate::window_ask::Destination::Hosted { provider: "ollama".into() });
    let on_device = RoutingConfig { preferred_provider: "ollama".into(), ollama_model: "gemma4:12b".into(), ..RoutingConfig::default() };
    assert_eq!(crate::window_ask::destination(&on_device), crate::window_ask::Destination::Local);
    fx.s.mark_local_only_text("An earlier local answer.").unwrap();
    let input = crate::window_ask::WindowInput { app_name: "Notes".into(), title: "Notes".into(), text: "lunch".into(), is_selection: false };
    let history = [llm::ChatTurn { role: llm::ChatRole::Assistant, text: "An earlier local answer.".into() }];
    assert_eq!(crate::window_ask::destination_for(&fx.s, &hosted_routing(), "and?", &history, &input), crate::window_ask::Destination::LocalOnly);
    assert_eq!(crate::window_ask::destination_for(&fx.s, &hosted_routing(), "and?", &[], &input), crate::window_ask::Destination::Hosted { provider: "anthropic".into() });
}

// ---- Fix round 2 -------------------------------------------------------------------------

/// Fix round 2, N2: `excludes_source` (judge, steps, naming) and `keeps` (retrieval) give the
/// same answer for every kind: a slide of a meeting excluded by a transcript name hit, and an
/// email whose thread sibling names the client, are excluded both ways. Controls: another
/// meeting's slide and another thread's email are kept both ways.
#[test]
fn excludes_source_agrees_with_keeps_for_every_kind() {
    let fx = fixture(true);
    let named = meeting(&fx.s, "Catch-up", &[], crate::util::now_unix_ms() - 900_000, &["Thanks, Acme."], "");
    let slide = |meeting_id: i64, text: &str| -> i64 {
        fx.c.execute(
            "INSERT INTO meeting_snapshot (meeting_id, captured_unix_ms, file_path, width, height, ocr_text) VALUES (?1, 1, 'f.png', 1, 1, ?2)",
            rusqlite::params![meeting_id, text],
        )
        .unwrap();
        let snap = fx.c.last_insert_rowid();
        fx.c.execute(
            "INSERT INTO items (device_id, kind, source, source_table, source_id, thread_id, ts_unix_ms, body_text) \
             VALUES ('d', 'snapshot', 'slides', 'meeting_snapshot', ?1, ?2, 1, ?3)",
            rusqlite::params![snap, meeting_id, text],
        )
        .unwrap();
        fx.c.last_insert_rowid()
    };
    let named_slide = slide(named, "Q3 targets");
    let other_slide = slide(fx.other_meeting, "Q3 targets");
    let email = |ext: &str, thread: &str, body: &str| -> i64 {
        fx.s.upsert_email_items(&[memory::NewEmailItem {
            device_id: "d".into(),
            external_id: ext.into(),
            ts_unix_ms: 1_000,
            from: "x@example.com".into(),
            to: "me@example.com".into(),
            subject: "Re: plan".into(),
            body_text: body.into(),
            gmail_thread_id: thread.into(),
            account_email: "me@example.com".into(),
            ..Default::default()
        }])
        .unwrap();
        fx.c.query_row("SELECT id FROM items WHERE external_id = ?1", [ext], |r| r.get(0)).unwrap()
    };
    let naming = email("e1", "t-acme", "Acme wants the plan.");
    let sibling = email("e2", "t-acme", "Sounds good, sending Friday.");
    let other_mail = email("e3", "t-other", "Sounds good, sending Friday.");
    fx.s.run_jobs_batch(10, crate::util::now_unix_ms()).unwrap();
    let f = crate::exclusion::context_filter(&fx.s);
    assert!(f.excludes_source("snapshot", named_slide), "slide of a name-hit meeting");
    assert!(!f.excludes_source("snapshot", other_slide), "control slide");
    assert!(f.excludes_source("email", naming));
    assert!(f.excludes_source("email", sibling), "thread sibling");
    assert!(!f.excludes_source("email", other_mail), "control email");
    // Parity with keeps, item by item.
    let item = |kind: &str, id: i64| f.excludes_source(kind, id);
    let thread_of = |id: i64| fx.s.thread_of_email(id).unwrap().unwrap();
    let thread = |id: i64| memory::Thread {
        id: thread_of(id),
        source: "gmail".into(),
        external_thread_id: String::new(),
        account_email: String::new(),
        subject: None,
        message_count: 1,
        last_activity_ts_unix_ms: 1,
        last_message_from: String::new(),
        summary: None,
        status: "waiting".into(),
        summarize_status: "done".into(),
    };
    for (kind, id, ci) in [
        ("snapshot", named_slide, llm::ContextItem::Snapshot(memory::MeetingSnapshotHit { id: named_slide, meeting_id: named, meeting_title: "Catch-up".into(), text: "Q3 targets".into(), file_path: String::new() })),
        ("snapshot", other_slide, llm::ContextItem::Snapshot(memory::MeetingSnapshotHit { id: other_slide, meeting_id: fx.other_meeting, meeting_title: "Team standup".into(), text: "Q3 targets".into(), file_path: String::new() })),
        ("email", sibling, llm::ContextItem::Thread(thread(sibling))),
        ("email", other_mail, llm::ContextItem::Thread(thread(other_mail))),
    ] {
        assert_eq!(item(kind, id), !f.keeps(&ci), "{kind} {id}: excludes_source and keeps disagree");
    }
}

/// Fix round 2, N3: a client meeting retitled at transcription ("Room audio only \u{b7} ...") and
/// one that started late stay out of hosted Ask. Control: another meeting goes.
#[test]
fn a_retitled_or_late_client_meeting_stays_out_of_hosted_ask() {
    let fx = fixture(true);
    let start = crate::util::now_unix_ms() - 4 * 3_600_000;
    fx.c.execute(
        "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
         VALUES ('d', 'calendar_event', 'gcal', ?1, '[]', 'Weekly sync', 'Weekly sync', 'evt-wk')",
        [start],
    )
    .unwrap();
    file(&fx.c, fx.acme_topic, "calendar_event", fx.c.last_insert_rowid());
    meeting(&fx.s, "Room audio only \u{b7} Weekly sync", &[], start + 120_000, &[&format!("The {CANARY} roadmap was set.")], "");
    meeting(&fx.s, "Sync", &[], start + 45 * 60_000, &[&format!("The {CANARY} roadmap again.")], "");
    let log = Log::default();
    ask(&fx, "what about the roadmap?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_eq!(log.count(), 1);
    assert_clean(&log);
    // Control: the same words in another client's meeting do go.
    let other = fixture(false);
    meeting(&other.s, "Room audio only \u{b7} Weekly sync", &[], start + 120_000, &[&format!("The {CANARY} roadmap was set.")], "");
    let log = Log::default();
    ask(&other, "what about the roadmap?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts()[0].contains(CANARY));
}

/// Fix round 2, N4: an episode over a capture whose URL names the client is the client's; its
/// locally made summary stays out of hosted Ask. Control: an episode over other captures goes.
#[test]
fn an_episode_over_captures_naming_the_client_stays_out() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let capture = |ts: i64, url: Option<&str>, text: &str| {
        fx.s.insert_capture_item(&memory::NewCaptureItem {
            device_id: "d".into(),
            ts_unix_ms: ts,
            app_bundle_id: "com.browser".into(),
            window_title: "Deck".into(),
            url: url.map(str::to_string),
            text: text.into(),
            hash: ts,
            source: "ax".into(),
        })
        .unwrap();
        fx.c.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get::<_, i64>(0)).unwrap()
    };
    let episode = |first: i64, last: i64, summary: &str| {
        fx.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('d', 'com.browser', 'Deck', ?3, ?3, ?1, ?2, 2, ?4, 'summarized', ?3, ?3)",
            rusqlite::params![first, last, now - 60_000, summary],
        )
        .unwrap();
    };
    let a = capture(now - 120_000, Some("https://acme.sharepoint.example/q3"), "Q3 pricing deck");
    let b = capture(now - 119_000, None, "Drafting a reply");
    episode(a, b, &format!("Reviewed the {CANARY} pricing deck; drafted a reply."));
    let c = capture(now - 100_000, None, "Lunch menu");
    let d = capture(now - 99_000, None, "Ordering");
    episode(c, d, "Ordered the giraffe lunch.");
    let log = Log::default();
    ask(&fx, "what did I do today with the deck?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_eq!(log.count(), 1);
    assert!(log.prompts()[0].contains("giraffe lunch"), "control episode goes");
    assert_clean(&log);
}

/// Fix round 2, N5: provenance uses the clients the filter actually left out. A client excluded
/// during the run makes it not clean.
#[test]
fn provenance_uses_the_keys_the_filter_used() {
    let fx = fixture(true);
    let used = crate::exclusion::context_filter(&fx.s);
    let globex = fx.s.list_seed_topics().unwrap().into_iter().find(|t| t.name == "Globex").unwrap();
    fx.s.set_client_control(&globex.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    fx.s.seed_builtin_routines().unwrap();
    let routine = fx.s.list_routines().unwrap()[0].id;
    let run = fx
        .s
        .insert_routine_run(&memory::NewRoutineRun { routine_id: routine, ran_at_unix_ms: 1, status: memory::RunStatus::Success, output_summary: "x".into(), output_json: "{}".into() })
        .unwrap();
    // Built from Globex's meeting, which the filter it used did not leave out.
    let inputs = [memory::topics::exclusion::input_ref("meeting", fx.other_meeting)];
    fx.s.set_routine_run_provenance(run, &crate::exclusion::provenance_for(&fx.s, &used, inputs)).unwrap();
    let now = crate::exclusion::context_filter(&fx.s);
    assert!(!now.material().unwrap().clean_runs.contains(&run), "built from Globex's material before Globex was filtered");
}

/// Fix round 2, N7: an answer from before exclusion was switched on is not clean history: a
/// hosted turn is refused. Control: an answer marked clean for the current clients goes.
#[test]
fn answers_from_before_exclusion_are_not_clean_history() {
    let fx = fixture(true);
    let history = |answer: &str| {
        [
            llm::ChatTurn { role: llm::ChatRole::User, text: "what is planned?".into() },
            llm::ChatTurn { role: llm::ChatRole::Assistant, text: answer.into() },
        ]
    };
    let h = Log::default();
    let r = crate::ask::answer_with(&fx.s, "and then?", &RequestOrigin::AskBar, 10, &history("We planned the launch."), &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Hosted.")], Vec::new);
    assert!(r.is_err(), "made before exclusion: refused for a hosted chain");
    assert_eq!(h.count(), 0);
    mark_clean_no_inputs(&fx, "Friday works.");
    crate::ask::answer_with(&fx.s, "and then?", &RequestOrigin::AskBar, 10, &history("Friday works."), &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Hosted.")], Vec::new).unwrap();
    assert_eq!(h.count(), 1, "control: a clean answer goes");
}

/// Fix round 2, N8: a read error while finding the live meeting fails closed.
#[test]
fn live_origin_fails_closed_on_a_read_error() {
    let fx = fixture(true);
    // A row the meeting reader cannot parse (the exclusion predicate does not read this column).
    fx.c.execute("UPDATE meeting SET device_id = X'00ff' WHERE id = ?1", [fx.other_meeting]).unwrap();
    assert!(fx.s.recent_meetings(50).is_err(), "the read fails");
    assert!(!crate::exclusion::context_filter(&fx.s).is_closed(), "while the setting reads fine");
    let origin = crate::live_question::live_origin(&fx.s, None);
    assert!(crate::exclusion::request_carries_client(&fx.s, "can you ship Friday?", origin));
}

// ---- Fix round 3 -------------------------------------------------------------------------

/// A hosted link that runs `hook` (once) during its first call: a setting changed mid-request.
struct Hooked {
    log: Log,
    reply: String,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl llm::LlmClient for Hooked {
    fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
        if let Some(hook) = self.hook.lock().unwrap().take() {
            hook();
        }
        self.log.0.lock().unwrap().push(prompt.to_string());
        Ok(llm::Completion { text: self.reply.clone(), token_usage: llm::TokenUsage::default() })
    }
}

fn hooked(store: &memory::Store, log: &Log, reply: &str, hook: Box<dyn FnOnce() + Send>) -> llm::ChainLink {
    llm::guard_hosted(
        llm::ChainLink {
            provider: llm::Provider::Anthropic,
            model: "claude-test".into(),
            client: Box::new(Hooked { log: log.clone(), reply: reply.into(), hook: Mutex::new(Some(hook)) }),
        },
        crate::exclusion::current_gate(store),
    )
}

/// Excludes seed `key` through its own connection, as another window or device would.
fn exclude_elsewhere(fx: &Fx, key: &str) -> Box<dyn FnOnce() + Send> {
    let path = fx._dir.path().join("m.db");
    let key = key.to_string();
    Box::new(move || {
        crate::test_store::raw(&path)
            .execute(
                "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) VALUES (?1, 1, NULL, 9) \
                 ON CONFLICT(topic_key) DO UPDATE SET exclude_hosted = 1",
                [key],
            )
            .unwrap();
    })
}

/// Marks `answer` clean with no recorded inputs, as an answer made from nothing stored would be.
fn mark_clean_no_inputs(fx: &Fx, answer: &str) {
    let filter = crate::exclusion::context_filter(&fx.s);
    crate::ask::mark_clean(&fx.s, crate::ask::clean_stamp(&filter, &fx.s, &[], [], &[], &[]), answer);
}

fn globex_key(fx: &Fx) -> String {
    fx.s.list_seed_topics().unwrap().into_iter().find(|t| t.name == "Globex").unwrap().key
}

fn as_history(answer: &str) -> [llm::ChatTurn; 2] {
    [
        llm::ChatTurn { role: llm::ChatRole::User, text: "what was agreed?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: answer.into() },
    ]
}

/// Fix round 3, T1: a blocking meeting answer is stamped clean only for what its prompt was
/// filtered for. A client excluded during the call makes it unusable as hosted history.
/// Control: an answer with no change during the call rides as history.
#[test]
fn a_meeting_answer_is_stamped_only_for_what_it_was_built_for() {
    let fx = fixture(true);
    let h = Log::default();
    let key = globex_key(&fx);
    let racy = crate::ask::answer_meeting_with(&fx.s, fx.other_meeting, "what was agreed?", &[], &RoutingConfig::default(), || vec![hooked(&fx.s, &h, "The giraffe review moved.", exclude_elsewhere(&fx, &key))], Vec::new)
        .unwrap();
    let later = Log::default();
    let r = crate::ask::answer_with(&fx.s, "and then?", &RequestOrigin::AskBar, 10, &as_history(&racy.text), &RoutingConfig::default(), || vec![hosted(&fx.s, &later, "x")], Vec::new);
    assert!(r.is_err(), "made from Globex's meeting while it was not filtered");
    assert_eq!(later.count(), 0);
    // Control.
    let fresh = fixture(true);
    let quiet = crate::ask::answer_meeting_with(&fresh.s, fresh.other_meeting, "what was agreed?", &[], &RoutingConfig::default(), || vec![hosted(&fresh.s, &Log::default(), "The giraffe review moved.")], Vec::new)
        .unwrap();
    let ok = Log::default();
    crate::ask::answer_with(&fresh.s, "and then?", &RequestOrigin::AskBar, 10, &as_history(&quiet.text), &RoutingConfig::default(), || vec![hosted(&fresh.s, &ok, "x")], Vec::new).unwrap();
    assert_eq!(ok.count(), 1);
}

/// Fix round 3: a client excluded during a prep batch is not sent for the rest of the batch.
#[test]
fn a_client_excluded_mid_prep_batch_is_prepared_locally() {
    let fx = fixture(true);
    fx.s.insert_routine(&memory::NewRoutine { name: "Meeting Prep".into(), template: memory::RoutineTemplate::MeetingPrep, schedule_hhmm: "00:00".into(), enabled: true }).unwrap();
    let now = crate::util::now_unix_ms();
    let upcoming = |id: &str, title: &str, start: i64| crate::connectors::calendar::QualifyingMeeting {
        id: id.into(),
        title: title.into(),
        start_unix_ms: start,
        end_unix_ms: start + 30 * 60_000,
        attendees: vec!["Carol <carol@example.com>".into()],
        attendee_details: Vec::new(),
        conference_link: None,
        source: crate::connectors::calendar::MeetingSource::GoogleCalendar,
        app_bundle_id: None,
        organizer_email: None,
        organizer_name: None,
        self_is_organizer: false,
        self_response: None,
    };
    let meetings = [upcoming("e1", "Planning", now + 10 * 60_000), upcoming("e2", "Globex review", now + 20 * 60_000)];
    let mut hook = Some(exclude_elsewhere(&fx, &globex_key(&fx)));
    let calls: std::cell::RefCell<Vec<(String, bool)>> = Default::default();
    crate::meeting_prep::run_gated_with(&fx.s, &meetings, now, 30.0, "", "", crate::routine_runs::RunGate::Full, &mut |p, local_only| {
        if let Some(h) = hook.take() {
            h();
        }
        calls.borrow_mut().push((p.to_string(), local_only));
        None
    })
    .unwrap();
    let calls = calls.into_inner();
    assert_eq!(calls.len(), 2);
    assert!(!calls[0].1, "control: the first brief went hosted");
    assert!(calls[1].1, "Globex, excluded during the batch, is prepared locally");
}

/// Fix round 3, R1: a meeting whose notes name the client is the client's: its transcript stays
/// out of hosted Ask. Control: without the note it goes.
#[test]
fn a_meeting_whose_notes_name_the_client_stays_out() {
    let open = fixture(true);
    let start = crate::util::now_unix_ms() - 900_000;
    meeting(&open.s, "Call", &[], start, &[&format!("The {CANARY} quote stands.")], "");
    let log = Log::default();
    ask(&open, "what about the quote?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts()[0].contains(CANARY), "control");
    let fx = fixture(true);
    let m = meeting(&fx.s, "Call", &[], start, &[&format!("The {CANARY} quote stands.")], "");
    fx.c.execute("UPDATE meeting SET notes = 'Acme Q3 pricing' WHERE id = ?1", [m]).unwrap();
    let log = Log::default();
    ask(&fx, "what about the quote?", RequestOrigin::AskBar, &log, None).unwrap();
    assert_eq!(log.count(), 1);
    assert_clean(&log);
}

/// Fix round 4, S1: every text written about a meeting counts as the meeting's words: the
/// workspace draft (the Mac's personal notes never reach `meeting.notes`; edits after the
/// recording), action items, follow-ups, speaker names, live alerts, live transcript lines, slide
/// text and attendee details. Each case writes through the real save API on a fixture whose
/// hosted Ask already carried the transcript (the control, which also warms the exclusion cache),
/// so only the table's generation trigger can make the next Ask see it. Then the summary goes
/// on-device. Every case is checked; failures are collected, not stopped at.
#[test]
fn a_meeting_whose_workspace_notes_name_the_client_stays_out() {
    let start = crate::util::now_unix_ms() - 900_000;
    let transcript = format!("The {CANARY} quote stands.");
    type Save = fn(&memory::Store, i64);
    let saves: [(&str, Save); 10] = [
        ("personal notes (Mac draft)", |s, m| s.save_meeting_personal_notes(m, "Acme Q3 pricing", 1).unwrap()),
        ("edited summary", |s, m| s.save_meeting_note_overrides(m, Some("Acme renewal agreed."), None, None, 1).unwrap()),
        ("edited decisions", |s, m| {
            s.save_meeting_note_overrides(m, None, Some(&["Roadrunner Co signs".to_string()]), Some(&["Ask Acme".to_string()]), 1).unwrap()
        }),
        ("edited action item", |s, m| {
            let id = s.insert_meeting_action_item(m, "Send the deck", 0, 1).unwrap();
            s.update_meeting_action_item_text(id, "Send Acme the deck", 2).unwrap();
        }),
        ("follow-up", |s, m| {
            s.insert_meeting_followup(m, "Chase Roadrunner Co for sign-off", None, 1).unwrap();
        }),
        ("speaker name", |s, m| {
            s.upsert_person("wile", "Wile from Acme", None).unwrap();
            s.set_meeting_speaker(m, 0, Some("wile"), None).unwrap();
        }),
        ("live alert", |s, m| {
            s.insert_live_alert(&memory::live_alert::NewLiveAlert {
                meeting_id: m,
                meeting_title: "Call".into(),
                quote: "Can you send it?".into(),
                restatement: "Send the Acme quote".into(),
                asker: None,
                at_cs: 0,
                created_at_unix_ms: 1,
            })
            .unwrap();
        }),
        ("live transcript line", |s, m| {
            s.insert_live_segments(&[memory::live::NewLiveSegment {
                meeting_id: m,
                channel: "mic".into(),
                chunk_index: 0,
                start_cs: 0,
                end_cs: 100,
                speaker_label: "Speaker 1".into(),
                text: "Thanks, Acme.".into(),
            }])
            .unwrap();
        }),
        ("slide text", |s, m| {
            s.insert_meeting_snapshot(&memory::NewMeetingSnapshot { meeting_id: m, captured_unix_ms: 1, file_path: "f.png".into(), width: 1, height: 1, ocr_text: "Acme roadmap".into() })
                .unwrap();
        }),
        ("attendee details", |s, m| {
            s.set_meeting_attendee_details(m, &[memory::speaker_names::Attendee { name: "Roadrunner Co legal".into(), email: None }]).unwrap();
        }),
    ];
    // Control: with nothing written about it, the meeting's summary goes hosted.
    let control = fixture(true);
    let m = meeting(&control.s, "Call", &[], start, &[&transcript], "");
    let h = Log::default();
    crate::summary::summarize_meeting_routed(&control.s, m, "", &transcript, "", &RoutingConfig::default(), "", || vec![hosted(&control.s, &h, summary_reply())], Vec::new).unwrap();
    assert_eq!(h.count(), 1, "control: the summary goes hosted");
    let mut failures = Vec::new();
    for (what, save) in saves {
        let fx = fixture(true);
        let m = meeting(&fx.s, "Call", &[], start, &[&transcript], "");
        let warm = Log::default();
        ask(&fx, "what about the quote?", RequestOrigin::AskBar, &warm, None).unwrap();
        assert!(warm.prompts()[0].contains(CANARY), "{what}: control, before the save");
        save(&fx.s, m);
        let log = Log::default();
        ask(&fx, "what about the quote?", RequestOrigin::AskBar, &log, None).unwrap();
        if log.prompts().iter().any(|p| p.contains(CANARY)) {
            failures.push(format!("{what}: hosted Ask"));
        }
        assert_clean_of_names(&log);
        let (h, l) = (Log::default(), Log::default());
        crate::summary::summarize_meeting_routed(&fx.s, m, "", &transcript, "", &RoutingConfig::default(), "", || vec![hosted(&fx.s, &h, summary_reply())], || vec![local(&l, summary_reply())]).unwrap();
        if h.count() != 0 || l.count() != 1 {
            failures.push(format!("{what}: summary"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The Mac says why a summary is missing: refused only when the meeting names an excluded client
/// and no local link exists, and it clears once one does.
#[test]
fn summary_refusal_is_reported_only_for_an_excluded_meeting_with_no_local_model() {
    let fx = fixture(true);
    // The meeting is filed under the excluded client: refused while the local chain is empty.
    assert!(crate::summary::summary_refused_with(&fx.s, fx.acme_meeting, || true));
    // A local model is available: the summary goes local, nothing to report.
    assert!(!crate::summary::summary_refused_with(&fx.s, fx.acme_meeting, || false));
    // Another client's meeting is summarised hosted, so never refused.
    assert!(!crate::summary::summary_refused_with(&fx.s, fx.other_meeting, || true));
    // With nothing excluded, no meeting is refused.
    let open = fixture(false);
    assert!(!crate::summary::summary_refused_with(&open.s, open.acme_meeting, || true));
    // The public check builds the real local chain: Ollama off and no bundled model is empty.
    let off = RoutingConfig { use_local_model: false, ..RoutingConfig::default() };
    assert_eq!(
        crate::summary::summary_refused_for_exclusion(&fx.s, fx.acme_meeting, "gemma4:12b", &off),
        crate::routing::build_local_chain("gemma4:12b", &off).is_empty()
    );
    let on = RoutingConfig { use_local_model: true, ..RoutingConfig::default() };
    assert!(!crate::summary::summary_refused_for_exclusion(&fx.s, fx.acme_meeting, "gemma4:12b", &on));
}

/// [`assert_clean`] without the canary, which the collecting tests check themselves.
fn assert_clean_of_names(log: &Log) {
    for p in log.prompts() {
        let lower = p.to_lowercase();
        for banned in ["acme", "roadrunner", "exclu", "left out"] {
            assert!(!lower.contains(banned), "a hosted request carried {banned:?}:\n{p}");
        }
    }
}

/// Fix round 3, R2: the other captures of an episode lifted by a capture naming the client stay
/// out of hosted Ask. Control: another episode's capture goes.
#[test]
fn a_lifted_episodes_other_captures_stay_out() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let capture = |ts: i64, url: Option<&str>, text: &str| {
        fx.s.insert_capture_item(&memory::NewCaptureItem { device_id: "d".into(), ts_unix_ms: ts, app_bundle_id: "com.deck".into(), window_title: "Deck".into(), url: url.map(str::to_string), text: text.into(), hash: ts, source: "ax".into() }).unwrap();
        fx.c.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get::<_, i64>(0)).unwrap()
    };
    let episode = |a: i64, b: i64| {
        fx.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','com.deck','Deck',1,2,?1,?2,2,'summarized',1,1)",
            rusqlite::params![a, b],
        )
        .unwrap();
    };
    let a = capture(now - 50_000, Some("https://acme.example/deck"), "Slide one");
    let b = capture(now - 49_000, None, &format!("{CANARY} pricing slide two"));
    episode(a, b);
    let c = capture(now - 40_000, None, "Slide one");
    let d = capture(now - 39_000, None, "giraffe pricing slide two");
    episode(c, d);
    let log = Log::default();
    ask(&fx, "pricing slide two", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts()[0].contains("giraffe pricing slide"), "control");
    assert_clean(&log);
}

/// Fix round 3, R4 and fix round 4, S2 (ruling a): a daily brief records the inputs it was built
/// from. It stays clean while new Acme material that was not among them arrives, and stops being
/// clean once a meeting it was built from is filed under Acme. Built by `run_daily_brief` with a
/// recording hosted transport, stamped as the tick stamps it; read back through hosted Ask.
#[test]
fn a_brief_built_from_a_meeting_later_filed_under_the_client_is_not_clean() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let m = meeting(&fx.s, "Call", &[], now - 900_000, &["The okapi pricing review is set."], "");
    fx.s.seed_builtin_routines().unwrap();
    let routine = fx.s.list_routines().unwrap()[0].id;
    let brief_log = Log::default();
    let chain = vec![hosted(&fx.s, &brief_log, r#"{"summary": "Yesterday: the okapibrief pricing review.", "open_loops": []}"#)];
    let filter = crate::exclusion::filter_for_chain(&fx.s, &chain);
    let events = [routines::CalendarEventSummary { id: Some("evt-1@google.com".into()), title: "Standup".into(), start_unix_ms: now }];
    let outcome = routines::run_daily_brief(&fx.s, chain[0].client.as_ref(), &filter, now - 86_400_000, now, &events, now, "", "").unwrap();
    assert!(brief_log.prompts()[0].contains("okapi pricing"), "the brief was built from the meeting");
    assert!(outcome.input_refs.contains(&memory::topics::exclusion::input_ref("meeting", m)), "{:?}", outcome.input_refs);
    let mut new_run = outcome.new_run;
    new_run.routine_id = routine;
    let run = fx.s.insert_routine_run(&new_run).unwrap();
    fx.s.set_routine_run_provenance(run, &crate::exclusion::provenance_for(&fx.s, &filter, outcome.input_refs)).unwrap();

    let brief_reaches_hosted_ask = || {
        let log = Log::default();
        ask(&fx, "what about the pricing review?", RequestOrigin::AskBar, &log, None).unwrap();
        assert_clean(&log);
        log.prompts()[0].contains("okapibrief")
    };
    assert!(brief_reaches_hosted_ask(), "control: clean when built");
    // Steady capture of Acme material that the brief was not built from: still clean.
    for i in 0..3 {
        fx.s.insert_capture_item(&memory::NewCaptureItem { device_id: "d".into(), ts_unix_ms: now + i, app_bundle_id: "com.mail".into(), window_title: "Mail".into(), url: None, text: format!("Acme contract redline {i}"), hash: 900 + i, source: "ax".into() }).unwrap();
    }
    file(&fx.c, fx.acme_topic, "meeting", meeting(&fx.s, "Acme review", &[], now - 100_000, &["Contract terms."], ""));
    assert!(brief_reaches_hosted_ask(), "new client material that was not among its inputs");
    // A meeting it was built from is filed under Acme: no longer clean.
    file(&fx.c, fx.acme_topic, "meeting", m);
    assert!(!brief_reaches_hosted_ask(), "built from a meeting now known to be Acme's");
}

/// Fix round 4, S2 (advisor): an input that no longer exists cannot be checked, so a stamp
/// recording it is not clean while a client is excluded. Built from Globex's meeting, the meeting
/// is then deleted and Globex excluded: a follow-up does not carry the answer to a hosted model.
/// Control: before the deletion and exclusion it does.
#[test]
fn an_answer_built_from_a_deleted_input_is_not_clean() {
    let fx = fixture(false);
    // A client with no material is excluded from the start (with nothing excluded, the question's
    // word fingerprints are not kept, and the answer would be unclean for that reason alone).
    let initech = fx.s.create_seed_topic("Initech", &[], None).unwrap();
    fx.s.set_client_control(&initech.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    let first = Log::default();
    crate::ask::answer_with(&fx.s, "what about the pricing review?", &RequestOrigin::AskBar, 10, &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &first, "The giraffe review is next week.")], Vec::new).unwrap();
    assert!(first.prompts()[0].contains(OTHER), "built from Globex's meeting");
    let history = [
        llm::ChatTurn { role: llm::ChatRole::User, text: "what about the pricing review?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: "The giraffe review is next week.".into() },
    ];
    let (h, l) = (Log::default(), Log::default());
    let follow_up = || {
        crate::ask::answer_with(&fx.s, "and who runs it?", &RequestOrigin::AskBar, 10, &history, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Carol does.")], || vec![local(&l, "Carol does.")])
    };
    // Control: the answer's inputs are not Initech's, so it is clean.
    follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 0), "control");
    fx.s.forget_meeting(fx.other_meeting).unwrap();
    fx.s.set_client_control(&globex_key(&fx), &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 1), "its input is gone, so it cannot be shown not to be Globex's");
}

/// The kept word fingerprint sets (`input_words`), decoded.
fn kept_fingerprints(c: &rusqlite::Connection) -> Vec<Vec<u32>> {
    c.prepare("SELECT fingerprints FROM input_words")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|b| b.unwrap().chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
        .collect()
}

/// Fix round 7, F1: stamps written while nothing was excluded keep no fingerprints, so the purge
/// by name never finds them. A meeting Ask and a live draft (neither is stored in a conversation)
/// naming Initech, with nothing excluded, leave stamps; `delete_client` Initech drops every stamp
/// that cannot be checked by name. Control: a stamp with checkable inputs (another client's) stays.
#[test]
fn deleting_a_client_drops_stamps_kept_without_fingerprints() {
    let fx = fixture(false);
    let initech = fx.s.create_seed_topic("Initech", &[], None).unwrap();
    let h = Log::default();
    let meeting_answer = "Initech wanted the pricing sheet.";
    crate::ask::answer_meeting_with(&fx.s, fx.other_meeting, "what did Initech want?", &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &h, meeting_answer)], Vec::new).unwrap();
    // A live draft: the meeting background goes in as a user turn (live_question::draft).
    let draft_answer = "I can send the Initech terms today.";
    let background = [llm::ChatTurn { role: llm::ChatRole::User, text: "Background, the last minutes of the meeting:\n[mic] Initech asked about terms".into() }];
    crate::ask::answer_with(&fx.s, "Draft my reply to this, asked of me in a meeting: can you send the terms?", &RequestOrigin::AskBar, 10, &background, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, draft_answer)], Vec::new).unwrap();
    assert_eq!(h.count(), 2, "both went hosted");
    let stamp = |answer: &str| fx.s.clean_text_stamp(answer).unwrap();
    assert!(stamp(meeting_answer).is_some() && stamp(draft_answer).is_some(), "both were stamped");
    // Control: a stamp whose only inputs are checkable rows (no words with no stored row).
    let filter = crate::exclusion::context_filter(&fx.s);
    crate::ask::mark_clean(&fx.s, crate::ask::clean_stamp(&filter, &fx.s, &[], [memory::topics::exclusion::input_ref("meeting", fx.other_meeting)], &[], &[]), "Hooli's quarter.");
    let sel = crate::client_delete::DeleteSelection { key: initech.key.clone(), include_possible: Vec::new(), delete_exports: false, vault: None, vault_off: false };
    crate::client_delete::delete_client(&fx.s, &sel, &[], None).unwrap();
    assert!(stamp(meeting_answer).is_none(), "the meeting Ask stamp stayed");
    assert!(stamp(draft_answer).is_none(), "the live draft stamp stayed");
    assert!(stamp("Hooli's quarter.").is_some(), "control: a checkable stamp stays");
}

/// Fix round 5, N1: kept word fingerprints are no trace of a deleted client. A question naming
/// Initech (not excluded, while another client is) is answered hosted and its fingerprints kept;
/// deleting Initech leaves no kept set that names it, and none of that ask's sets. Control: a
/// question naming Hooli (not deleted) keeps its set.
#[test]
fn deleting_a_client_leaves_no_fingerprint_of_its_name() {
    let fx = fixture(true);
    let initech = fx.s.create_seed_topic("Initech", &[], None).unwrap();
    for q in ["what did Initech say about the renewal?", "what did Hooli say about the renewal?"] {
        let h = Log::default();
        crate::ask::answer_with(&fx.s, q, &RequestOrigin::AskBar, 10, &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &h, &format!("Answer to {q}"))], Vec::new).unwrap();
        assert_eq!(h.count(), 1, "{q}: hosted");
    }
    let names = |name: &str, sets: &[Vec<u32>]| sets.iter().any(|f| memory::words::fingerprints_name_any(f, &[memory::words::folded_words(name)]));
    let before = kept_fingerprints(&fx.c);
    assert!(names("Initech", &before) && names("Hooli", &before), "control: both questions were kept");
    let sel = crate::client_delete::DeleteSelection { key: initech.key.clone(), include_possible: Vec::new(), delete_exports: false, vault: None, vault_off: false };
    crate::client_delete::delete_client(&fx.s, &sel, &[], None).unwrap();
    let after = kept_fingerprints(&fx.c);
    assert!(!names("Initech", &after), "a kept set still names the deleted client");
    assert!(names("Hooli", &after), "control: another client's question is kept");
}

/// Fix round 5, N2: a window Ask's stamp keeps fingerprints of the redacted, capped text the prompt
/// carries, never of a secret in the raw window text. Control: the window's other words are kept.
#[test]
fn a_window_secret_is_never_fingerprinted() {
    let fx = fixture(true);
    let key = format!("apikey_{}", "a1b2_c3".repeat(10));
    let input = crate::window_ask::WindowInput {
        app_name: "Terminal".into(),
        title: "deploy".into(),
        text: format!("export TOKEN={key}\nThe okapi rollout is Friday."),
        is_selection: false,
    };
    let prepared = crate::window_ask::prepare_window_ask(&fx.s, &input, "when is the rollout?", &[], &hosted_routing()).unwrap();
    crate::ask::finalize_ask(&fx.s, &prepared.prepare_id, "Friday.", "claude-test", llm::TokenUsage::default());
    let kept = kept_fingerprints(&fx.c);
    let has = |words: &str| kept.iter().any(|f| f.binary_search(&memory::words::fingerprint(&memory::words::folded_words(words))).is_ok());
    assert!(has("okapi rollout"), "control: the window's words are kept");
    assert!(has("terminal deploy"), "the app name and title are one input with the text (N3)");
    assert!(!has("apikey a1b2 c3 a1b2"), "a secret the prompt never carries was fingerprinted");
    // Fix round 6, T2: the words the raw token folds to (the run above never occurs even raw).
    assert!(!has("c3a1b2") && !has("apikey"), "a secret the prompt never carries was fingerprinted");
}

/// Fix round 6, J1: a pending email draft (a weekly digest's) whose body names the client only at
/// the start of a line, behind JSON's escaped newlines, is deleted with the client. So is a recap
/// run that names it only in its `output_json`, with its unsent sync op, whose payload carries
/// that JSON escaped twice. Controls naming only another client stay.
#[test]
fn rows_naming_the_client_after_an_escaped_newline_go_with_the_client() {
    let fx = fixture(true);
    fx.s.set_own_device_id("dev-a");
    let initech = fx.s.create_seed_topic("Initech", &[], None).unwrap();
    fx.s.seed_builtin_routines().unwrap();
    let routine = fx.s.list_routines().unwrap()[0].id;
    let draft = |body: &str| {
        let action = serde_json::json!({"kind": "send_email", "to": "me@example.com", "subject": "Weekly Digest", "body": body});
        fx.s.insert_pending_routine_run(&memory::NewPendingRoutineRun {
            routine_id: routine,
            ran_at_unix_ms: 1,
            output_summary: "Wants to email \"Weekly Digest\" to me@example.com".into(),
            pending_action_json: action.to_string(),
        })
        .unwrap()
    };
    let recap = |loops: &str| {
        let json = serde_json::json!({"summary": "A week.", "open_loops": [loops]}).to_string();
        fx.s.insert_routine_run(&memory::NewRoutineRun { routine_id: routine, ran_at_unix_ms: 2, status: memory::RunStatus::Success, output_summary: "A week.".into(), output_json: json })
            .unwrap()
    };
    let gone_draft = draft("Week.\n\nInitech: kumquat terms signed.");
    let kept_draft = draft("Week.\n\nHooli: quince terms signed.");
    let gone_run = recap("Chase\nInitech for the papaya deck");
    let kept_run = recap("Chase\nHooli for the guava deck");
    let ops = |word: &str| -> i64 {
        fx.c.query_row("SELECT COUNT(*) FROM sync_outbox WHERE instr(payload_json, ?1) > 0 AND state IN ('pending', 'blocked')", [word], |r| r.get(0)).unwrap()
    };
    assert!(ops("papaya") > 0 && ops("guava") > 0, "control: both recaps queued a sync op");
    let sel = crate::client_delete::DeleteSelection { key: initech.key.clone(), include_possible: Vec::new(), delete_exports: false, vault: None, vault_off: false };
    crate::client_delete::delete_client(&fx.s, &sel, &[], None).unwrap();
    let exists = |id: i64| -> bool { fx.c.query_row("SELECT COUNT(*) FROM routine_run WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap() > 0 };
    assert!(!exists(gone_draft), "the draft naming Initech after an escaped newline survived");
    assert!(!exists(gone_run), "the recap naming Initech in its output_json survived");
    assert_eq!(ops("papaya"), 0, "its sync op survived");
    assert!(exists(kept_draft) && exists(kept_run) && ops("guava") > 0, "control: the other client's rows and op stay");
}

/// Fix round 6, J2: a meeting whose only mention of the client is an edited decision on its own
/// line (stored as JSON, so behind an escaped newline) is the client's: its transcript stays out
/// of hosted Ask. Control: before the edit, it goes.
#[test]
fn a_meeting_named_only_behind_a_json_escape_stays_out() {
    let fx = fixture(true);
    let m = meeting(&fx.s, "Call", &[], crate::util::now_unix_ms() - 900_000, &[&format!("The {CANARY} quote stands.")], "");
    let warm = Log::default();
    ask(&fx, "what about the quote?", RequestOrigin::AskBar, &warm, None).unwrap();
    assert!(warm.prompts()[0].contains(CANARY), "control");
    fx.s.save_meeting_note_overrides(m, None, Some(&["Price agreed.\nAcme signs Friday".to_string()]), None, 1).unwrap();
    let raw: String = fx.c.query_row("SELECT decisions_edited_json FROM meeting_note_draft WHERE meeting_id = ?1", [m], |r| r.get(0)).unwrap();
    assert!(raw.contains("\\nAcme"), "the name sits behind an escape: {raw}");
    let log = Log::default();
    ask(&fx, "what about the quote?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(!log.prompts()[0].contains(CANARY), "{}", log.prompts()[0]);
}

/// Fix round 5, N3 (test added in round 6): every user turn of the history is recorded as an input
/// of the answer's stamp (a live draft's meeting background goes in one).
#[test]
fn user_turns_are_recorded_inputs() {
    let fx = fixture(true);
    let filter = crate::exclusion::context_filter(&fx.s);
    let background = "Background, the last minutes of the meeting:\n[mic] the okapi terms";
    let history = [llm::ChatTurn { role: llm::ChatRole::User, text: background.into() }];
    let stamp = crate::ask::clean_stamp(&filter, &fx.s, &[], [], &["draft my reply"], &history).unwrap();
    let (turn_ref, ..) = memory::topics::exclusion::input_text_ref(background);
    assert!(stamp.refs().contains(&turn_ref), "{:?}", stamp.refs());
    assert!(stamp.kept_words().iter().any(|(h, _)| turn_ref.ends_with(&h.to_string())), "its fingerprints are kept while a client is excluded");
}

/// Fix round 4, S2 (window text): a window Ask's own text has no stored row. It is recorded with
/// the answer and read again by name: once the client the window named is excluded, a hosted
/// follow-up does not carry that answer. Control: before then it does. The client has no stored
/// material, so only the window's recorded words can tie the answer to it.
#[test]
fn a_window_answer_whose_text_names_a_client_excluded_later_is_not_clean() {
    let fx = fixture(true);
    // A client with no stored material: only the window's words tie the answer to it.
    let initech = fx.s.create_seed_topic("Initech", &[], None).unwrap();
    let input = crate::window_ask::WindowInput {
        app_name: "Mail".into(),
        title: "Inbox".into(),
        text: "From Initech: the renewal is due soon.".into(),
        is_selection: false,
    };
    let prepared = crate::window_ask::prepare_window_ask(&fx.s, &input, "when is it due?", &[], &hosted_routing()).unwrap();
    crate::ask::finalize_ask(&fx.s, &prepared.prepare_id, "It is due on Friday.", "claude-test", llm::TokenUsage::default());
    let history = [
        llm::ChatTurn { role: llm::ChatRole::User, text: "when is it due?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: "It is due on Friday.".into() },
    ];
    let (h, l) = (Log::default(), Log::default());
    let follow_up = || {
        crate::ask::answer_with(&fx.s, "and who signs?", &RequestOrigin::AskBar, 10, &history, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Pat does.")], || vec![local(&l, "Pat does.")])
    };
    follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 0), "control: clean while Initech is not excluded");
    fx.s.set_client_control(&initech.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 1), "the window it was built from named Initech");
}

/// Fix round 4, S2: a hosted follow-up after the user captured more of the excluded client's
/// material (not among the first answer's inputs) still goes hosted with its history. Control:
/// once an input of the first answer becomes an excluded client's, the follow-up does not go.
#[test]
fn a_follow_up_after_unrelated_client_captures_stays_hosted() {
    let fx = fixture(true);
    let now = crate::util::now_unix_ms();
    let first = Log::default();
    crate::ask::answer_with(&fx.s, "what about the pricing review?", &RequestOrigin::AskBar, 10, &[], &RoutingConfig::default(), || vec![hosted(&fx.s, &first, "The giraffe review is next week.")], Vec::new).unwrap();
    assert!(first.prompts()[0].contains(OTHER), "the answer was built from Globex's meeting");
    let history = [
        llm::ChatTurn { role: llm::ChatRole::User, text: "what about the pricing review?".into() },
        llm::ChatTurn { role: llm::ChatRole::Assistant, text: "The giraffe review is next week.".into() },
    ];
    for i in 0..3 {
        fx.s.insert_capture_item(&memory::NewCaptureItem { device_id: "d".into(), ts_unix_ms: now + i, app_bundle_id: "com.mail".into(), window_title: "Mail".into(), url: None, text: format!("Acme contract redline {i}"), hash: 900 + i, source: "ax".into() }).unwrap();
    }
    let (h, l) = (Log::default(), Log::default());
    let follow_up = || {
        crate::ask::answer_with(&fx.s, "and who runs it?", &RequestOrigin::AskBar, 10, &history, &RoutingConfig::default(), || vec![hosted(&fx.s, &h, "Carol does.")], || vec![local(&l, "Carol does.")])
    };
    let (_, outcome) = follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 0), "went hosted: {outcome:?}");
    assert!(h.prompts()[0].contains("The giraffe review is next week."), "with its history");
    assert_clean(&h);
    // Control: Globex, whose meeting the first answer was built from, is excluded.
    fx.s.set_client_control(&globex_key(&fx), &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    let (_, outcome) = follow_up().unwrap();
    assert_eq!((h.count(), l.count()), (1, 1), "an input became an excluded client's: {outcome:?}");
}

/// Fix round 3 (N2 through a transport): a slide of a name-hit meeting and (fix round 4, M5) a
/// thread sibling of an email naming the client stay out of hosted Ask. Controls go.
#[test]
fn slides_and_thread_siblings_stay_out_of_hosted_ask() {
    let fx = fixture(true);
    let named = meeting(&fx.s, "Catch-up", &[], crate::util::now_unix_ms() - 900_000, &["Thanks, Acme."], "");
    let slide = |meeting_id: i64, text: &str| {
        fx.c.execute("INSERT INTO meeting_snapshot (meeting_id, captured_unix_ms, file_path, width, height, ocr_text) VALUES (?1, 1, 'f.png', 1, 1, ?2)", rusqlite::params![meeting_id, text]).unwrap();
        let snap = fx.c.last_insert_rowid();
        fx.c.execute(
            "INSERT INTO items (device_id, kind, source, source_table, source_id, thread_id, ts_unix_ms, body_text) VALUES ('d', 'snapshot', 'slides', 'meeting_snapshot', ?1, ?2, ?3, ?4)",
            rusqlite::params![snap, meeting_id, crate::util::now_unix_ms() - 800_000, text],
        )
        .unwrap();
    };
    slide(named, &format!("{CANARY} targets"));
    slide(fx.other_meeting, "giraffe targets");
    // Fix round 4, M5: a thread sibling. One email of the thread names the client; its sibling,
    // from someone else, never does. A thread of its own is the control.
    let email = |ext: &str, thread: &str, from: &str, body: &str| memory::NewEmailItem {
        device_id: "d".into(),
        external_id: ext.into(),
        ts_unix_ms: crate::util::now_unix_ms() - 700_000,
        from: from.into(),
        to: "me@example.com".into(),
        subject: "Quarter plan".into(),
        body_text: body.into(),
        gmail_thread_id: thread.into(),
        account_email: "me@example.com".into(),
        ..Default::default()
    };
    fx.s.upsert_email_items(&[
        email("e1", "th-1", "Legal <legal@law.example>", "Notes from the Acme call."),
        email("e2", "th-1", "Pat <pat@law.example>", &format!("The {CANARY} sibling targets.")),
        email("e3", "th-2", "Pat <pat@law.example>", "The okapi sibling targets."),
    ])
    .unwrap();
    fx.s.run_jobs_batch(10, 5_000).unwrap();
    let log = Log::default();
    ask(&fx, "targets", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(log.prompts()[0].contains("giraffe targets"), "control slide");
    assert!(log.prompts()[0].contains("okapi sibling targets"), "control thread");
    assert_clean(&log);
}

/// Fix round 3 (N6 through a transport): a series repoint (a `topic_source` update) that makes
/// another meeting the client's is seen by the very next hosted Ask, though the cache was warm.
#[test]
fn a_topic_source_repoint_reaches_the_next_hosted_ask() {
    let fx = fixture(true);
    let later = meeting(&fx.s, "Sync", &[], crate::util::now_unix_ms() - 600_000, &[&format!("The {CANARY} plan for next week.")], "");
    let warm = Log::default();
    ask(&fx, "what is the plan for next week?", RequestOrigin::AskBar, &warm, None).unwrap();
    assert!(warm.prompts()[0].contains(&format!("{CANARY} plan")), "control: not yet the client's");
    fx.c.execute("UPDATE topic_source SET ref_id = ?1 WHERE kind = 'meeting' AND ref_id = ?2", [later, fx.acme_meeting]).unwrap();
    let log = Log::default();
    ask(&fx, "what is the plan for next week?", RequestOrigin::AskBar, &log, None).unwrap();
    assert!(!log.prompts()[0].contains(&format!("{CANARY} plan")), "{}", log.prompts()[0]);
}

/// Fix round 3 (N8 through a transport): with the live meeting unreadable, detect is judged
/// locally. Control: a readable non-client setup goes hosted.
#[test]
fn an_unreadable_live_meeting_is_judged_locally() {
    let fx = fixture(true);
    fx.c.execute("UPDATE meeting SET device_id = X'00ff' WHERE id = ?1", [fx.other_meeting]).unwrap();
    let reply = r#"{"is_for_user":false,"confidence":"low","quote":"q","asker":null,"restatement":"r"}"#;
    let (h, l) = (Log::default(), Log::default());
    let origin = crate::live_question::live_origin(&fx.s, None);
    crate::live_question::detect_with(&fx.s, "Mike, can you ship Friday?", &origin, || vec![hosted(&fx.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!((h.count(), l.count()), (0, 1));
    let ok = fixture(true);
    let origin = crate::live_question::live_origin(&ok.s, None);
    crate::live_question::detect_with(&ok.s, "Mike, can you ship Friday?", &origin, || vec![hosted(&ok.s, &h, reply)], || vec![local(&l, reply)]).unwrap();
    assert_eq!(h.count(), 1, "control");
}

/// Fix round 3, R9: the daily brief's events carry their calendar id: a retitled, moved instance
/// of the client's event is left out by id. Control: another event stays.
#[test]
fn the_daily_briefs_events_are_matched_by_calendar_id() {
    let fx = fixture(true);
    fx.c.execute(
        "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) VALUES ('d', 'calendar_event', 'gcal', 90000000, '[]', 'Budget review', 'Budget review', 'evt-acme')",
        [],
    )
    .unwrap();
    file(&fx.c, fx.acme_topic, "calendar_event", fx.c.last_insert_rowid());
    let events = [
        routines::CalendarEventSummary { id: Some("evt-acme".into()), title: "Renamed".into(), start_unix_ms: 95_000_000 },
        routines::CalendarEventSummary { id: Some("evt-other".into()), title: "Lunch".into(), start_unix_ms: 91_000_000 },
    ];
    let kept = crate::exclusion::events_without_excluded(&fx.s, &events, &llm::ContextFilter::none());
    assert_eq!(kept.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["Lunch"]);
}

// ---- The sweep: every hosted caller, each with its test ---------------------------------

/// Calls that build or send a prompt a hosted model can receive, or that feed retrieval into
/// one. Each site below names the test that proves it keeps an excluded client's material out.
const CALLERS: [&str; 33] = [
    "build_chain_for(",
    "build_chain(",
    "build_chain_for_routine(",
    "build_light_chain(",
    "TypeSafeClient::new(",
    "OllamaClient::new(",
    "GuardedClient::new(",
    "gather_and_assemble_context(",
    "gather_and_assemble_context_for_window(",
    "routines::run_daily_brief(",
    "routines::run_eod_recap(",
    "routines::run_weekly_digest(",
    // Main's Limited Use entry points (the same routines, told whether the link may get Google data).
    "routines::run_daily_brief_with(",
    "routines::run_eod_recap_with(",
    "routines::run_weekly_digest_with(",
    "meeting_prep::run_with_chain(",
    // The device's prep with the cloud policy (Task 6b): the same chain and gate as `run_with_chain`.
    "meeting_prep::run_with_chain_cloud(",
    "inference::start(",
    "generate_daily_digest(",
    "llm_client.as_ref()",
    // A retrieval that chose to keep everything: only for prompts that stay on this computer.
    "ContextFilter::none()",
    // An MCP tool: every one returns stored material to outside AI tools.
    "#[tool(",
    // Fix round 1, M3: the whimbrel stream, Ask used as a building block, the TypeSafe title check.
    "start_whimbrel(",
    "ask::answer(",
    "ask::answer_detailed(",
    "answer_from(",
    "answer_from_filter(",
    "check_calendar_title(",
    // Merge 3 (main's #816 and the forced re-summary): meeting Ask reads a long transcript part
    // by part on a link, and "Re-summarize with Whimbrel" pins the preferred link.
    "gather_findings(",
    "streamed_link(",
    "summarize_meeting_preferred(",
    // The cloud runner's entries: the same routines and prep, each link's client made by a transport.
    "tick_cloud_with_transport(",
    "run_with_chain_transport(",
];

/// (file, call, count, the test that covers it). A new call anywhere fails until it is covered
/// and listed here.
const COVERED: &[(&str, &str, usize, &str)] = &[
    // Streams: hosted ones behind the gate; an Ollama cloud one refuses prompts a prepare marked.
    ("apps/windows/src-tauri/src/chat/mod.rs", "inference::start(", 1, "an_ollama_cloud_stream_is_gated"),
    ("core/ffi/src/inference_ffi.rs", "inference::start(", 1, "an_ollama_cloud_stream_is_gated"),
    // Skin chat: the user's own words, no retrieval; hosted (Ollama cloud too) behind the gate.
    ("apps/windows/src-tauri/src/commands/skins.rs", "GuardedClient::new(", 1, "skin_calls_naming_an_excluded_client_are_refused_and_charge_nothing"),
    ("apps/windows/src-tauri/src/commands/skins.rs", "OllamaClient::new(", 1, "skin_calls_naming_an_excluded_client_are_refused_and_charge_nothing"),
    ("core/ffi/src/lib.rs", "GuardedClient::new(", 1, "skin_calls_naming_an_excluded_client_are_refused_and_charge_nothing"),
    ("core/ffi/src/lib.rs", "OllamaClient::new(", 1, "skin_calls_naming_an_excluded_client_are_refused_and_charge_nothing"),
    // Meeting prep through `run_with_chain`.
    ("core/ffi/src/lib.rs", "meeting_prep::run_with_chain(", 1, "prep_never_sends_excluded_text"),
    // Windows' prep with the cloud policy (Task 7b; `None` is `run_with_chain`): the same chain and gate.
    ("apps/windows/src-tauri/src/routines.rs", "meeting_prep::run_with_chain_cloud(", 1, "prep_never_sends_excluded_text"),
    ("core/ffi/src/cloud_routines_ffi.rs", "meeting_prep::run_with_chain_cloud(", 1, "prep_never_sends_excluded_text"),
    ("core/api/src/meeting_prep.rs", "build_chain_for(", 1, "prep_never_sends_excluded_text"),
    // TypeSafe: topic judge and steps (Windows, ffi, the review example), priorities, titles.
    ("apps/windows/src-tauri/src/topics.rs", "TypeSafeClient::new(", 2, "topics_judge_never_sends_excluded_text_and_excluded_sources_do_not_stall_the_queue"),
    ("core/ffi/src/topics_ffi.rs", "TypeSafeClient::new(", 2, "topics_steps_never_send_excluded_text_and_excluded_sources_do_not_stall_the_queue"),
    ("core/api/examples/topics_review.rs", "TypeSafeClient::new(", 1, "topics_judge_never_sends_excluded_text_and_excluded_sources_do_not_stall_the_queue"),
    ("core/ffi/src/lib.rs", "TypeSafeClient::new(", 1, "unattended_jobs_never_send_excluded_text"),
    ("core/api/src/title_check.rs", "TypeSafeClient::new(", 1, "title_check_never_sends_excluded_text"),
    // Ask: the answer, the meeting answer, conversation titles; retrieval for both.
    ("core/api/src/ask.rs", "build_chain_for(", 4, "meeting_ask_and_titles_never_send_excluded_text"),
    ("core/api/src/ask.rs", "gather_and_assemble_context(", 1, "ask_never_sends_excluded_text"),
    ("core/api/src/ask.rs", "gather_and_assemble_context_for_window(", 1, "ask_never_sends_excluded_text"),
    ("core/api/src/window_ask.rs", "gather_and_assemble_context(", 1, "window_ask_never_sends_excluded_text"),
    ("core/api/src/live_question.rs", "build_chain_for(", 1, "live_question_never_sends_excluded_text"),
    // Day-level callers.
    ("core/api/src/digest.rs", "build_chain_for(", 1, "digest_never_sends_excluded_text"),
    ("core/api/src/digest.rs", "gather_and_assemble_context_for_window(", 1, "digest_never_sends_excluded_text"),
    ("core/api/src/routine_runs.rs", "build_chain_for_routine(", 1, "routines_never_send_excluded_text"),
    ("core/api/src/routine_runs.rs", "routines::run_daily_brief_with(", 1, "routines_never_send_excluded_text"),
    ("core/api/src/routine_runs.rs", "routines::run_eod_recap_with(", 1, "routines_never_send_excluded_text"),
    ("core/api/src/routine_runs.rs", "routines::run_weekly_digest_with(", 1, "routines_never_send_excluded_text"),
    ("core/routines/src/lib.rs", "gather_and_assemble_context_for_window(", 3, "routines_never_send_excluded_text"),
    ("core/api/src/profile_suggest.rs", "build_chain_for(", 1, "profile_suggest_never_sends_excluded_text"),
    // Meetings.
    ("core/api/src/summary.rs", "build_chain_for(", 1, "summary_never_sends_excluded_text"),
    ("core/api/src/speaker_names.rs", "build_chain_for(", 1, "speaker_names_never_send_excluded_text"),
    // Topic names.
    ("core/api/src/topics.rs", "build_chain_for(", 1, "topic_naming_never_sends_excluded_text"),
    // Unattended Ollama jobs: an Ollama cloud model is behind the gate and skips the client.
    ("core/api/src/jobs.rs", "GuardedClient::new(", 1, "unattended_jobs_never_send_excluded_text"),
    ("core/api/src/jobs.rs", "OllamaClient::new(", 1, "unattended_jobs_never_send_excluded_text"),
    // The chain builders themselves: an Ollama cloud link is hosted.
    ("core/api/src/routing.rs", "OllamaClient::new(", 2, "ollama_cloud_models_are_treated_as_hosted"),
    ("core/api/src/routing.rs", "build_chain(", 6, "ollama_cloud_models_are_treated_as_hosted"),
    ("core/api/src/routing.rs", "build_light_chain(", 2, "ollama_cloud_models_are_treated_as_hosted"),
    // The gate's own wrapper.
    ("core/llm/src/guard.rs", "GuardedClient::new(", 1, "guarded_client_refuses_prompt_naming_excluded_client_without_calling_inner"),
    // The MCP server: every tool result is treated as hosted.
    ("core/mcp-server/src/bin/whimbrel-mcp-server.rs", "OllamaClient::new(", 1, "mcp_tools_never_return_excluded_text"),
    ("core/mcp-server/src/lib.rs", "gather_and_assemble_context(", 1, "mcp_tools_never_return_excluded_text"),
    ("core/mcp-server/src/lib.rs", "generate_daily_digest(", 1, "mcp_tools_never_return_excluded_text"),
    ("core/mcp-server/src/lib.rs", "llm_client.as_ref()", 1, "mcp_tools_never_return_excluded_text"),
    // Every MCP tool (a new one fails here until it filters and is tested).
    ("core/mcp-server/src/lib.rs", "#[tool(", 7, "mcp_tools_never_return_excluded_text"),
    // Retrieval that keeps everything, only where the prompt stays on this computer: Ask's
    // local-only and on-device links, a stream to a local model (its prompt is then marked
    // local-only), an all-local chain, an on-device Ollama job, unpaid (local) step reads.
    ("core/api/src/ask.rs", "ContextFilter::none()", 3, "request_naming_client_goes_local_only"),
    ("core/api/src/window_ask.rs", "ContextFilter::none()", 1, "window_ask_never_sends_excluded_text"),
    ("core/api/src/exclusion.rs", "ContextFilter::none()", 1, "routines_never_send_excluded_text"),
    ("core/api/src/jobs.rs", "ContextFilter::none()", 1, "unattended_jobs_never_send_excluded_text"),
    ("core/api/src/topic_steps.rs", "ContextFilter::none()", 2, "topics_steps_never_send_excluded_text_and_excluded_sources_do_not_stall_the_queue"),
    // Fix round 1, M3: the whimbrel stream, Ask as a building block, the TypeSafe title check.
    ("apps/windows/src-tauri/src/chat/mod.rs", "start_whimbrel(", 1, "a_hosted_stream_naming_an_excluded_client_is_never_sent"),
    ("core/ffi/src/inference_ffi.rs", "start_whimbrel(", 1, "a_hosted_stream_naming_an_excluded_client_is_never_sent"),
    // The blocking Ask that reports the answering model (Windows since PR 874 parity; the Mac's ffi entry).
    ("apps/windows/src-tauri/src/commands/chat.rs", "ask::answer_detailed(", 1, "answer_detailed_never_sends_excluded_text"),
    ("core/ffi/src/lib.rs", "ask::answer_detailed(", 1, "answer_detailed_never_sends_excluded_text"),
    ("core/ffi/src/lib.rs", "ask::answer(", 1, "ask_never_sends_excluded_text"),
    ("core/eval/src/main.rs", "ask::answer(", 1, "ask_never_sends_excluded_text"),
    ("core/api/src/ask.rs", "answer_from(", 1, "request_naming_client_goes_local_only"),
    ("core/api/src/ask.rs", "answer_from_filter(", 1, "request_naming_client_goes_local_only"),
    ("core/api/src/live_question.rs", "answer_from_filter(", 1, "live_questions_in_the_clients_meeting_stay_local"),
    ("core/api/src/summary.rs", "check_calendar_title(", 1, "title_check_never_sends_excluded_text"),
    // The retrieval eval (a developer tool) retrieves as a hosted Ask does.
    ("core/eval/src/main.rs", "build_chain_for(", 1, "ask_never_sends_excluded_text"),
    ("core/eval/src/retrieval.rs", "gather_and_assemble_context(", 1, "ask_never_sends_excluded_text"),
    ("core/eval/src/retrieval.rs", "gather_and_assemble_context_for_window(", 1, "ask_never_sends_excluded_text"),
    // Merge 3: meeting Ask's part-by-part reading (blocking and streamed) and the forced re-summary.
    ("core/api/src/ask.rs", "gather_findings(", 2, "meeting_ask_map_step_never_sends_excluded_text"),
    ("core/api/src/ask.rs", "streamed_link(", 1, "meeting_ask_map_step_never_sends_excluded_text"),
    ("core/ffi/src/lib.rs", "summarize_meeting_preferred(", 2, "forced_resummary_never_sends_excluded_text"),
    ("apps/windows/src-tauri/src/meetings/pipeline.rs", "summarize_meeting_preferred(", 1, "forced_resummary_never_sends_excluded_text"),
    // The cloud runner: routines and meeting prep on a scratch store it filed after its pull.
    ("core/api/src/routine_runs.rs", "tick_cloud_with_transport(", 1, "a_substituted_transport_stays_behind_the_exclusion_gate"),
    ("core/cloud-runner/src/run.rs", "tick_cloud_with_transport(", 1, "an_item_filed_under_an_excluded_client_but_not_naming_it_is_left_out_of_a_briefs_context_on_the_runner"),
    ("core/cloud-runner/src/run.rs", "run_with_chain_transport(", 1, "runner_prep_never_sends_excluded_text"),
];

/// Fix round 1, M3: each covering test must call the real caller (or the function it is a thin
/// wrapper over), not just exist. (test, `|`-separated calls its body must make)
const EXERCISES: &[(&str, &str)] = &[
    ("an_ollama_cloud_stream_is_gated", "start("),
    ("a_hosted_stream_naming_an_excluded_client_is_never_sent", "start_whimbrel_at(|start("),
    ("skin_calls_naming_an_excluded_client_are_refused_and_charge_nothing", "whimbrel_skin_turn("),
    ("prep_never_sends_excluded_text", "run_gated_with("),
    ("topics_judge_never_sends_excluded_text_and_excluded_sources_do_not_stall_the_queue", "run_topic_judge_batch(|route = 'topic_judge'"),
    ("topics_steps_never_send_excluded_text_and_excluded_sources_do_not_stall_the_queue", "run_topic_steps_batch("),
    ("unattended_jobs_never_send_excluded_text", "run_thread_priority_batch(|ollama_client("),
    ("title_check_never_sends_excluded_text", "check_calendar_title_with("),
    ("meeting_ask_and_titles_never_send_excluded_text", "answer_meeting_with(|generate_conversation_title_with(|prepare_meeting_ask(|assert_reached(&fx.c, \"ask\")|assert_reached(&fx.c, \"conversation_title\")"),
    ("ask_never_sends_excluded_text", "ask(|prepare_ask(|prepare_ask_for(|assert_reached(&fx.c, \"ask\")"),
    ("window_ask_never_sends_excluded_text", "prepare_window_ask(|prepare_window_ask_for("),
    ("live_question_never_sends_excluded_text", "detect_with(|assert_reached(&fx.c, \"live_question\")"),
    ("digest_never_sends_excluded_text", "daily_digest_with(|assert_reached(&fx.c, \"digest\")"),
    ("routines_never_send_excluded_text", "run_daily_brief(|run_eod_recap(|run_weekly_digest("),
    ("profile_suggest_never_sends_excluded_text", "run_with_prompts("),
    ("summary_never_sends_excluded_text", "summarize_meeting_routed(|summarize_meeting_with(|assert_reached(&fx.c, \"meeting_summary\")"),
    ("speaker_names_never_send_excluded_text", "assign_meeting_speakers_with("),
    ("topic_naming_never_sends_excluded_text", "run_model_naming_batch_with_complete("),
    ("ollama_cloud_models_are_treated_as_hosted", "build_chain(|build_local_chain("),
    ("guarded_client_refuses_prompt_naming_excluded_client_without_calling_inner", "GuardedClient::new("),
    ("mcp_tools_never_return_excluded_text", "search_memory(|get_timeline(|get_daily_digest(|assemble_context(|get_meeting("),
    ("request_naming_client_goes_local_only", "ask("),
    ("answer_detailed_never_sends_excluded_text", "answer_detailed(|answer_with_filter_by(|assert_reached(&fx.c, \"ask\")"),
    ("a_local_answer_never_rides_to_a_hosted_model_as_history_or_title", "answer_with(|generate_conversation_title_with(|finalize_ask("),
    ("live_questions_in_the_clients_meeting_stay_local", "live_origin(|detect_with("),
    ("routine_runs_without_clean_provenance_stay_out_of_hosted_prompts", "provenance_for("),
    ("a_brief_built_from_a_meeting_later_filed_under_the_client_is_not_clean", "run_daily_brief(|provenance_for("),
    ("meeting_ask_map_step_never_sends_excluded_text", "answer_meeting_with(|prepare_meeting_ask_with(|part 1 of|assert_reached(&fx.c, \"ask\")"),
    ("forced_resummary_never_sends_excluded_text", "summarize_meeting_routed_as(|summarize_meeting_preferred(|assert_reached(&fx.c, \"meeting_summary\")"),
    ("a_substituted_transport_stays_behind_the_exclusion_gate", "tick_cloud_with_transport("),
    ("an_item_filed_under_an_excluded_client_but_not_naming_it_is_left_out_of_a_briefs_context_on_the_runner", "run_once_with(|client_peer(&relay, true"),
    ("runner_prep_never_sends_excluded_text", "run_once_with(|file_by_verdict("),
];

/// The body of test `name` in `source`: from its `fn` line to the next test.
fn test_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let start = source.find(&format!("fn {name}("))?;
    let rest = &source[start..];
    let end = rest[1..].find("#[test]").map_or(rest.len(), |i| i + 1);
    Some(&rest[..end])
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `path`'s source without its top-level test modules (as `routing`'s scan cuts them).
fn non_test_source(path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let mut kept = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let test_mod = line == "#[cfg(test)]" && lines.peek().is_some_and(|next| next.starts_with("mod ") && next.ends_with('{'));
        if test_mod {
            lines.by_ref().find(|l| *l == "}");
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

#[test]
fn every_hosted_caller_is_covered() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let mut files = Vec::new();
    let mut dirs = vec![repo.join("apps/windows/src-tauri/src")];
    for krate in std::fs::read_dir(repo.join("core")).unwrap() {
        let krate = krate.unwrap().path();
        dirs.extend(["src", "examples"].map(|d| krate.join(d)));
    }
    for dir in dirs.iter().filter(|d| d.is_dir()) {
        rust_files(dir, &mut files);
    }
    assert!(files.len() > 100, "the scan found the sources ({} files)", files.len());
    let mut all_source = String::new();
    let mut found = std::collections::BTreeMap::new();
    for path in &files {
        all_source.push_str(&std::fs::read_to_string(path).unwrap());
        // This file is test code (a `#[cfg(test)] mod` declared in lib.rs).
        if path.ends_with("exclusion_sweep.rs") {
            continue;
        }
        let source = non_test_source(path);
        let rel = path.strip_prefix(repo).unwrap().to_string_lossy().replace('\\', "/");
        for call in CALLERS {
            let bare = call.trim_end_matches('(');
            let defs = source.matches(&format!("fn {bare}(")).count();
            // `x::build_chain(` also contains `build_chain(`; count each call once, by its
            // longest name.
            let longer: usize = CALLERS
                .iter()
                .filter(|c| **c != call && c.ends_with(call))
                .map(|c| source.matches(c).count())
                .sum();
            let hits = source.matches(call).count().saturating_sub(defs + longer);
            if hits > 0 {
                found.insert((rel.clone(), call), hits);
            }
        }
    }
    let listed: std::collections::BTreeMap<_, _> = COVERED.iter().map(|&(f, c, n, _)| ((f.to_string(), c), n)).collect();
    for (_, _, _, test) in COVERED {
        let body = test_body(&all_source, test).unwrap_or_else(|| panic!("the covering test {test} exists"));
        let (_, calls) = EXERCISES.iter().find(|(t, _)| t == test).unwrap_or_else(|| panic!("{test} is listed in EXERCISES"));
        for call in calls.split('|') {
            assert!(body.contains(call), "the covering test {test} calls {call}");
        }
    }
    assert_eq!(found, listed, "a hosted caller is not covered: keep excluded material out of it, test it, then list it here");
}
