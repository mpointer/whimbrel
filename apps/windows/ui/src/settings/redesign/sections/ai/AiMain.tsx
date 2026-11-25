import { useEffect, useState } from "react";
import {
  keyCheck,
  localModelRemove,
  openExternal,
  providerModels,
  setApiKey,
  setApiKeySync,
  setImageSearchKey,
  setProviderBudget,
  whimbrelAiBuy,
} from "../../../../api";
import { t, useL10n } from "../../../../l10n";
import type { AiSettings, Provider, ProviderInfoView, UsageView, WhimbrelAiView } from "../../../../generated/types";
import {
  WButton,
  WConfirmDialog,
  WNoticeBox,
  WSettingsCard,
  WSettingsControlRow,
  WSettingsLinkRow,
  WSettingsNote,
  WSettingsNumberRow,
  WSettingsProgressRow,
  WSettingsSecretRow,
  WSettingsSelectRow,
  WSettingsStatusRow,
  WSettingsTextRow,
  WSettingsToggleRow,
} from "../../../../system";
import {
  costLine,
  dataTermsLine,
  hostedNotice,
  keyCheckLine,
  modelField,
  modelOptions,
  modelWarningLine,
  monthSpent,
  ollamaWarningLine,
  onRampOnceKnown,
  providerLabel,
  providers,
  whimbrelAllowanceLine,
  whimbrelPrepaidLine,
  whimbrelSnap,
} from "../../../aiModel";
import { useJump } from "../../jump";
import { budgetForAmount, budgetInvalidMessage, budgetOffTitle, budgetOffWords, monthBudget, onRampWords } from "./aiRedesignModel";
import { aiMessage, type Ai } from "./useAi";

type Settings = AiSettings;

