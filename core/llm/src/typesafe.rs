//! TypeSafe client for Reply Priorities: one request carries the five questions
//! (docs/superpowers/specs/2026-10-05-reply-priorities-design.md).

use memory::priority::PriorityJudgments;
use serde_json::{json, Value};

use super::{ureq_error_to_llm, LlmError, TokenUsage};

/// Already redacted and truncated by the caller (`api::jobs`).
#[derive(Debug, Clone, PartialEq)]
pub struct PriorityInput {
    pub subject: String,
    pub sender_name: String,
    pub sender_domain: String,
    pub latest_message: String,
}

pub trait PriorityScorer {
    fn score(&self, input: &PriorityInput) -> Result<PriorityJudgments, LlmError>;
}

pub struct TypeSafeClient {
    base_url: String,
    api_key: String,
}

const TYPESAFE_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const TYPESAFE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// The title check runs synchronously inside summarize, so it gives up sooner.
const TITLE_MATCH_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

impl TypeSafeClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url("https://api.typesafe.ai", api_key)
    }

    pub fn with_base_url(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), api_key: api_key.into() }
    }
}

fn questions() -> Value {
    json!({
        "asks_reply": {
            "type": "noul",
            "instructions": "Does `latest_message` ask the recipient to reply, answer a question, make a decision, or take an action?"
        },
        "urgency": {
            "type": "score",
            "instructions": "How time-sensitive is it for the recipient to respond to `latest_message`?",
            "criteria": [
                "No time pressure",
                "Should respond within about a week",
                "Should respond within a day or two",
                "Needs a response now, or a deadline has passed or is today"
            ]
        },
        "person": {
            "type": "noul",
            "instructions": "Was `latest_message` written by a real individual to this recipient personally, as opposed to a company, a system, or a mailing list?"
        },
        "bulk": {
            "type": "noul",
            "instructions": "Is `latest_message` bulk, automated or mass-marketing mail sent to many recipients at once?"
        },
        "automated": {
            "type": "noul",
            "instructions": "Is `latest_message` an automated account, security, sign-in, billing or system notification that informs the recipient and does not need a personal response?"
        }
    })
}

pub fn build_request(input: &PriorityInput) -> Value {
    json!({
        "state": {
            "subject": input.subject,
            "sender_name": input.sender_name,
            "sender_domain": input.sender_domain,
            "latest_message": input.latest_message,
        },
        "model": "jev-latest",
        "questions": questions(),
    })
}

/// Rejects NaN and infinities so the priority formula only ever sees finite numbers.
fn finite(id: &str, v: f64) -> Result<f64, LlmError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(LlmError::Provider(format!("TypeSafe answer {id} is not a finite number")))
    }
}

fn answer(body: &Value, id: &str, field: &str) -> Result<f64, LlmError> {
    let v = body["answers"][id][field]
        .as_f64()
        .ok_or_else(|| LlmError::Provider(format!("TypeSafe response is missing answers.{id}.{field}")))?;
    finite(id, v)
}

pub fn parse_response(body: &Value) -> Result<PriorityJudgments, LlmError> {
    Ok(PriorityJudgments {
        asks_reply: answer(body, "asks_reply", "noul")?.clamp(0.0, 1.0),
        urgency: answer(body, "urgency", "score")?.clamp(0.0, 3.0),
        person: answer(body, "person", "noul")?.clamp(0.0, 1.0),
        bulk: answer(body, "bulk", "noul")?.clamp(0.0, 1.0),
        automated: answer(body, "automated", "noul")?.clamp(0.0, 1.0),
    })
}

impl TypeSafeClient {
    /// POSTs one `/v1/systemone` request and returns the JSON body.
    fn post(&self, body: Value) -> Result<Value, LlmError> {
        self.post_with_read_timeout(body, TYPESAFE_READ_TIMEOUT)
    }

