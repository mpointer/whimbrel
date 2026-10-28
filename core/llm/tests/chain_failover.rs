use std::cell::RefCell;

use llm::{try_chain, ChainLink, Completion, LlmClient, LlmError, Provider, TokenUsage};

/// Returns queued results in order (repeating the last once exhausted).
struct FakeClient {
    responses: RefCell<Vec<Result<&'static str, LlmError>>>,
}

impl FakeClient {
    fn new(responses: Vec<Result<&'static str, LlmError>>) -> Self {
        Self {
            responses: RefCell::new(responses),
        }
    }
}

impl LlmClient for FakeClient {
    fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
        let mut responses = self.responses.borrow_mut();
        let next = if responses.len() > 1 {
            responses.remove(0)
        } else {
            responses[0].clone_result()
        };
        next.map(|text| Completion {
            text: text.to_string(),
            token_usage: TokenUsage::default(),
        })
    }
}

// LlmError isn't Clone (by design — it wraps provider response bodies that don't need
// to be duplicated); this test-only helper clones just enough to let FakeClient repeat
// its last queued response indefinitely without consuming the Vec.
trait CloneResult {
    fn clone_result(&self) -> Result<&'static str, LlmError>;
}
impl CloneResult for Result<&'static str, LlmError> {
    fn clone_result(&self) -> Result<&'static str, LlmError> {
        match self {
            Ok(s) => Ok(s),
            Err(LlmError::Http { status, body }) => Err(LlmError::Http {
                status: *status,
                body: body.clone(),
            }),
            Err(LlmError::Network(s)) => Err(LlmError::Network(s.clone())),
            Err(LlmError::Provider(s)) => Err(LlmError::Provider(s.clone())),
            Err(LlmError::NotImplemented(s)) => Err(LlmError::NotImplemented(s)),
        }
    }
}

fn retryable_err() -> LlmError {
    LlmError::Http {
        status: 503,
        body: "server busy".to_string(),
    }
}

fn non_retryable_err() -> LlmError {
    LlmError::Http {
        status: 400,
        body: "bad request".to_string(),
    }
}

#[test]
fn falls_to_the_next_link_on_a_retryable_error() {
    let first = FakeClient::new(vec![Err(retryable_err())]);
    let second = FakeClient::new(vec![Ok("from link 2")]);
    let chain = vec![
        ChainLink {
            provider: Provider::Anthropic,
            model: "test-model".to_string(),
            client: Box::new(first),
        },
        ChainLink {
            provider: Provider::OpenAI,
            model: "test-model".to_string(),
            client: Box::new(second),
        },
    ];
    let (completion, link) = try_chain(&chain, |link| link.client.complete("hi")).unwrap();
    assert_eq!(completion.text, "from link 2");
    assert_eq!(link.provider, Provider::OpenAI);
}

#[test]
fn a_non_retryable_error_stops_the_chain_immediately() {
    let first = FakeClient::new(vec![Err(non_retryable_err())]);
    // If the chain incorrectly fell through, this would be called and we'd see its
    // text instead of None — proving the chain actually stopped requires that this
    // link is never reached, which the None assertion below establishes.
    let second = FakeClient::new(vec![Ok("should never be reached")]);
    let chain = vec![
        ChainLink {
            provider: Provider::Anthropic,
            model: "test-model".to_string(),
            client: Box::new(first),
        },
        ChainLink {
            provider: Provider::OpenAI,
            model: "test-model".to_string(),
            client: Box::new(second),
        },
    ];
    assert!(try_chain(&chain, |link| link.client.complete("hi")).is_none());
}

#[test]
fn returns_none_when_every_link_is_exhausted() {
    let first = FakeClient::new(vec![Err(retryable_err())]);
    let second = FakeClient::new(vec![Err(retryable_err())]);
    let chain = vec![
        ChainLink {
            provider: Provider::Anthropic,
            model: "test-model".to_string(),
            client: Box::new(first),
        },
        ChainLink {
            provider: Provider::OpenAI,
            model: "test-model".to_string(),
            client: Box::new(second),
        },
    ];
    assert!(try_chain(&chain, |link| link.client.complete("hi")).is_none());
}

#[test]
fn a_single_successful_link_returns_immediately_without_trying_further_links() {
    let only = FakeClient::new(vec![Ok("the only answer")]);
    let chain = vec![ChainLink {
        provider: Provider::Ollama,
        model: "test-model".to_string(),
        client: Box::new(only),
    }];
    let (completion, link) = try_chain(&chain, |link| link.client.complete("hi")).unwrap();
    assert_eq!(completion.text, "the only answer");
    assert_eq!(link.provider, Provider::Ollama);
}