/** The first card: Local-only mode first, then where AI runs and the switches that route it. */
export function AiRoutingCard({ ai, settings }: { ai: Ai; settings: Settings }) {
  const { error, saveRouting, whimbrelModels, loadWhimbrelModels, local, ollamaUp, whimbrel } = ai;
  const routing = settings.routing;
  const provider = routing.preferredProvider;
  const whimbrelSignedIn = !!whimbrel?.signedIn;
  const jumpLocalOnly = useJump("ai.localOnly");
  const jumpProvider = useJump("ai.provider");
  const [models, setModels] = useState<Partial<Record<Provider, string[]>>>({});
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const list = provider === "whimbrel" ? (whimbrelModels ?? undefined) : models[provider];
  const info = (p: Provider) => settings.providers.find((x) => x.id === p);
  const modelValue = routing[modelField(provider)];

  useEffect(() => setListError(null), [provider]);
  // Whimbrel AI's list needs no key: read it from the server when the picker shows it.
  useEffect(() => {
    if (provider === "whimbrel" && !whimbrelModels) loadWhimbrelModels().catch(() => undefined);
  }, [provider, whimbrelModels]);
  // A pick the list no longer offers snaps to the default, as on the Mac.
  useEffect(() => {
    if (provider !== "whimbrel" || !whimbrelModels) return;
    const snapped = whimbrelSnap(whimbrelModels, routing.whimbrelModel);
    if (snapped) saveRouting({ ...routing, whimbrelModel: snapped });
  }, [provider, whimbrelModels, routing.whimbrelModel]);

  const loadModels = () => {
    setLoading(true);
    setListError(null);
    providerModels(provider)
      .then((loaded) => setModels((m) => ({ ...m, [provider]: loaded })))
      .catch((e: unknown) => setListError(aiMessage(e)))
      .finally(() => setLoading(false));
  };

  const saveModel = (value: string) => {
    const v = value.trim();
    if (v && v !== routing[modelField(provider)]) saveRouting({ ...routing, [modelField(provider)]: v });
  };

  const notice = hostedNotice(routing, settings.keys, whimbrelSignedIn);
  const onRamp = onRampOnceKnown(routing, settings.keys, ollamaUp, local ? local.installed : null, whimbrelSignedIn);
  const words = onRampWords(provider);
  const google = info("google");
  const openUrl = (url: string | null | undefined) => url && void openExternal(url);

  return (
    <WSettingsCard title={t("AI")} headingLevel={3} error={error}>
      <WSettingsToggleRow
        id="ai.localOnly"
        highlighted={jumpLocalOnly}
        label={t("Local-only mode")}
        description={t("Send nothing to hosted providers.")}
        checked={routing.localOnly}
        onToggle={(v) => saveRouting({ ...routing, localOnly: v })}
      />
      {onRamp && (
        <div data-testid="ai-on-ramp">
          <WNoticeBox
            title={words.title}
            actions={
              <>
                <WButton onClick={() => openUrl("https://ollama.com/download")}>{t("Install Ollama")}</WButton>
                <WButton onClick={() => openUrl(google?.keyPageUrl)}>{t("Get a free Gemini key")}</WButton>
              </>
            }
          >
            <p>{words.body}</p>
            <p>{words.note}</p>
          </WNoticeBox>
        </div>
      )}
      {local?.installed && (
        <WSettingsControlRow label={t("Small Local Model")}>
          <WButton onClick={() => void localModelRemove().then(ai.setLocalStatus, (e: unknown) => ai.setError(aiMessage(e)))}>{t("Remove the Small Local Model")}</WButton>
        </WSettingsControlRow>
      )}
      <WSettingsSelectRow
        id="ai.provider"
        highlighted={jumpProvider}
        label={t("Preferred provider")}
        value={provider}
        options={providers().map((p) => ({ value: p.id, label: p.label }))}
        onChange={(v) => saveRouting({ ...routing, preferredProvider: v as Provider })}
      />
      {list ? (
        <WSettingsSelectRow
          key={provider}
          label={t("{provider} model", { provider: providerLabel(provider) })}
          description={provider === "openrouter" ? openRouterNote() : undefined}
          value={modelValue}
          options={[
            ...(provider === "openrouter" ? [{ value: "", label: t("Not picked") }] : []),
            ...modelOptions(list, modelValue).map((m) => ({ value: m, label: m })),
          ]}
          onChange={saveModel}
        />
      ) : (
        <WSettingsTextRow
          key={provider}
          label={t("{provider} model", { provider: providerLabel(provider) })}
          description={provider === "openrouter" ? openRouterNote() : undefined}
          placeholder={provider === "openrouter" ? t("vendor/model or vendor/model:free") : undefined}
          value={modelValue}
          required
          requiredMessage={t("Enter a model name.")}
          onCommit={saveModel}
        />
      )}
      {provider !== "whimbrel" && (
        <WSettingsControlRow
          label={t("Model list")}
          description={list ? t("{count} models from {provider}.", { count: list.length, provider: providerLabel(provider) }) : undefined}
          error={listError}
        >
          <WButton disabled={loading} onClick={loadModels}>
            {loading ? t("Loading…") : list ? t("Reload") : t("Load models")}
          </WButton>
        </WSettingsControlRow>
      )}
      <WSettingsToggleRow
        label={t("Use local model (Ollama)")}
        description={
          routing.localOnly
            ? t("Local-only mode is on, so the local model is always used.")
            : t("Off skips Ollama entirely. Turn it off on a PC that does not run Ollama, so calls stop trying to reach it first.")
        }
        checked={routing.useLocalModel}
        onToggle={(v) => saveRouting({ ...routing, useLocalModel: v })}
      />
      <WSettingsToggleRow
        label={t("Use the cheapest model for small tasks")}
        description={t(
          "Conversation titles, topic names and profile suggestions go to the local model first, then to the cheapest model you have a key for. Chat, summaries and digests keep using your preferred provider. Each routine picks its own model in Automations.",
        )}
        checked={routing.cheapFirst}
        onToggle={(v) => saveRouting({ ...routing, cheapFirst: v })}
      />
      <WSettingsToggleRow
        label={t("My Google AI Studio project has billing on")}
        description={t("Gmail and Calendar text goes to Gemini only when its project has billing on, because Google may train on the free tier.")}
        checked={routing.googleAiBillingConfirmed}
        onToggle={(v) => saveRouting({ ...routing, googleAiBillingConfirmed: v })}
      />
      <WSettingsToggleRow
        label={t("Allow Gmail and Calendar text with a cloud or remote Ollama model")}
        description={t("Off keeps Gmail and Calendar text on this computer when Ollama runs elsewhere.")}
        checked={routing.remoteOllamaGoogleData}
        onToggle={(v) => saveRouting({ ...routing, remoteOllamaGoogleData: v })}
      />
      {notice && <WSettingsNote>{notice}</WSettingsNote>}
      {settings.ollamaWarning && (
        <WNoticeBox tone="warning" live>
          {ollamaWarningLine(settings.ollamaWarning)}
        </WNoticeBox>
      )}
      {/* Gone once the user picks another model, before the next reload. */}
      {settings.modelWarnings
        .filter((w) => routing[modelField(w.provider)] === w.model)
        .map((w) => (
          <WNoticeBox key={w.provider} tone="warning" live>
            {modelWarningLine(w)}
          </WNoticeBox>
        ))}
    </WSettingsCard>
  );
}

