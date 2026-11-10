//! Monthly AI budgets (docs/superpowers/specs/2026-11-05-provider-budgets-design.md): every
//! governed call skips a provider whose month spend has reached its budget, and fails with
//! [`BudgetSpent`] when no provider is left. Free providers (`ollama`, `local`) are never
//! blocked; `memory::Store::provider_over_budget` decides that.

/// A provider's budget for this UTC month is spent. Reaches the shells as the envelope
/// `{"ok":false,"error":{"kind":"budget_spent","provider":…,"spent_cents":…,"budget_cents":…}}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetSpent {
    pub provider: llm::Provider,
    pub spent_cents: i64,
    pub budget_cents: i64,
}

/// Replaces every budget (`provider -> cents`; 0 or a free provider sets none). The shells'
/// Settings call this; Windows calls it directly, the Mac through `whimbrel_set_provider_budgets`.
pub fn set_budgets(store: &memory::Store, budgets: &std::collections::BTreeMap<String, i64>) -> rusqlite::Result<()> {
    store.set_provider_budgets(&budgets.iter().map(|(p, c)| (p.clone(), *c)).collect::<Vec<_>>())
}

/// Every budget set, `provider -> cents`.
pub fn budgets(store: &memory::Store) -> rusqlite::Result<std::collections::BTreeMap<String, i64>> {
    Ok(store.provider_budgets()?.into_iter().collect())
}

/// Budget thresholds newly reached this device hasn't told the user about; each is returned once.
pub fn take_notices(store: &memory::Store, now_unix_ms: i64) -> rusqlite::Result<Vec<memory::provider_budget::BudgetNotice>> {
    store.take_budget_notices(now_unix_ms)
}

/// `Err` when `provider`'s budget is spent. For a call pinned to one provider (streaming Ask,
/// skins). A store read error counts as not over, as `llm::check_spend_cap` treats one.
pub fn check_provider(store: &memory::Store, provider: llm::Provider, now_unix_ms: i64) -> Result<(), BudgetSpent> {
    // Whimbrel AI has no local budget: the server's prepaid credit is its limit.
    if provider == llm::Provider::Whimbrel {
        return Ok(());
    }
    match store.provider_over_budget(provider.as_str(), now_unix_ms) {
        Ok(Some((spent_cents, budget_cents))) => Err(BudgetSpent { provider, spent_cents, budget_cents }),
        _ => Ok(()),
    }
}