    fn post_with_read_timeout(&self, body: Value, read_timeout: std::time::Duration) -> Result<Value, LlmError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(TYPESAFE_CONNECT_TIMEOUT)
            .timeout_read(read_timeout)
            .build();
        let response = agent
            .post(&format!("{}/v1/systemone", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(body)
            .map_err(ureq_error_to_llm)?;
        response
            .into_json()
            .map_err(|e| LlmError::Provider(format!("failed to parse TypeSafe response: {e}")))
    }
}

impl PriorityScorer for TypeSafeClient {
    fn score(&self, input: &PriorityInput) -> Result<PriorityJudgments, LlmError> {
        parse_response(&self.post(build_request(input))?)
    }
}

/// Already redacted by the caller (`api::title_check`).
#[derive(Debug, Clone, PartialEq)]
pub struct TitleMatchInput {
    pub event_title: String,
    pub attendees: Vec<String>,
    pub tldr: String,
    pub decisions: Vec<String>,
}

/// One Noul question: is the summary of the calendar event it was filed under?
pub fn build_title_match_request(input: &TitleMatchInput) -> Value {
    json!({
        "state": {
            "calendar_event": { "title": input.event_title, "attendees": input.attendees },
            "meeting_summary": { "tldr": input.tldr, "decisions": input.decisions },
        },
        "model": "jev-latest",
        "questions": {
            "summary_is_this_event": {
                "type": "noul",
                "instructions": "Is `meeting_summary` a summary of the meeting described by `calendar_event`?",
                "criteria": {
                    "true": "The discussion fits this event: its topic, purpose or participants match, or the event title is too generic to rule it out.",
                    "false": "The discussion is clearly a different meeting: its topic and participants don't fit what the event describes."
                }
            }
        }
    })
}

/// Probability (0..=1) that the summary is of the event.
pub fn parse_title_match(body: &Value) -> Result<f64, LlmError> {
    Ok(answer(body, "summary_is_this_event", "noul")?.clamp(0.0, 1.0))
}

pub trait TitleMatcher {
    fn probability(&self, input: &TitleMatchInput) -> Result<f64, LlmError>;
}

impl TitleMatcher for TypeSafeClient {
    fn probability(&self, input: &TitleMatchInput) -> Result<f64, LlmError> {
        parse_title_match(&self.post_with_read_timeout(build_title_match_request(input), TITLE_MATCH_READ_TIMEOUT)?)
    }
}

/// Already redacted and truncated by the caller. Labels are `(label, description)`;
/// callers pass opaque labels (`t1..tN`) so they never collide with `none`.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicJudgeInput {
    pub kind: String,
    pub text: String,
    pub labels: Vec<(String, String)>,
    pub ask_work: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopicJudgment {
    /// One entry per sent label, in order, then `none`.
    pub probabilities: Vec<(String, f64)>,
    /// P(real work), present only when asked.
    pub work: Option<f64>,
    pub model: String,
    pub usage: Option<TokenUsage>,
}

pub trait TopicJudge {
    fn judge(&self, input: &TopicJudgeInput) -> Result<TopicJudgment, LlmError>;
}

const NONE_LABEL: &str = "none";

pub fn build_topic_request(input: &TopicJudgeInput) -> Value {
    let mut criteria = serde_json::Map::new();
    for (label, description) in &input.labels {
        criteria.insert(label.clone(), json!(description));
    }
    criteria.insert(NONE_LABEL.into(), json!("About none of these topics."));
    let mut questions = json!({
        "topic": {
            "type": "choice",
            "instructions": "Which topic is `item` mainly about? Pick none if it is about none of them.",
            "criteria": criteria,
        }
    });
    if input.ask_work {
        questions["work"] = json!({
            "type": "noul",
            "instructions": "Is `item` real work on a subject (reading, writing, a call), rather than idle time, navigation or app screens?",
            "criteria": {"true": "real work on a subject", "false": "idle, navigation or app chrome"}
        });
    }
    json!({
        "model": "jev-latest",
        "state": {"item": input.text, "kind": input.kind},
        "questions": questions,
    })
}

/// Reads `usage.input_tokens` / `usage.output_tokens`; `None` when neither is present.
pub fn parse_usage(body: &Value) -> Option<TokenUsage> {
    let n = |k: &str| body["usage"][k].as_u64().and_then(|v| u32::try_from(v).ok());
    let usage = TokenUsage { input_tokens: n("input_tokens"), output_tokens: n("output_tokens") };
    (usage != TokenUsage::default()).then_some(usage)
}

pub fn parse_topic_response(
    body: &Value,
    labels: &[(String, String)],
    ask_work: bool,
) -> Result<TopicJudgment, LlmError> {
    let probs = &body["answers"]["topic"]["probabilities"];
    let mut probabilities = Vec::with_capacity(labels.len() + 1);
    for label in labels.iter().map(|(l, _)| l.as_str()).chain([NONE_LABEL]) {
        let p = probs[label].as_f64().ok_or_else(|| {
            LlmError::Provider(format!("TypeSafe response is missing a probability for topic label {label}"))
        })?;
        probabilities.push((label.to_string(), finite(label, p)?));
    }
    let work = if ask_work { Some(answer(body, "work", "noul")?.clamp(0.0, 1.0)) } else { None };
    Ok(TopicJudgment {
        probabilities,
        work,
        model: body["model"].as_str().unwrap_or_default().to_string(),
        usage: parse_usage(body),
    })
}

impl TopicJudge for TypeSafeClient {
    fn judge(&self, input: &TopicJudgeInput) -> Result<TopicJudgment, LlmError> {
        let body = self.post_with_read_timeout(build_topic_request(input), TITLE_MATCH_READ_TIMEOUT)?;
        parse_topic_response(&body, &input.labels, input.ask_work)
    }
}

/// Already redacted and truncated by the caller. `examples` are `(text, reason)` pairs
/// of lines the user said were not steps.
#[derive(Debug, Clone, PartialEq)]
pub struct StepJudgeInput {
    pub kind: String,
    pub me: Vec<String>,
    pub from: Option<String>,
    pub context: String,
    pub lines: Vec<String>,
    pub open_steps: Vec<String>,
    pub examples: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepJudgment {
    /// Per line: `(label, p)` over `mine`, `owed`, `date`, `none`.
    pub lines: Vec<Vec<(String, f64)>>,
    /// P(done) per open step.
    pub closes: Vec<f64>,
    pub model: String,
    pub usage: Option<TokenUsage>,
}

pub trait StepJudge {
    fn judge_steps(&self, input: &StepJudgeInput) -> Result<StepJudgment, LlmError>;
}

const STEP_LABELS: [&str; 4] = ["mine", "owed", "date", "none"];

pub fn build_step_request(input: &StepJudgeInput) -> Value {
    let mut state = serde_json::Map::new();
    state.insert("item".into(), json!(input.context));
    state.insert("kind".into(), json!(input.kind));
    state.insert("me".into(), json!(input.me));
    state.insert("from".into(), json!(input.from));
    let mut questions = serde_json::Map::new();
    let examples: Vec<String> = input
        .examples
        .iter()
        .map(|(t, r)| format!("\"{t}\" ({r})"))
        .collect();
    let none = if examples.is_empty() {
        "None of these.".to_string()
    } else {
        format!("None of these. For example: {}", examples.join(", "))
    };
    for (i, line) in input.lines.iter().enumerate() {
        let n = i + 1;
        state.insert(format!("line_{n}"), json!(line));
        questions.insert(
            format!("c{n}"),
            json!({
                "type": "choice",
                "instructions": format!("What is `line_{n}`?"),
                "criteria": {
                    "mine": format!("`line_{n}` asks one of `me` to do something, or one of `me` promises to do it"),
                    "owed": format!("`line_{n}` has someone other than `me` promise to do something for `me`"),
                    "date": format!("`line_{n}` states a deadline or event date and asks nothing of anyone"),
                    "none": none,
                }
            }),
        );
    }
    for (j, step) in input.open_steps.iter().enumerate() {
        let n = j + 1;
        state.insert(format!("step_{n}"), json!(step));
        questions.insert(
            format!("x{n}"),
            json!({"type": "noul", "instructions": format!("Does `item` say that `step_{n}` has been done?")}),
        );
    }
    json!({"model": "jev-latest", "state": state, "questions": questions})
}

pub fn parse_step_response(
    body: &Value,
    n_lines: usize,
    n_steps: usize,
) -> Result<StepJudgment, LlmError> {
    let mut lines = Vec::with_capacity(n_lines);
    for i in 1..=n_lines {
        let id = format!("c{i}");
        let mut row = Vec::with_capacity(STEP_LABELS.len());
        for label in STEP_LABELS {
            let p = body["answers"][&id]["probabilities"][label]
                .as_f64()
                .ok_or_else(|| {
                    LlmError::Provider(format!(
                        "TypeSafe response is missing answers.{id}.probabilities.{label}"
                    ))
                })?;
            row.push((label.to_string(), finite(&id, p)?));
        }
        lines.push(row);
    }
    let closes = (1..=n_steps)
        .map(|j| Ok(answer(body, &format!("x{j}"), "noul")?.clamp(0.0, 1.0)))
        .collect::<Result<Vec<_>, LlmError>>()?;
    Ok(StepJudgment {
        lines,
        closes,
        model: body["model"].as_str().unwrap_or_default().to_string(),
        usage: parse_usage(body),
    })
}

impl StepJudge for TypeSafeClient {
    fn judge_steps(&self, input: &StepJudgeInput) -> Result<StepJudgment, LlmError> {
        let body =
            self.post_with_read_timeout(build_step_request(input), TITLE_MATCH_READ_TIMEOUT)?;
        parse_step_response(&body, input.lines.len(), input.open_steps.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn input() -> PriorityInput {
        PriorityInput {
            subject: "Action Required".into(),
            sender_name: "Ben Lawrick".into(),
            sender_domain: "alphasights.com".into(),
            latest_message: "Please confirm by Friday.".into(),
        }
    }

    #[test]
    fn the_request_carries_the_state_model_and_all_five_questions() {
        let req = build_request(&input());
        assert_eq!(req["model"], "jev-latest");
        assert_eq!(req["state"]["subject"], "Action Required");
        assert_eq!(req["state"]["sender_name"], "Ben Lawrick");
        assert_eq!(req["state"]["sender_domain"], "alphasights.com");
        assert_eq!(req["state"]["latest_message"], "Please confirm by Friday.");
        let q = &req["questions"];
        for (id, kind) in [("asks_reply", "noul"), ("urgency", "score"), ("person", "noul"), ("bulk", "noul"), ("automated", "noul")] {
            assert_eq!(q[id]["type"], kind, "{id}");
            assert!(q[id]["instructions"].as_str().unwrap().contains("`latest_message`"), "{id}");
        }
        assert_eq!(q["urgency"]["criteria"].as_array().unwrap().len(), 4);
    }

    fn sample_body() -> Value {
        json!({
            "model": "jev-1.13.0",
            "answers": {
                "asks_reply": {"type": "noul", "noul": 0.99},
                "urgency": {"type": "score", "score": 2.2, "confidence": 0.8},
                "person": {"type": "noul", "noul": 0.69},
                "bulk": {"type": "noul", "noul": 0.09},
                "automated": {"type": "noul", "noul": 0.03}
            },
            "usage": {"input_tokens": 300, "output_tokens": 40}
        })
    }

    #[test]
    fn a_full_response_parses_into_judgments() {
        let j = parse_response(&sample_body()).unwrap();
        assert_eq!(j, PriorityJudgments { asks_reply: 0.99, urgency: 2.2, person: 0.69, bulk: 0.09, automated: 0.03 });
    }

    #[test]
    fn a_response_missing_an_answer_is_a_provider_error() {
        let mut body = sample_body();
        body["answers"].as_object_mut().unwrap().remove("automated");
        match parse_response(&body) {
            Err(LlmError::Provider(msg)) => assert!(msg.contains("automated"), "{msg}"),
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_answers_are_clamped_and_non_finite_ones_are_rejected() {
        let mut body = sample_body();
        body["answers"]["asks_reply"]["noul"] = json!(1.7);
        body["answers"]["person"]["noul"] = json!(-0.2);
        body["answers"]["urgency"]["score"] = json!(5.0);
        let j = parse_response(&body).unwrap();
        assert_eq!((j.asks_reply, j.person, j.urgency), (1.0, 0.0, 3.0));

        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            match finite("bulk", bad) {
                Err(LlmError::Provider(msg)) => assert!(msg.contains("bulk"), "{msg}"),
                other => panic!("expected Provider error, got {other:?}"),
            }
        }
    }

    /// Serves one canned HTTP response and returns the request text it received.
    fn serve_once(status_line: &'static str, body: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read the whole request: headers, then Content-Length body bytes.
            let mut data = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..end]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0, "client closed before sending a full request");
                data.extend_from_slice(&chunk[..n]);
            }
            let request = String::from_utf8_lossy(&data).to_string();
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            // Let the client close first; closing under it makes ureq panic while pooling the socket.
            let _ = stream.read(&mut chunk);
            request
        });
        (url, handle)
    }

