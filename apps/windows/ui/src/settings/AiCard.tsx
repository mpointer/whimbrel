import { useEffect, useState } from "react";
import { aiSettings, providerModels, setApiKey, setPrompt, setRoutingSettings, setSummaryInstructions } from "../api";
import { describeError, isCommandError } from "../errors";
import type { AiSettings, PromptSurface, Provider, RoutingSettings } from "../generated/types";
import {
  PROVIDERS,
  SURFACES,
  capToDollars,
  formatCents,
  hostedNotice,
  keyLine,
  modelField,
  modelOptions,
  parseCapDollars,
  providerLabel,
} from "./aiModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function AiCard() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    aiSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  if (!settings) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  const saveRouting = (routing: RoutingSettings) => {
    setSettings({ ...settings, routing });
    setRoutingSettings(routing)
      .then(() => setError(null))
      .catch((e: unknown) => setError(message(e)));
  };

  return (
    <section className="card about" aria-labelledby="ai-heading">
      <h2 id="ai-heading" className="section-label">
        AI &amp; routing
      </h2>
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
      <h3 className="section-label">Instructions</h3>
      <Instructions settings={settings} onError={setError} />
      {settings.spend && (
        <p className="muted small">
          Spent today: {formatCents(settings.spend.globalCents)} across hosted providers (resets at midnight UTC).
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
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

function KeyRow(props: { provider: Provider; line: string; onSave: (key: string | null) => Promise<void> }) {
  const [draft, setDraft] = useState("");
  const saved = props.line !== "Not set";
  return (
    <form
      className="field-row key-row"
      onSubmit={(e) => {
        e.preventDefault();
        if (draft.trim()) props.onSave(draft).then(() => setDraft(""));
      }}
    >
      <span className="key-name">{providerLabel(props.provider)}</span>
      <span className="muted small mono key-line">{props.line}</span>
      <input
        className="input grow"
        type="password"
        autoComplete="off"
        placeholder={saved ? "Replace key" : "Paste key"}
        aria-label={`${providerLabel(props.provider)} API key`}
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
