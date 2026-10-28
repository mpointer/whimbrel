import type { McpClientView, McpServerView } from "../generated/types";
import { shortDate } from "../library/rows";

// What Settings → AI assistants says, apart from the component.

export interface ScopeInfo {
  id: "memory" | "meetings" | "transcripts";
  label: string;
  description: string;
}

/** The permissions a client can be given, said in terms of what it can actually read. */
export const SCOPES: ScopeInfo[] = [
  {
    id: "memory",
    label: "Memory",
    description:
      "Search and read what Whimbrel captured from your screen, your episode summaries, your routine outputs and the day digest, and the subjects, people and summaries of your email threads.",
  },
  {
    id: "meetings",
    label: "Meetings",
    description: "List your meetings and read each one's summary, decisions and action items.",
  },
  {
    id: "transcripts",
    label: "Transcripts",
    description:
      "Read meeting transcripts word for word, and let searches include what was said in meetings. This is the most sensitive of the three.",
  },
];

export const MCP_INTRO =
  "Let an AI assistant on this PC read your Whimbrel data. The server listens only on this PC, and a program can read only what a token you create here allows. " +
  "Turn it off, or revoke a token, and access ends at once.";

export const MCP_TOKEN_ONCE =
  "This is the only time this token is shown. Whimbrel keeps only a fingerprint of it, so it cannot show it again. " +
  "Copy it into your assistant now; the assistant will keep it in its own settings in plain text, where anything running as you can read it. Revoke it here if you stop using that assistant.";

export const revokeQuestion = (name: string): string =>
  `Revoke ${name}? It loses access at once, and to use it again you would need to make a new token.`;

export function serverLine(view: McpServerView): string {
  if (view.status === "running") return `Running at ${view.address}`;
  if (view.status === "failed") return `Not running: ${view.error ?? "it stopped and did not say why."}`;
  return "Not running.";
}

export const canCreate = (name: string, scopes: string[]): boolean => name.trim().length > 0 && scopes.length > 0;

/** Adds or removes a permission, keeping the card's order. */
export function toggleScope(current: string[], id: string): string[] {
  const next = current.includes(id) ? current.filter((s) => s !== id) : [...current, id];
  return SCOPES.map((s) => s.id).filter((s) => next.includes(s));
}

export function clientLine(client: McpClientView): string {
  if (client.revokedUnixMs != null) return `Revoked ${shortDate(client.revokedUnixMs)}`;
  return `Can read: ${client.scopes.join(", ")} · made ${shortDate(client.createdUnixMs)}`;
}

/** What an assistant needs to connect. Deliberately no recipe for any one program until one is proven here. */
export const connectHelp = (address: string): string =>
  `Point the assistant at ${address} and have it send the header "Authorization: Bearer <your token>" with each request.`;