const openRouterNote = () =>
  t(
    "One key for any model on OpenRouter's catalogue, free ones included. Until you pick one, OpenRouter is left out of every chain, because their catalogue changes weekly and there is no fallback model to substitute.",
  );

/** Whimbrel AI: prepaid credit through Whimbrel's server, signed in with sync. The balance, the two packs, the data note. */
export function WhimbrelAiCard({ view, refreshModels }: { view: WhimbrelAiView | null; refreshModels: () => Promise<void> }) {
  const { locale } = useL10n();
  const [current, setCurrent] = useState(view);
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setCurrent(view), [view]);
  const buy = (pack: "10" | "25") => {
    setBusy(true);
    setError(null);
    whimbrelAiBuy(pack)
      .then(setCurrent, (e: unknown) => setError(aiMessage(e)))
      .finally(() => setBusy(false));
  };
  const refresh = () => {
    setRefreshing(true);
    setError(null);
    refreshModels()
      .catch((e: unknown) => setError(aiMessage(e)))
      .finally(() => setRefreshing(false));
  };
  const allowance = current?.signedIn ? whimbrelAllowanceLine(current, locale) : null;
  const prepaid = current?.signedIn ? whimbrelPrepaidLine(current) : null;
  return (
    <WSettingsCard id="ai.whimbrelAI" title={providerLabel("whimbrel")} headingLevel={3} error={error}>
      <div data-testid="whimbrel-ai-card" className="rs-ai-wrap">
        {current?.signedIn ? (
          <>
            {allowance && <WSettingsNote>{allowance}</WSettingsNote>}
            {prepaid && <WSettingsNote>{prepaid}</WSettingsNote>}
            {current.low && (
              <WNoticeBox tone="warning" live>
                {t("Credit is running low.")}
              </WNoticeBox>
            )}
            <WSettingsControlRow label={t("Add credit")}>
              <WButton disabled={busy} onClick={() => buy("10")}>
                {t("Add $10")}
              </WButton>
              <WButton disabled={busy} onClick={() => buy("25")}>
                {t("Add $25")}
              </WButton>
            </WSettingsControlRow>
            <WSettingsControlRow label={t("Models")}>
              <WButton disabled={refreshing} onClick={refresh}>
                {refreshing ? t("Loading…") : t("Refresh Models")}
              </WButton>
            </WSettingsControlRow>
          </>
        ) : (
          <WSettingsNote>{t("Sign in to sync to use Whimbrel AI")}</WSettingsNote>
        )}
        {/* Core's `whimbrel_ai_passthrough` note, in the user's language (as on the Mac, PR 691). */}
        <WSettingsNote>{t("Your request goes through Whimbrel's server to Anthropic. Whimbrel keeps a record of tokens used, not what you asked.")}</WSettingsNote>
      </div>
    </WSettingsCard>
  );
}

