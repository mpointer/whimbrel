//! TypeSafe client for Reply Priorities: one request carries the five questions
//! (docs/superpowers/specs/2026-10-05-reply-priorities-design.md).

use memory::priority::PriorityJudgments;
use serde_json::{json, Value};

use super::{ureq_error_to_llm, LlmError};

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

    /// Live call; run with `TYPESAFE_API_KEY=... cargo test -p llm live_ -- --ignored`.
    #[test]
    #[ignore]
    fn live_typesafe_scores_an_obvious_ask() {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let j = TypeSafeClient::new(key).score(&input()).unwrap();
        assert!(j.asks_reply > 0.5, "{j:?}");
    }
}