    #[test]
    fn score_posts_the_bearer_key_and_parses_the_answers() {
        let (url, handle) = serve_once("200 OK", sample_body().to_string());
        let client = TypeSafeClient::with_base_url(url, "secret-key");
        let j = client.score(&input()).unwrap();
        assert_eq!(j.asks_reply, 0.99);
        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /v1/systemone"), "{request}");
        assert!(request.to_lowercase().contains("authorization: bearer secret-key"), "{request}");
    }

    #[test]
    fn a_rejected_key_surfaces_as_an_http_401() {
        let (url, handle) = serve_once("401 Unauthorized", "{\"error\":\"bad key\"}".to_string());
        let client = TypeSafeClient::with_base_url(url, "wrong");
        assert!(matches!(client.score(&input()), Err(LlmError::Http { status: 401, .. })));
        handle.join().unwrap();
    }

    fn title_input() -> TitleMatchInput {
        TitleMatchInput {
            event_title: "CMS Daily Win Room".into(),
            attendees: vec!["John Jennings".into()],
            tldr: "Partner costs.".into(),
            decisions: vec!["Cut GitHub spend".into()],
        }
    }

    #[test]
    fn the_title_match_request_carries_the_event_summary_and_one_noul_question() {
        let req = build_title_match_request(&title_input());
        assert_eq!(req["model"], "jev-latest");
        assert_eq!(req["state"]["calendar_event"]["title"], "CMS Daily Win Room");
        assert_eq!(req["state"]["calendar_event"]["attendees"][0], "John Jennings");
        assert_eq!(req["state"]["meeting_summary"]["tldr"], "Partner costs.");
        assert_eq!(req["state"]["meeting_summary"]["decisions"][0], "Cut GitHub spend");
        let q = &req["questions"]["summary_is_this_event"];
        assert_eq!(q["type"], "noul");
        assert!(q["instructions"].as_str().unwrap().contains("`calendar_event`"));
        assert!(q["criteria"]["true"].is_string() && q["criteria"]["false"].is_string());
    }

