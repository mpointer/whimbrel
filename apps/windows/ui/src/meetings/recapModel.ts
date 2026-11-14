import type { RecapSummaryInput, WorkspaceView } from "../generated/types";
import { t } from "../l10n";

/** The summary as the page shows it, unsaved summary edits included. */
export function recapInput(view: WorkspaceView, summary: string | null): RecapSummaryInput {
  return {
    title: view.row.title,
    summary: summary ?? view.summary ?? "",
    decisions: view.decisions,
    actionItems: view.actionItems.map((a) => a.text),
    openQuestions: view.openQuestions,
  };
}

/** "Meeting recap: Planning (Nov 14, 2026)", the date in the app's language. */
export function recapSubject(title: string, startUnixMs: number, locale: string): string {
  const date = new Date(startUnixMs).toLocaleDateString(locale, { year: "numeric", month: "short", day: "numeric" });
  return t("Meeting recap: {title} ({date})", { title, date });
}

/** The meeting page's line once a recap went out from this PC. */
export function recapSentLine(count: number): string {
  return count === 1 ? t("Sent the recap to 1 person") : t("Sent the recap to {count} people", { count });
}

/** A plausible address to add by hand: no spaces or separators, one @, a dot after it. Core
 *  checks again before anything is sent. */
export function addressLooksUsable(email: string): boolean {
  return /^[^\s@,;<>"]+@[^\s@,;<>"]+\.[^\s@,;<>"]+$/.test(email.trim());
}

/** The ticked addresses plus any added by hand, once each (case-insensitive), in order. */
export function chosenRecipients(ticked: string[], added: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const e of [...ticked, ...added]) {
    const key = e.trim().toLowerCase();
    if (key && !seen.has(key)) {
      seen.add(key);
      out.push(e.trim());
    }
  }
  return out;
}