/** API keys: one group per provider, each key on a secret row (a saved key is never drawn), then the image search key. */
export function AiKeysCard({ ai, settings }: { ai: Ai; settings: Settings }) {
  const { setSettings, setError } = ai;
  const info = (p: Provider) => settings.providers.find((x) => x.id === p);
  return (
    <WSettingsCard id="ai.keys" title={t("API keys")} headingLevel={3} description={t("Stored in Windows Credential Manager under the same names the Mac's Keychain uses.")}>
      {settings.keys.map((k) => (
        <KeyBlock
          key={k.provider}
          provider={k.provider}
          info={info(k.provider)}
          usage={settings.usage}
          saved={!!k.masked}
          syncEnabled={k.syncEnabled}
          budgetCents={settings.budgets.find((b) => b.provider === k.provider)?.cents ?? null}
          onSave={(key) =>
            setApiKey(k.provider, key).then((keys) => {
              setSettings((s) => s && { ...s, keys });
              setError(null);
            })
          }
          onSync={(enabled) =>
            setApiKeySync(k.provider, enabled)
              .then((keys) => {
                setSettings((s) => s && { ...s, keys });
                setError(null);
              })
              .catch((e: unknown) => setError(aiMessage(e)))
          }
          onBudget={(dollars) =>
            setProviderBudget(k.provider, dollars)
              .then((budgets) => {
                setSettings((s) => s && { ...s, budgets });
                setError(null);
              })
              .catch((e: unknown) => setError(aiMessage(e)))
          }
        />
      ))}
      {settings.keys.some((k) => k.masked) && <WSettingsNote>{t("Includes your other computers.")}</WSettingsNote>}
      <ImageSearchKey
        saved={!!settings.imageSearchKey}
        onSave={(key) =>
          setImageSearchKey(key).then((imageSearchKey) => {
            setSettings((s) => s && { ...s, imageSearchKey });
            setError(null);
          })
        }
      />
    </WSettingsCard>
  );
}

function KeyBlock(props: {
  provider: Provider;
  info: ProviderInfoView | undefined;
  usage: UsageView | null | undefined;
  saved: boolean;
  syncEnabled: boolean;
  budgetCents: number | null;
  onSave: (key: string | null) => Promise<void>;
  onSync: (enabled: boolean) => void;
  onBudget: (dollars: number | null) => void;
}) {
  const { provider, info, saved } = props;
  const [busy, setBusy] = useState(false);
  const [check, setCheck] = useState<{ text: string; isError: boolean } | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const label = providerLabel(provider);
  const openUrl = (url: string | null | undefined) => url && void openExternal(url);

  // Save, then ask the provider whether it takes the key. The key itself goes nowhere else. Resolves true once the key
  // is stored (the row then clears what was typed), false when saving failed (the row keeps it so it can be corrected).
  const save = (key: string): Promise<boolean> => {
    setBusy(true);
    setSaveError(null);
    setCheck({ text: t("Saved. Checking the key…"), isError: false });
    return props
      .onSave(key)
      .then(
        () =>
          keyCheck(provider)
            .then(
              (result) => setCheck(keyCheckLine(result)),
              (e: unknown) => {
                setCheck(null);
                setSaveError(aiMessage(e));
              },
            )
            .then(() => true),
        (e: unknown) => {
          setCheck(null);
          setSaveError(aiMessage(e));
          return false;
        },
      )
      .finally(() => setBusy(false));
  };
  const remove = () => {
    setCheck(null);
    setBusy(true);
    props
      .onSave(null)
      .catch((e: unknown) => setSaveError(aiMessage(e)))
      .finally(() => setBusy(false));
  };

  return (
    <WSettingsCard title={label} headingLevel={4}>
      <WSettingsSecretRow
        label={t("{provider} API key", { provider: label })}
        saved={saved}
        onSave={save}
        onRemove={remove}
        removeQuestion={
          props.syncEnabled
            ? t("Remove the {provider} key from this PC? Sync is on for this key, so it is also removed from your other computers.", { provider: label })
            : t("Remove the {provider} key from this PC?", { provider: label })
        }
        placeholder={t("Paste key")}
        busy={busy}
        status={check && !check.isError ? check.text : undefined}
        error={saveError ?? (check?.isError ? check.text : null)}
      />
      {/* Off by default; the key moves only between devices that have this on (as on the Mac). */}
      <WSettingsToggleRow label={t("Sync to devices")} checked={props.syncEnabled} onToggle={props.onSync} />
      {info?.keyPageUrl && (
        <WSettingsLinkRow kind="external" label={t("Key page")} linkText={t("Get an API key from {provider}", { provider: info.name })} href={info.keyPageUrl} onPress={() => openUrl(info.keyPageUrl)} />
      )}
      {info?.policyUrl && <WSettingsLinkRow kind="external" label={t("Data policy")} href={info.policyUrl} onPress={() => openUrl(info.policyUrl)} />}
      {info && <WSettingsNote>{costLine(info, props.usage)}</WSettingsNote>}
      {saved && <MonthlyBudget provider={label} spentCents={monthSpent(props.usage, provider)} budgetCents={props.budgetCents} onBudget={props.onBudget} />}
      {info && dataTermsLine(info) && <WSettingsNote>{dataTermsLine(info)}</WSettingsNote>}
    </WSettingsCard>
  );
}