    #[test]
    fn the_title_match_answer_parses_clamps_and_rejects_missing_or_non_finite() {
        let body = |v: Value| json!({"answers": {"summary_is_this_event": {"type": "noul", "noul": v}}});
        assert_eq!(parse_title_match(&body(json!(0.07))).unwrap(), 0.07);
        assert_eq!(parse_title_match(&body(json!(1.7))).unwrap(), 1.0);
        assert_eq!(parse_title_match(&body(json!(-0.2))).unwrap(), 0.0);
        assert!(matches!(parse_title_match(&json!({"answers": {}})), Err(LlmError::Provider(_))));
        // JSON cannot carry NaN, so the finite guard is exercised directly.
        assert!(matches!(finite("summary_is_this_event", f64::NAN), Err(LlmError::Provider(_))));
    }

    #[test]
    fn the_title_match_gives_up_sooner_than_scoring() {
        assert!(TITLE_MATCH_READ_TIMEOUT < TYPESAFE_READ_TIMEOUT);
        assert_eq!(TITLE_MATCH_READ_TIMEOUT.as_secs(), 15);
    }

    #[test]
    fn title_match_posts_to_the_same_endpoint_and_returns_the_probability() {
        let body = json!({"answers": {"summary_is_this_event": {"type": "noul", "noul": 0.05}}});
        let (url, handle) = serve_once("200 OK", body.to_string());
        let client = TypeSafeClient::with_base_url(url, "secret-key");
        assert_eq!(client.probability(&title_input()).unwrap(), 0.05);
        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /v1/systemone"), "{request}");
        assert!(request.to_lowercase().contains("authorization: bearer secret-key"), "{request}");
        assert!(request.contains("summary_is_this_event"), "{request}");
    }