/// `chain` without its over-budget links. `Err` for the first dropped provider when nothing
/// is left, or when `pinned` (the user picked the first link's provider) and the first link
/// is over: a pinned call never falls through to another provider for budget.
pub fn apply_budgets(
    store: &memory::Store,
    chain: Vec<llm::ChainLink>,
    pinned: bool,
    now_unix_ms: i64,
) -> Result<Vec<llm::ChainLink>, BudgetSpent> {
    let mut first_dropped = None;
    let mut kept = Vec::with_capacity(chain.len());
    for (i, link) in chain.into_iter().enumerate() {
        match check_provider(store, link.provider, now_unix_ms) {
            Ok(()) => kept.push(link),
            Err(spent) if pinned && i == 0 => return Err(spent),
            Err(spent) => {
                first_dropped.get_or_insert(spent);
            }
        }
    }
    match first_dropped {
        Some(spent) if kept.is_empty() => Err(spent),
        _ => Ok(kept),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::Provider;

    // 2026-11-15T12:00:00Z
    const NOV: i64 = 1_794_744_000_000;

    struct Fake;
    impl llm::LlmClient for Fake {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Ok(llm::Completion { text: "ok".into(), token_usage: llm::TokenUsage::default() })
        }
    }

    fn chain(providers: &[Provider]) -> Vec<llm::ChainLink> {
        providers.iter().map(|&provider| llm::ChainLink { provider, model: "m".into(), client: Box::new(Fake) }).collect()
    }

    fn providers(chain: &[llm::ChainLink]) -> Vec<Provider> {
        chain.iter().map(|l| l.provider).collect()
    }

    fn spend(store: &memory::Store, provider: &str, cents: i64) {
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: provider.into(),
                model: "m".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: cents,
                cache_hit: false,
                trace_id: format!("{provider}-{cents}"),
                ts_unix_ms: NOV,
            })
            .unwrap();
    }

    fn budgets(store: &memory::Store, budgets: &[(&str, i64)]) {
        let budgets: Vec<(String, i64)> = budgets.iter().map(|(p, c)| (p.to_string(), *c)).collect();
        store.set_provider_budgets(&budgets).unwrap();
    }

    #[test]
    fn over_budget_link_skipped_next_used() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "anthropic", 2500);
        budgets(&store, &[("anthropic", 2500), ("openai", 2500)]);
        let kept = apply_budgets(&store, chain(&[Provider::Anthropic, Provider::OpenAI]), false, NOV).unwrap();
        assert_eq!(providers(&kept), [Provider::OpenAI]);
        let (_, link) = llm::try_chain(&kept, |l| l.client.complete("q")).unwrap();
        assert_eq!(link.provider, Provider::OpenAI);
    }

    #[test]
    fn all_over_budget_errors() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "anthropic", 3000);
        spend(&store, "openai", 100);
        budgets(&store, &[("anthropic", 2500), ("openai", 100)]);
        let err = apply_budgets(&store, chain(&[Provider::Anthropic, Provider::OpenAI]), false, NOV).err().unwrap();
        assert_eq!(err, BudgetSpent { provider: Provider::Anthropic, spent_cents: 3000, budget_cents: 2500 });
    }

    #[test]
    fn no_budget_unchanged() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "anthropic", 99_999);
        let all = [Provider::Anthropic, Provider::OpenAI, Provider::Google, Provider::OpenRouter];
        assert_eq!(providers(&apply_budgets(&store, chain(&all), true, NOV).unwrap()), all);
        // An empty chain (no keys, no local model) stays a provider failure, not BudgetSpent.
        assert!(apply_budgets(&store, Vec::new(), false, NOV).unwrap().is_empty());
    }

    #[test]
    fn free_provider_never_blocked() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "anthropic", 2500);
        spend(&store, "ollama", 9_999);
        budgets(&store, &[("anthropic", 2500), ("ollama", 1), ("local", 1)]);
        let kept = apply_budgets(&store, chain(&[Provider::Anthropic, Provider::Ollama, Provider::Local]), false, NOV).unwrap();
        assert_eq!(providers(&kept), [Provider::Ollama, Provider::Local]);
        assert!(check_provider(&store, Provider::Ollama, NOV).is_ok());
    }

    #[test]
    fn pinned_over_budget_errors_without_fallthrough() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "openai", 500);
        budgets(&store, &[("openai", 500)]);
        let err = apply_budgets(&store, chain(&[Provider::OpenAI, Provider::Ollama]), true, NOV).err().unwrap();
        assert_eq!(err.provider, Provider::OpenAI);
        assert_eq!(check_provider(&store, Provider::OpenAI, NOV), Err(err));
        // Pinned to a provider with budget left: an over-budget fallback behind it is still dropped.
        let kept = apply_budgets(&store, chain(&[Provider::Anthropic, Provider::OpenAI]), true, NOV).unwrap();
        assert_eq!(providers(&kept), [Provider::Anthropic]);
    }

    #[test]
    fn raising_budget_unblocks_and_rearms() {
        let store = memory::Store::open_in_memory().unwrap();
        spend(&store, "openai", 2500);
        budgets(&store, &[("openai", 2500)]);
        assert!(apply_budgets(&store, chain(&[Provider::OpenAI]), false, NOV).is_err());
        let thresholds = |s: &memory::Store| s.take_budget_notices(NOV).unwrap().iter().map(|n| n.threshold).collect::<Vec<_>>();
        assert_eq!(thresholds(&store), [80, 100]);

        budgets(&store, &[("openai", 5000)]);
        assert_eq!(providers(&apply_budgets(&store, chain(&[Provider::OpenAI]), false, NOV).unwrap()), [Provider::OpenAI]);
        assert!(thresholds(&store).is_empty(), "50% of the new budget notices nothing");
        spend(&store, "openai", 1500);
        assert_eq!(thresholds(&store), [80], "80% of the new budget notices again");
        spend(&store, "openai", 1000);
        assert_eq!(check_provider(&store, Provider::OpenAI, NOV).unwrap_err().budget_cents, 5000);
        assert_eq!(thresholds(&store), [100]);
    }
}
