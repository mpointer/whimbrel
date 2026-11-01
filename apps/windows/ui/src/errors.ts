import type { CommandError } from "./generated/types";
import { t } from "./l10n";

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
      return t("No model provider could answer. Check that Ollama is running or that a hosted provider has a key in Settings.");
    default:
      return err.message;
  }
}

/** Whether a rejected `invoke` carried a `CommandError` (vs. a Tauri-level failure). */
export function isCommandError(value: unknown): value is CommandError {
  return typeof value === "object" && value !== null && typeof (value as { kind?: unknown }).kind === "string";
}
