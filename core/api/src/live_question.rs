//! Governed live-question calls: `detect` (one cheap judgment per candidate line) and
//! `draft` (a memory-grounded reply via `ask::answer`).

use llm::live_question::{
    build_detect_prompt, context_window, following, parse_verdict, should_alert, LiveLine,
    LiveQuestionVerdict,
};

use crate::envelope::ApiResult;
use crate::routing::{build_chain_for, RoutingConfig};
use crate::util::now_unix_ms;

/// One model call; `None` unless the reply parses and clears `should_alert`.
fn judge(
    client: &dyn llm::LlmClient,
    prompt: &str,
) -> Result<(Option<LiveQuestionVerdict>, llm::TokenUsage), llm::LlmError> {
    let c = client.complete(prompt)?;
    Ok((parse_verdict(&c.text).filter(should_alert), c.token_usage))
}

pub fn detect(
    store: &memory::Store,
    lines: &[LiveLine],
    index: usize,
    names: &[String],
    ollama_model: &str,
    routing: &RoutingConfig,
) -> ApiResult<Option<LiveQuestionVerdict>> {
    let Some(candidate) = lines.get(index) else {
        return Ok(None);
    };
    llm::check_spend_cap(store, &routing.spend_cap_config(), None, Some("live_question"), now_unix_ms())?;

    let prompt = build_detect_prompt(names, context_window(lines, index), candidate, following(lines, index));
    let chain = crate::budget::apply_budgets(store, build_chain_for(llm::TaskKind::Chat, ollama_model, routing), false, now_unix_ms())?;
    let ((verdict, usage), entry) =
        llm::try_chain_with_failures(&chain, |e| judge(e.client.as_ref(), &prompt)).map_err(crate::chain_failed)?;
    crate::util::log_link_usage(store, entry, "live_question", usage);
    Ok(verdict)
}

pub fn draft(
    store: &memory::Store,
    verdict: &LiveQuestionVerdict,
    recent: &[LiveLine],
    ollama_model: &str,
    routing: &RoutingConfig,
) -> ApiResult<llm::ChatAnswer> {
    let newest = recent.iter().map(|l| l.start_cs).max().unwrap_or(0);
    let background: String = recent
        .iter()
        .filter(|l| l.start_cs >= newest - 5 * 60 * 100)
        .map(|l| format!("[{}] {}\n", l.channel, l.text))
        .collect();
    let history = [llm::ChatTurn {
        role: llm::ChatRole::User,
        text: format!("Background, the last minutes of the meeting:\n{background}"),
    }];
    let query = format!(
        "Draft my reply to this, asked of me in a meeting: {}\n\n\
         Answer briefly, as me. Cite every claim. If my memory does not contain the answer, say so in one line.",
        verdict.quote
    );
    crate::ask::answer(store, &query, ollama_model, 10, &history, routing)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock(&'static str);
    impl llm::LlmClient for Mock {
        fn complete(&self, _p: &str) -> Result<llm::Completion, llm::LlmError> {
            Ok(llm::Completion { text: self.0.to_string(), token_usage: llm::TokenUsage::default() })
        }
    }

    fn lines() -> Vec<LiveLine> {
        vec![LiveLine { id: 1, channel: "them".into(), start_cs: 0, text: "Mike, can you ship it Friday?".into() }]
    }

    #[test]
    fn detect_returns_none_below_high_confidence() {
        let medium = r#"{"is_for_user":true,"confidence":"medium","quote":"q","asker":null,"restatement":"r"}"#;
        let (v, _) = judge(&Mock(medium), "p").unwrap();
        assert!(v.is_none());
        let high = r#"{"is_for_user":true,"confidence":"high","quote":"q","asker":null,"restatement":"r"}"#;
        assert!(judge(&Mock(high), "p").unwrap().0.is_some());
        assert!(judge(&Mock("not json"), "p").unwrap().0.is_none());
    }

    #[test]
    fn detect_respects_spend_cap() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 10_000,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: crate::util::now_unix_ms(),
            })
            .unwrap();
        let routing = RoutingConfig { global_daily_cap_cents: Some(1), ..RoutingConfig::default() };
        let r = detect(&store, &lines(), 0, &["Mike".to_string()], "gemma4:12b", &routing);
        assert!(matches!(r, Err(crate::ApiError::SpendCap(_))));
    }
}