/** A paid provider's month against its budget: the line, a bar, and the amount. A bad amount writes nothing. */
function MonthlyBudget(props: { provider: string; spentCents: number; budgetCents: number | null; onBudget: (dollars: number | null) => void }) {
  const [custom, setCustom] = useState(false);
  const [askNone, setAskNone] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const month = monthBudget(props.spentCents, props.budgetCents);
  const none = props.budgetCents === null || props.budgetCents <= 0;
  return (
    <>
      {askNone && (
        <WConfirmDialog
          title={budgetOffTitle(props.provider)}
          body={budgetOffWords(props.provider)}
          confirmLabel={t("Remove the budget")}
          onCancel={() => setAskNone(false)}
          onConfirm={() => {
            setAskNone(false);
            setCustom(false);
            props.onBudget(null);
          }}
        />
      )}
      {month.fill !== null ? (
        <>
          <WSettingsProgressRow label={t("This month")} progress={month.fill} valueText={month.value} over={month.over} />
          {month.over && <WSettingsStatusRow label={t("Budget")} description={t("This month's spending has reached the budget.")} status={{ tone: "problem", children: t("Over budget") }} />}
        </>
      ) : (
        <WSettingsNote>{month.none}</WSettingsNote>
      )}
      <WSettingsNumberRow
        label={t("Monthly budget")}
        value={(props.budgetCents ?? 0) / 100}
        min={0}
        integer={false}
        money
        unit={t("dollars")}
        invalidMessage={budgetInvalidMessage()}
        error={problem}
        companion={{
          label: t("No budget"),
          checked: none && !custom,
          onChange: (noBudget) => {
            setProblem(null);
            // Ticking "No budget" on a real budget removes the limit: ask first.
            if (noBudget && !none) return setAskNone(true);
            setCustom(!noBudget);
          },
        }}
        onCommit={(n) => {
          const next = budgetForAmount(n);
          if (!next) return setProblem(budgetInvalidMessage());
          setProblem(null);
          // 0 is "no budget": the same question as ticking the box.
          if (next.dollars === null && !none) return setAskNone(true);
          if (next.cents !== props.budgetCents) props.onBudget(next.dollars);
          if (next.dollars === null) setCustom(false);
        }}
      />
    </>
  );
}

/** The skin maker's web image search key. Not a provider: no model list to check it against. */
function ImageSearchKey(props: { saved: boolean; onSave: (key: string | null) => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (key: string | null): Promise<boolean> => {
    setBusy(true);
    setError(null);
    return props
      .onSave(key)
      .then(
        () => true,
        (e: unknown) => {
          setError(aiMessage(e));
          return false;
        },
      )
      .finally(() => setBusy(false));
  };
  return (
    <WSettingsCard title={t("Image Search Key (Brave Search)")} headingLevel={4}>
      <WSettingsSecretRow
        label={t("Brave Search API key")}
        description={t("The skin maker uses it to find images on the web. It stays on this PC.")}
        saved={props.saved}
        onSave={run}
        onRemove={() => void run(null)}
        removeQuestion={t("Remove the Brave Search key from this PC?")}
        placeholder={t("Brave Search API key")}
        busy={busy}
        error={error}
      />
    </WSettingsCard>
  );
}
