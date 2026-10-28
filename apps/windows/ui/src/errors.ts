import type { CommandError } from "./generated/types";

/** A sentence for the user. The spend-cap wording follows the Mac's. */
export function describeError(err: CommandError): string {
  switch (err.kind) {
    case "spend_cap":
      return err.scope === "user"
        ? `You've reached today's spend cap (${err.spent_cents}¢ of ${err.cap_cents}¢). It resets at midnight UTC, or raise it in Settings.`
        : `Whimbrel's app-wide spend limit for today is reached (${err.spent_cents}¢ of ${err.cap_cents}¢). Try again tomorrow.`;
    case "provider_failure":
      return "No model provider could answer. Check that Ollama is running or that a hosted provider has a key in Settings.";
    default:
      return err.message;
  }
}

/** Whether a rejected `invoke` carried a `CommandError` (vs. a Tauri-level failure). */
export function isCommandError(value: unknown): value is CommandError {
  return typeof value === "object" && value !== null && typeof (value as { kind?: unknown }).kind === "string";
}