    fn labels() -> Vec<(String, String)> {
        vec![("t1".into(), "Pricing work".into()), ("t2".into(), "Hiring".into())]
    }

    fn topic_input(ask_work: bool) -> TopicJudgeInput {
        TopicJudgeInput { kind: "email".into(), text: "Q3 pricing deck".into(), labels: labels(), ask_work }
    }

    fn topic_body() -> Value {
        json!({
            "model": "jev-1.13.0",
            "answers": {
                "topic": {"type": "choice", "choice": "t1", "confidence": 0.8,
                          "probabilities": {"t1": 0.8, "t2": 0.15, "none": 0.05}},
                "work": {"type": "noul", "noul": 0.9}
            },
            "usage": {"input_tokens": 300, "output_tokens": 40}
        })
    }

    #[test]
    fn topic_request_carries_choice_criteria_with_none_and_optional_work() {
        let req = build_topic_request(&topic_input(true));
        assert_eq!(req["model"], "jev-latest");
        assert_eq!(req["state"]["item"], "Q3 pricing deck");
        assert_eq!(req["state"]["kind"], "email");
        let t = &req["questions"]["topic"];
        assert_eq!(t["type"], "choice");
        assert_eq!(t["criteria"]["t1"], "Pricing work");
        assert_eq!(t["criteria"]["t2"], "Hiring");
        assert_eq!(t["criteria"]["none"], "About none of these topics.");
        assert_eq!(req["questions"]["work"]["type"], "noul");
        assert!(build_topic_request(&topic_input(false))["questions"].get("work").is_none());
    }

