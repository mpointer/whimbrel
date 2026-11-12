//! The daily read of the hosted-model defaults file (`api::model_defaults`). Opening the
//! store already put the cached defaults in force; this fetches the file once soon after
//! launch and then every 24 hours, so a retired model is fixed without a release.

use std::time::Duration;

use crate::store_actor::StoreHandle;

/// Long enough after launch to stay out of the startup work.
const FIRST_FETCH_AFTER: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_millis(api::model_defaults::REFRESH_EVERY_MS as u64);

/// One refresh: the GET runs on this thread, never inside a store lane; only the write and
/// the reload go through `store`.
pub fn refresh(store: &StoreHandle, fetch: impl FnOnce() -> Result<String, String>) -> Result<api::model_defaults::Defaults, String> {
    let raw = fetch()?;
    store
        .call_blocking(move |s| {
            api::model_defaults::store_file(s, &raw, api::util::now_unix_ms()).map_err(|e| e.to_string())?;
            Ok(api::model_defaults::load(s))
        })
        .map_err(|e| e.to_string())?
}

/// Starts the daily loop on its own thread. `store` should be the background lane.
pub fn start(store: StoreHandle) {
    let spawned = std::thread::Builder::new().name("whimbrel-model-defaults".into()).spawn(move || {
        std::thread::sleep(FIRST_FETCH_AFTER);
        loop {
            match refresh(&store, || api::model_defaults::fetch(api::model_defaults::DEFAULTS_URL)) {
                Ok(d) => tracing::info!(
                    google = d.model(llm::Provider::Google),
                    anthropic = d.model(llm::Provider::Anthropic),
                    openai = d.model(llm::Provider::OpenAI),
                    "model defaults: read the defaults file"
                ),
                Err(e) => tracing::info!(error = %e, "model defaults: kept the cached defaults"),
            }
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the model defaults refresh");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_fetch_changes_nothing_and_a_good_file_is_stored() {
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        assert_eq!(refresh(&store, || Err("HTTP 404".into())).unwrap_err(), "HTTP 404");
        // The built-in table's own values, so the process-wide defaults other tests read don't move.
        let builtin = api::model_defaults::Defaults::builtin();
        let file = serde_json::json!({"version": 1, "providers": builtin.providers}).to_string();
        let d = refresh(&store, || Ok(file)).unwrap();
        assert_eq!(d.source, api::model_defaults::Source::File);
        assert_eq!(d.model(llm::Provider::Google), builtin.model(llm::Provider::Google));
        let stored = store.call_blocking(|s| api::model_defaults::current(s)).unwrap();
        assert_eq!(stored.source, api::model_defaults::Source::File);
    }
}
