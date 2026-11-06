import type { CommandError, ProviderFailureReason } from "./generated/types";
import { t } from "./l10n";
import { formatDollars, providers } from "./settings/aiModel";

/** A sentence for the user, in the UI language. The spend-cap wording follows the Mac's. The shell
 *  renders every other kind's `message` in the UI language itself (W5), so it is shown as is. */
export function describeError(err: CommandError): string {
  switch (err.kind) {
    case "spend_cap": {
      // Cents as strings, so "1487¢" doesn't gain a separator.
      const amounts = { spent: String(err.spent_cents), cap: String(err.cap_cents) };
      return err.scope === "user"
        ? t("You've reached today's spend cap ({spent}¢ of {cap}¢). It resets at midnight UTC, or raise it in Settings.", amounts)
        : t("Whimbrel's app-wide spend limit for today is reached ({spent}¢ of {cap}¢). Try again tomorrow.", amounts);
    }
    case "provider_failure":
      return (
        (err.reason && providerFailureReason(err.reason)) ??
        t("No model provider could answer. Check that Ollama is running or that a hosted provider has a key in Settings.")
      );
    case "budget_spent": {
      const known = providers().find((p) => p.id === err.provider);
      return t("Your {provider} budget for this month is spent ({spent} of {budget}). Raise it in Settings → AI Providers.", {
        provider: known?.label ?? err.provider,
        spent: formatDollars(err.spent_cents),
        budget: formatDollars(err.budget_cents),
      });
    }
    default:
      return err.message;
  }
}

/** Why every provider failed, the Mac's sentences (core #575); `null` keeps the general line. */
export function providerFailureReason(reason: ProviderFailureReason): string | null {
  const name = (p: string) => providers().find((x) => x.id === p)?.label ?? p;
  switch (reason.kind) {
    case "ollama_model_missing":
      return t("Ollama doesn't have the model {model}. Pick one you have in Settings → AI Providers, or run: ollama pull {model}", {
        model: reason.model,
      });
    case "ollama_not_running":
      return t("Ollama isn't running on this computer. Start Ollama, or add an API key in Settings → AI Providers.");
    case "no_provider_configured":
      return t("No AI provider is set up. Add one in Settings → AI Providers.");
    case "unauthorized":
      return t("{provider} rejected the API key. Check it in Settings → AI Providers.", { provider: name(reason.provider) });
    case "rate_limited":
      return t("{provider} is busy or out of credit. Try again shortly.", { provider: name(reason.provider) });
    default:
      return null;
  }
}

/** Whether a rejected `invoke` carried a `CommandError` (vs. a Tauri-level failure). */
export function isCommandError(value: unknown): value is CommandError {
  return typeof value === "object" && value !== null && typeof (value as { kind?: unknown }).kind === "string";
}