    #[test]
    fn topic_response_parses_probabilities_work_and_usage() {
        let j = parse_topic_response(&topic_body(), &labels(), true).unwrap();
        assert_eq!(
            j.probabilities,
            vec![("t1".to_string(), 0.8), ("t2".to_string(), 0.15), ("none".to_string(), 0.05)]
        );
        assert_eq!(j.work, Some(0.9));
        assert_eq!(j.model, "jev-1.13.0");
        assert_eq!(j.usage, Some(TokenUsage { input_tokens: Some(300), output_tokens: Some(40) }));
        let no_work = parse_topic_response(&topic_body(), &labels(), false).unwrap();
        assert_eq!(no_work.work, None);
    }

    #[test]
    fn missing_label_probability_is_a_provider_error() {
        let mut body = topic_body();
        body["answers"]["topic"]["probabilities"].as_object_mut().unwrap().remove("t2");
        match parse_topic_response(&body, &labels(), false) {
            Err(LlmError::Provider(msg)) => assert!(msg.contains("t2"), "{msg}"),
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[test]
    fn topic_judge_401_surfaces_as_http_401() {
        let (url, handle) = serve_once("401 Unauthorized", "{\"error\":\"bad key\"}".to_string());
        let client = TypeSafeClient::with_base_url(url, "wrong");
        assert!(matches!(client.judge(&topic_input(false)), Err(LlmError::Http { status: 401, .. })));
        handle.join().unwrap();
    }

    fn step_input() -> StepJudgeInput {
        StepJudgeInput {
            kind: "email".into(),
            me: vec!["Mike".into()],
            from: Some("Ana".into()),
            context: "Pricing thread".into(),
            lines: vec!["Mike to send the deck".into(), "Offsite is on 3 Dec".into()],
            open_steps: vec!["Send the deck".into()],
            examples: vec![("Thanks all".into(), "pleasantry".into())],
        }
    }

    #[test]
    fn step_request_has_a_choice_per_line_and_a_noul_per_open_step() {
        let req = build_step_request(&step_input());
        let q = req["questions"].as_object().unwrap();
        let mut keys: Vec<_> = q.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["c1", "c2", "x1"]);
        assert_eq!(q["c1"]["type"], "choice");
        let mut crit: Vec<_> = q["c1"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        crit.sort();
        assert_eq!(crit, ["date", "mine", "none", "owed"]);
        assert_eq!(
            q["c1"]["criteria"]["none"],
            "None of these. For example: \"Thanks all\" (pleasantry)"
        );
        assert_eq!(q["x1"]["type"], "noul");
        let s = &req["state"];
        assert_eq!(
            (
                s["item"].as_str(),
                s["line_2"].as_str(),
                s["step_1"].as_str()
            ),
            (
                Some("Pricing thread"),
                Some("Offsite is on 3 Dec"),
                Some("Send the deck")
            )
        );
        assert_eq!(
            (s["kind"].as_str(), s["from"].as_str()),
            (Some("email"), Some("Ana"))
        );
    }

    #[test]
    fn step_response_parses_lines_closes_and_usage() {
        let body = json!({"model":"jev-1.13.0","answers":{"c1":{"type":"choice","probabilities":{"mine":0.8,"owed":0.1,"date":0.05,"none":0.05}},"x1":{"type":"noul","noul":0.9}},"usage":{"input_tokens":900,"output_tokens":60}});
        let j = parse_step_response(&body, 1, 1).unwrap();
        assert_eq!(
            j.lines,
            vec![vec![
                ("mine".to_string(), 0.8),
                ("owed".into(), 0.1),
                ("date".into(), 0.05),
                ("none".into(), 0.05)
            ]]
        );
        assert_eq!(j.closes, vec![0.9]);
        assert_eq!(j.model, "jev-1.13.0");
        assert_eq!(
            j.usage,
            Some(TokenUsage {
                input_tokens: Some(900),
                output_tokens: Some(60)
            })
        );
    }

    #[test]
    fn missing_line_answer_is_a_provider_error() {
        let body = json!({"answers":{"x1":{"type":"noul","noul":0.9}}});
        match parse_step_response(&body, 1, 1) {
            Err(LlmError::Provider(msg)) => assert!(msg.contains("c1"), "{msg}"),
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[test]
    fn step_judge_401_surfaces_as_http_401() {
        let (url, handle) = serve_once("401 Unauthorized", "{\"error\":\"bad key\"}".to_string());
        let client = TypeSafeClient::with_base_url(url, "wrong");
        assert!(matches!(
            client.judge_steps(&step_input()),
            Err(LlmError::Http { status: 401, .. })
        ));
        handle.join().unwrap();
    }

    /// Live call; run with `TYPESAFE_API_KEY=... cargo test -p llm live_ -- --ignored`.
    #[test]
    #[ignore]
    fn live_typesafe_scores_an_obvious_ask() {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let j = TypeSafeClient::new(key).score(&input()).unwrap();
        assert!(j.asks_reply > 0.5, "{j:?}");
    }
}
