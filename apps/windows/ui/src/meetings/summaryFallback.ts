import type { SummaryFallbackView } from "../generated/types";
import { t } from "../l10n";

/** What a core `fallback_reason` code means to the user; reasons that read the same share one
 *  sentence (the Mac's `SummaryFallbackNotice`). */
type Cause = "signedOut" | "credit" | "model" | "unavailable";

function cause(reason: string): Cause {
  switch (reason) {
    case "not_signed_in":
    case "no_device_key":
    case "unauthorized":
      return "signedOut";
    case "budget":
    case "credit_exhausted":
      return "credit";
    case "model_unavailable":
      return "model";
    default:
      return "unavailable";
  }
}

/** "Summarized on this PC with gemma4:12b because you were not signed in to Whimbrel." One whole
 *  sentence per cause, never composed from pieces, so each translates as a unit. */
export function fallbackLine(f: SummaryFallbackView): string {
  const model = f.model;
  const onThisPc = f.used === "ollama" || f.used === "local";
  switch (cause(f.reason)) {
    case "signedOut":
      return onThisPc
        ? t("Summarized on this PC with {model} because you were not signed in to Whimbrel.", { model })
        : t("Summarized with {model} because you were not signed in to Whimbrel.", { model });
    case "credit":
      return onThisPc
        ? t("Summarized on this PC with {model} because your Whimbrel AI credit or budget ran out.", { model })
        : t("Summarized with {model} because your Whimbrel AI credit or budget ran out.", { model });
    case "model":
      return onThisPc
        ? t("Summarized on this PC with {model} because Whimbrel does not offer the chosen model.", { model })
        : t("Summarized with {model} because Whimbrel does not offer the chosen model.", { model });
    default:
      return onThisPc
        ? t("Summarized on this PC with {model} because Whimbrel was not available.", { model })
        : t("Summarized with {model} because Whimbrel was not available.", { model });
  }
}

/** The button: decided by whether this PC is signed in now, not by the stored reason, so after
 *  signing in (the usual fix) it offers the re-summary. */
export function fallbackAction(signedIn: boolean): "resummarize" | "signIn" {
  return signedIn ? "resummarize" : "signIn";
}
