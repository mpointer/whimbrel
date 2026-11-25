import { useEffect, useState } from "react";
import { setPrompt, setSummaryInstructions, usageRows } from "../../../../api";
import { t, useL10n } from "../../../../l10n";
import type { AiSettings, PromptSurface, RoutingSettings, UsageRowView } from "../../../../generated/types";
import {
  WButton,
  WConfirmDialog,
  WSettingsCard,
  WSettingsChoiceRow,
  WSettingsControlRow,
  WSettingsDisclosure,
  WSettingsNote,
  WSettingsNumberRow,
  WSettingsTextRow,
  WSettingsValueList,
} from "../../../../system";
import { formatCents, surfaces, usageTotalsLine } from "../../../aiModel";
import { capForMode, capInvalidMessage, capOffWords, capModeOf, dollarsToCents, usageItems, type CapMode } from "./aiRedesignModel";
import { aiMessage, type Ai } from "./useAi";
import "./ai.css";

/** What the summary and per-feature instructions say. Saved when the box loses focus and the text changed. */
export function AiInstructionsCard({ ai, settings }: { ai: Ai; settings: AiSettings }) {
  const [summary, setSummary] = useState(settings.summaryInstructions);
  const [prompts, setPrompts] = useState<Record<string, string>>(Object.fromEntries(settings.prompts.map((p) => [p.surface, p.text])));
  const done = (p: Promise<void>) => p.then(() => ai.setError(null)).catch((e: unknown) => ai.setError(aiMessage(e)));
  return (
    <WSettingsCard id="ai.instructions" title={t("Instructions")} headingLevel={3}>
      <WSettingsTextRow
        label={t("Every summary")}
        placeholder={t("e.g. Use bullet points. Call out decisions and owners.")}
        rows={3}
        mono
        value={summary}
        onCommit={(text) => {
          setSummary(text);
          void done(setSummaryInstructions(text));
        }}
      />
      <WSettingsDisclosure title={t("Per-feature instructions")}>
        {surfaces().map((s) => (
          <WSettingsTextRow
            key={s.id}
            label={s.label}
            rows={3}
            mono
            value={prompts[s.id] ?? ""}
            onCommit={(text) => {
              setPrompts((p) => ({ ...p, [s.id]: text }));
              void done(setPrompt(s.id as PromptSurface, text));
            }}
          />
        ))}
      </WSettingsDisclosure>
    </WSettingsCard>
  );
}

/** A daily cap: the built-in one, an amount, or off. Only a real dollar amount is written. */
function CapRows(props: { label: string; amountLabel: string; cents: number | null | undefined; onSave: (cents: number | null) => void }) {
  const [picked, setPicked] = useState<CapMode | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [askOff, setAskOff] = useState(false);
  const stored = capModeOf(props.cents);
  const mode = picked ?? stored;
  return (
    <>
      {askOff && (
        <WConfirmDialog
          title={t("Turn off {cap}?", { cap: props.label })}
          body={capOffWords()}
          confirmLabel={t("Turn off the cap")}
          onCancel={() => setAskOff(false)}
          onConfirm={() => {
            setAskOff(false);
            setPicked(null);
            props.onSave(0);
          }}
        />
      )}
      <WSettingsChoiceRow
        manual
        label={props.label}
        description={t("Built-in uses the app's own cap. Off removes the cap.")}
        value={mode}
        options={[
          { value: "builtin", label: t("Built-in") },
          { value: "custom", label: t("Set an amount") },
          { value: "off", label: t("Off") },
        ]}
        onChange={(next) => {
          setProblem(null);
          const write = capForMode(next);
          if (write === undefined) return setPicked("custom");
          // Off removes the limit on spending: ask first, whatever way it was chosen.
          if (next === "off") return next === stored ? setPicked(null) : setAskOff(true);
          setPicked(null);
          if (next !== stored) props.onSave(write);
        }}
      />
      {mode === "custom" && (
        <WSettingsNumberRow
          label={props.amountLabel}
          value={props.cents ? props.cents / 100 : 0}
          min={0}
          integer={false}
          money
          unit={t("dollars")}
          invalidMessage={capInvalidMessage()}
          error={problem}
          onCommit={(n) => {
            const cents = dollarsToCents(n);
            if (cents === undefined) return setProblem(capInvalidMessage());
            setProblem(null);
            // Zero is the same as Off: ask the same question, and keep the amount row as it was until it is answered.
            if (cents === 0) return setAskOff(true);
            setPicked(null);
            props.onSave(cents);
          }}
        />
      )}
    </>
  );
}

/** What every call cost, from the ledger the spend caps already keep, and the two daily caps. */
export function AiUsageCard({ ai, settings }: { ai: Ai; settings: AiSettings }) {
  const [rows, setRows] = useState<UsageRowView[]>([]);
  const [days, setDays] = useState(7);
  const { locale } = useL10n();
  const routing = settings.routing;
  const saveCap = (field: "userDailyCapCents" | "globalDailyCapCents") => (cents: number | null) => {
    ai.setError(null);
    ai.saveRouting({ ...routing, [field]: cents } as RoutingSettings);
  };

  useEffect(() => {
    usageRows(days).then(setRows, (e: unknown) => ai.setError(aiMessage(e)));
  }, [days]);

  const groups = usageItems(rows, locale);
  return (
    <WSettingsCard id="ai.usage" title={t("Model usage")} headingLevel={3}>
      {settings.spend && (
        <WSettingsNote>{t("Spent today: {amount} across hosted providers (resets at midnight UTC).", { amount: formatCents(settings.spend.globalCents) })}</WSettingsNote>
      )}
      {settings.usage && <WSettingsNote>{usageTotalsLine(settings.usage)}</WSettingsNote>}
      <CapRows label={t("Your daily cap")} amountLabel={t("Your daily cap amount")} cents={routing.userDailyCapCents} onSave={saveCap("userDailyCapCents")} />
      <CapRows label={t("App-wide daily cap")} amountLabel={t("App-wide daily cap amount")} cents={routing.globalDailyCapCents} onSave={saveCap("globalDailyCapCents")} />
      {groups.length === 0 ? (
        <WSettingsNote>{t("No model calls yet.")}</WSettingsNote>
      ) : (
        groups.map((g) => (
          <div key={g.day} className="rs-ai-day">
            <h4 className="rs-ai-day-title">{g.day}</h4>
            <WSettingsValueList label={g.day} items={g.items} />
          </div>
        ))
      )}
      <WSettingsControlRow label={t("Time range")}>
        <WButton onClick={() => setDays(days === 7 ? 30 : 7)}>{days === 7 ? t("Show last 30 days") : t("Show last 7 days")}</WButton>
      </WSettingsControlRow>
    </WSettingsCard>
  );
}
