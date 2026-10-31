import type { McpClientView, McpServerView } from "../generated/types";
import { t } from "../l10n";
import { shortDate } from "../library/rows";

// What Settings → AI assistants says, apart from the component. Functions, not constants, so the
// text follows the current language.

export interface ScopeInfo {
  id: "memory" | "meetings" | "transcripts";
  label: string;
  description: string;
}

/** The scope ids the server understands, in the card's order. */
const SCOPE_IDS: ScopeInfo["id"][] = ["memory", "meetings", "transcripts"];

/** The permissions a client can be given, said in terms of what it can actually read. */
export function scopes(): ScopeInfo[] {
  return [
    {
      id: "memory",
      label: t("Memory"),
      description: t(
        "Search and read what Whimbrel captured from your screen, your episode summaries, your routine outputs and the day digest, and the subjects, people and summaries of your email threads.",
      ),
    },
    {
      id: "meetings",
      label: t("Meetings"),
      description: t("List your meetings and read each one's summary, decisions and action items."),
    },
    {
      id: "transcripts",
      label: t("Transcripts"),
      description: t(
        "Read meeting transcripts word for word, and let searches include what was said in meetings. This is the most sensitive of the three.",
      ),
    },
  ];
}

export const mcpIntro = (): string =>
  t(
    "Let an AI assistant on this PC read your Whimbrel data. The server listens only on this PC, and a program can read only what a token you create here allows. Turn it off, or revoke a token, and access ends at once.",
  );

export const mcpTokenOnce = (): string =>
  t(
    "This is the only time this token is shown. Whimbrel keeps only a fingerprint of it, so it cannot show it again. Copy it into your assistant now; the assistant will keep it in its own settings in plain text, where anything running as you can read it. Revoke it here if you stop using that assistant.",
  );

export const revokeQuestion = (name: string): string =>
  t("Revoke {name}? It loses access at once, and to use it again you would need to make a new token.", { name });

export function serverLine(view: McpServerView): string {
  if (view.status === "running") return t("Running at {address}", { address: view.address });
  if (view.status === "failed")
    return view.error ? t("Not running: {error}", { error: view.error }) : t("Not running: it stopped and did not say why.");
  return t("Not running.");
}

export const canCreate = (name: string, scopes: string[]): boolean => name.trim().length > 0 && scopes.length > 0;

/** Adds or removes a permission, keeping the card's order. */
export function toggleScope(current: string[], id: string): string[] {
  const next = current.includes(id) ? current.filter((s) => s !== id) : [...current, id];
  return SCOPE_IDS.filter((s) => next.includes(s));
}

export function clientLine(client: McpClientView, locale: string): string {
  if (client.revokedUnixMs != null) return t("Revoked {date}", { date: shortDate(client.revokedUnixMs, new Date(), locale) });
  return t("Can read: {scopes} · made {date}", {
    scopes: client.scopes.join(", "),
    date: shortDate(client.createdUnixMs, new Date(), locale),
  });
}

/** What an assistant needs to connect. Deliberately no recipe for any one program until one is proven here. */
export const connectHelp = (address: string): string =>
  t("Point the assistant at {address} and have it send the header \"Authorization: Bearer <your token>\" with each request.", {
    address,
  });
