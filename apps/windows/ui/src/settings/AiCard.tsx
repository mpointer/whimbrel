import { useEffect, useState } from "react";
import {
  aiSettings,
  keyCheck,
  keyFormatHint,
  openExternal,
  providerModels,
  setApiKey,
  setImageSearchKey,
  setPrompt,
  setRoutingSettings,
  setSummaryInstructions,
  usageRows,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { AiSettings, PromptSurface, Provider, ProviderInfoView, RoutingSettings, UsageRowView, UsageView } from "../generated/types";
import {
  PROVIDERS,
  SURFACES,
  capToDollars,
  costLine,
  dataTermsLine,
  formatCents,
  groupUsageByDay,
  hostedNotice,
  keyCheckLine,
  keyLine,
  modelField,
  modelOptions,
  parseCapDollars,
  providerLabel,
  showsOnRamp,
  usageCost,
  usageTime,
  usageTokens,
  usageTotalsLine,
} from "./aiModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function AiCard() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ollamaUp, setOllamaUp] = useState(false);

  useEffect(() => {
    aiSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
    // Whether the local model answers decides if the on-ramp shows; a failed probe is "no".
    providerModels("ollama").then(
      (list) => setOllamaUp(list.length > 0),
      () => setOllamaUp(false),
    );
  }, []);

  if (!settings) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  const saveRouting = (routing: RoutingSettings) => {
    setSettings({ ...settings, routing });
    setRoutingSettings(routing)
      .then(() => setError(null))
      .catch((e: unknown) => setError(message(e)));
  };

  const info = (provider: Provider) => settings.providers.find((p) => p.id === provider);

  return (
    <section className="card about" aria-labelledby="ai-heading">
      <h2 id="ai-heading" className="section-label">
        AI &amp; routing
      </h2>
      {showsOnRamp(settings.routing, settings.keys, ollamaUp) && <OnRamp google={info("google")} />}
      <Routing
        routing={settings.routing}
        onChange={saveRouting}
        notice={hostedNotice(settings.routing, settings.keys)}
        onError={setError}
      />
      <h3 className="section-label">API keys</h3>
      <p className="muted small">Stored in Windows Credential Manager under the same names the Mac's Keychain uses.</p>
      {settings.keys.map((k) => (
        <KeyRow
          key={k.provider}
          provider={k.provider}
          info={info(k.provider)}
          usage={settings.usage}
          line={keyLine(k)}
          onSave={(key) =>
            setApiKey(k.provider, key)
              .then((keys) => {
                setSettings({ ...settings, keys });
                setError(null);
              })
              .catch((e: unknown) => setError(message(e)))
          }
        />
      ))}
      <ImageSearchKeyRow
        line={keyLine({ masked: settings.imageSearchKey })}
        onSave={(key) =>
          setImageSearchKey(key)
            .then((imageSearchKey) => {
              setSettings({ ...settings, imageSearchKey });
              setError(null);
            })
            .catch((e: unknown) => setError(message(e)))
        }
      />
      <h3 className="section-label">Instructions</h3>
      <Instructions settings={settings} onError={setError} />
      {settings.spend && (
        <p className="muted small">
          Spent today: {formatCents(settings.spend.globalCents)} across hosted providers (resets at midnight UTC).
        </p>
      )}
      <h3 className="section-label">Model usage</h3>
      <Usage usage={settings.usage} onError={setError} />
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

/** Shown when nothing can answer: no hosted key and no local model reachable. */
function OnRamp(props: { google: ProviderInfoView | undefined }) {
  const openUrl = (url: string | null | undefined) => url && void openExternal(url);
  return (
    <div className="stack-sm" data-testid="ai-on-ramp">
      <p>
        <strong>Pick where AI runs.</strong> Whimbrel works without any AI model: capture, transcripts and search still work. Chat,
        summaries and routines need one.
      </p>
      <span className="field-row">
        <button type="button" className="mode" onClick={() => openUrl("https://ollama.com/download")}>
          Install Ollama
        </button>
        <button type="button" className="mode" onClick={() => openUrl(props.google?.keyPageUrl)}>
          Get a free Gemini key
        </button>
      </span>
      <p className="muted small">
        Ollama is free, runs on this PC and nothing leaves it. Gemini's free tier is free, and Google may use prompts to improve its
        products. Or paste any Anthropic, OpenAI, Google or OpenRouter key below.
      </p>
    </div>
  );
}

function Routing(props: {
  routing: RoutingSettings;
  notice: string | null;
  onChange: (r: RoutingSettings) => void;
  onError: (e: string | null) => void;
}) {
  const { routing, onChange, notice, onError } = props;
  const provider = routing.preferredProvider;
  const [model, setModel] = useState(routing[modelField(provider)]);
  const [models, setModels] = useState<Partial<Record<Provider, string[]>>>({});
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const list = models[provider];
  const [caps, setCaps] = useState({
    user: capToDollars(routing.userDailyCapCents),
    global: capToDollars(routing.globalDailyCapCents),
  });

  useEffect(() => setModel(routing[modelField(provider)]), [provider, routing]);
  useEffect(() => setListError(null), [provider]);

  // The result and any error show beside the button, not at the foot of the card.
  const loadModels = () => {
    setLoading(true);
    setListError(null);
    providerModels(provider)
      .then((loaded) => setModels((m) => ({ ...m, [provider]: loaded })))
      .catch((e: unknown) => setListError(message(e)))
      .finally(() => setLoading(false));
  };

  const saveModel = (value: string) => {
    const v = value.trim();
    if (v && v !== routing[modelField(provider)]) onChange({ ...routing, [modelField(provider)]: v });
  };

  const saveCap = (which: "user" | "global") => {
    const cents = parseCapDollars(caps[which]);
    if (cents === undefined) return onError("A daily cap is a dollar amount, like 2.50. Leave it blank for the default; 0 turns it off.");
    onError(null);
    onChange({ ...routing, [which === "user" ? "userDailyCapCents" : "globalDailyCapCents"]: cents });
  };

  return (
    <div className="stack-sm">
      <label className="field">
        <span>Preferred provider</span>
        <select
          className="input"
          value={provider}
          onChange={(e) => onChange({ ...routing, preferredProvider: e.target.value as Provider })}
        >
          {PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      <div className="field">
        <label htmlFor="model-input">{providerLabel(provider)} model</label>
        <span className="field-row">
          {list ? (
            <select
              id="model-input"
              className="input grow"
              value={model}
              onChange={(e) => {
                setModel(e.target.value);
                saveModel(e.target.value);
              }}
            >
              {provider === "openrouter" && <option value="">Not picked</option>}
              {modelOptions(list, model).map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : (
            <input
              id="model-input"
              className="input grow"
              placeholder={provider === "openrouter" ? "vendor/model or vendor/model:free" : undefined}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              onBlur={(e) => saveModel(e.target.value)}
            />
          )}
          <button type="button" className="mode" disabled={loading} onClick={loadModels}>
            {loading ? "Loading…" : list ? "Reload" : "Load models"}
          </button>
        </span>
        {list && (
          <span className="muted small">
            {list.length} models from {providerLabel(provider)}.
          </span>
        )}
        {provider === "openrouter" && (
          <span className="muted small">
            One key for any model on OpenRouter's catalogue, free ones included. Until you pick one, OpenRouter is left out of every
            chain, because their catalogue changes weekly and there is no fallback model to substitute.
          </span>
        )}
        {listError && (
          <span role="alert" className="small">
            {listError}
          </span>
        )}
      </div>
      <label className="toggle">
        <input type="checkbox" checked={routing.localOnly} onChange={(e) => onChange({ ...routing, localOnly: e.target.checked })} />
        Local-only mode (send nothing to hosted providers)
      </label>
      <label className="toggle">
        <input type="checkbox" checked={routing.useLocalModel} onChange={(e) => onChange({ ...routing, useLocalModel: e.target.checked })} />
        Use local model (Ollama)
      </label>
      <p className="muted small">
        {routing.localOnly
          ? "Local-only mode is on, so the local model is always used."
          : "Off skips Ollama entirely. Turn it off on a PC that does not run Ollama, so calls stop trying to reach it first."}
      </p>
      <label className="toggle">
        <input type="checkbox" checked={routing.cheapFirst} onChange={(e) => onChange({ ...routing, cheapFirst: e.target.checked })} />
        Use the cheapest model for small tasks
      </label>
      <p className="muted small">
        Conversation titles, topic names and profile suggestions go to the local model first, then to the cheapest model you have a key
        for. Chat, summaries and digests keep using your preferred provider. Each routine picks its own model in Automations.
      </p>
      {notice && <p className="muted small">{notice}</p>}
      <div className="field-row">
        {(["user", "global"] as const).map((which) => (
          <label key={which} className="field">
            <span>{which === "user" ? "Your daily cap ($)" : "App-wide daily cap ($)"}</span>
            <input
              className="input"
              inputMode="decimal"
              placeholder="default"
              value={caps[which]}
              onChange={(e) => setCaps({ ...caps, [which]: e.target.value })}
              onBlur={() => saveCap(which)}
            />
          </label>
        ))}
      </div>
      <p className="muted small">Blank uses the built-in cap; 0 turns the cap off.</p>
    </div>
  );
}

function KeyRow(props: {
  provider: Provider;
  info: ProviderInfoView | undefined;
  usage: UsageView | null | undefined;
  line: string;
  onSave: (key: string | null) => Promise<void>;
}) {
  const { info, provider } = props;
  const [draft, setDraft] = useState("");
  const [hint, setHint] = useState<string | null>(null);
  const [check, setCheck] = useState<{ text: string; isError: boolean } | null>(null);
  const saved = props.line !== "Not set";

  // The format hint is advisory: prefixes change, so the key saves either way.
  const updateDraft = (value: string) => {
    setDraft(value);
    if (!value.trim()) return setHint(null);
    keyFormatHint(provider, value).then(setHint, () => setHint(null));
  };

  const save = () => {
    if (!draft.trim()) return;
    setCheck({ text: "Saved. Checking the key…", isError: false });
    props
      .onSave(draft)
      .then(() => {
        setDraft("");
        setHint(null);
        return keyCheck(provider);
      })
      .then(
        (result) => setCheck(keyCheckLine(result)),
        (e: unknown) => setCheck({ text: message(e), isError: true }),
      );
  };

  const openUrl = (url: string | null | undefined) => url && void openExternal(url);

  return (
    <div className="stack-sm key-block">
      <form
        className="field-row key-row"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <span className="key-name">{providerLabel(provider)}</span>
        <span className="muted small mono key-line">{props.line}</span>
        <input
          className="input grow"
          type="password"
          autoComplete="off"
          placeholder={saved ? "Replace key" : "Paste key"}
          aria-label={`${providerLabel(provider)} API key`}
          value={draft}
          onChange={(e) => updateDraft(e.target.value)}
        />
        <button type="submit" className="mode" disabled={!draft.trim()}>
          Save
        </button>
        {saved && (
          <button
            type="button"
            className="mode"
            onClick={() => {
              setCheck(null);
              void props.onSave(null);
            }}
          >
            Remove
          </button>
        )}
      </form>
      {info && (
        <span className="field-row">
          {info.keyPageUrl && (
            <button type="button" className="link small" onClick={() => openUrl(info.keyPageUrl)}>
              Get a {info.name} key
            </button>
          )}
          {info.policyUrl && (
            <button type="button" className="link small" onClick={() => openUrl(info.policyUrl)}>
              Data policy
            </button>
          )}
        </span>
      )}
      {hint && <p className="muted small">{hint}</p>}
      {check && (
        <p className="small" role={check.isError ? "alert" : undefined}>
          {check.text}
        </p>
      )}
      {info && <p className="muted small">{costLine(info, props.usage)}</p>}
      {info && dataTermsLine(info) && <p className="muted small">{dataTermsLine(info)}</p>}
    </div>
  );
}

/** What every call cost, from the ledger the spend caps already keep. */
function Usage(props: { usage: UsageView | null | undefined; onError: (e: string | null) => void }) {
  const [rows, setRows] = useState<UsageRowView[]>([]);
  const [days, setDays] = useState(7);

  useEffect(() => {
    usageRows(days).then(setRows, (e: unknown) => props.onError(message(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [days]);

  return (
    <div className="stack-sm">
      {props.usage && <p className="muted small">{usageTotalsLine(props.usage)}</p>}
      {rows.length === 0 ? (
        <p className="muted small">No model calls yet.</p>
      ) : (
        groupUsageByDay(rows).map((group) => (
          <div key={group.day} className="stack-sm">
            <p className="small">
              <strong>{group.day}</strong>
            </p>
            <ul className="usage-list">
              {group.rows.map((row) => (
                <li key={`${row.tsUnixMs}-${row.provider}-${row.model}-${row.task}`} className="usage-row muted small mono">
                  <span>{usageTime(row.tsUnixMs)}</span>
                  <span>{row.task}</span>
                  <span>
                    {row.provider} {row.model}
                  </span>
                  <span>{usageTokens(row)}</span>
                  <span>{usageCost(row)}</span>
                </li>
              ))}
            </ul>
          </div>
        ))
      )}
      <button type="button" className="mode" onClick={() => setDays(days === 7 ? 30 : 7)}>
        {days === 7 ? "Show last 30 days" : "Show last 7 days"}
      </button>
    </div>
  );
}

/** The skin maker's web image search key. Not a provider: no model list to check it against. */
function ImageSearchKeyRow(props: { line: string; onSave: (key: string | null) => Promise<void> }) {
  const [draft, setDraft] = useState("");
  const saved = props.line !== "Not set";
  return (
    <div className="stack-sm key-block">
      <form
        className="field-row key-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (draft.trim()) props.onSave(draft).then(() => setDraft(""));
        }}
      >
        <span className="key-name">Image Search Key (Brave Search)</span>
        <span className="muted small mono key-line">{props.line}</span>
        <input
          className="input grow"
          type="password"
          autoComplete="off"
          placeholder={saved ? "Replace key" : "Brave Search API key"}
          aria-label="Brave Search API key"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button type="submit" className="mode" disabled={!draft.trim()}>
          Save
        </button>
        {saved && (
          <button type="button" className="mode" onClick={() => props.onSave(null)}>
            Remove
          </button>
        )}
      </form>
      <p className="muted small">The skin maker uses it to find images on the web. It stays on this PC.</p>
    </div>
  );
}

function Instructions(props: { settings: AiSettings; onError: (e: string | null) => void }) {
  const [summary, setSummary] = useState(props.settings.summaryInstructions);
  const [prompts, setPrompts] = useState<Record<string, string>>(
    Object.fromEntries(props.settings.prompts.map((p) => [p.surface, p.text])),
  );
  const done = (p: Promise<void>) => p.then(() => props.onError(null)).catch((e: unknown) => props.onError(message(e)));

  return (
    <div className="stack-sm">
      <label className="field">
        <span>Every summary</span>
        <textarea
          className="input"
          rows={2}
          placeholder="e.g. Use bullet points. Call out decisions and owners."
          value={summary}
          onChange={(e) => setSummary(e.target.value)}
          onBlur={() => done(setSummaryInstructions(summary))}
        />
      </label>
      <details>
        <summary className="small">Per-feature instructions</summary>
        {SURFACES.map((s) => (
          <label key={s.id} className="field">
            <span>{s.label}</span>
            <textarea
              className="input"
              rows={2}
              value={prompts[s.id] ?? ""}
              onChange={(e) => setPrompts({ ...prompts, [s.id]: e.target.value })}
              onBlur={() => done(setPrompt(s.id as PromptSurface, prompts[s.id] ?? ""))}
            />
          </label>
        ))}
      </details>
    </div>
  );
}
