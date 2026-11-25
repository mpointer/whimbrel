// What the redesigned Connectors section says, apart from the components. The old models (accountsModel,
// gmailModel, mcpModel, RemoveGoogleAccount.removalMessage) hold the words; this holds only what the redesign adds.
import type { McpServerView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import type { WStatusTone } from "../../../../system";
import { serverLine } from "../../../mcpModel";

/** The chip on the MCP server row: the state in words, with a tone that repeats it. */
export function mcpServerChip(view: McpServerView): { tone: WStatusTone; text: string } {
  if (view.status === "running") return { tone: "good", text: serverLine(view) };
  if (view.status === "failed") return { tone: "problem", text: t("Not running.") };
  return { tone: "neutral", text: serverLine(view) };
}

/** The card's alert for a server that failed: the whole line, with Rust's reason. Null otherwise. */
export const mcpServerError = (view: McpServerView): string | null => (view.status === "failed" ? serverLine(view) : null);

/** Why Create token is off, in words, or null when it is on. */
export function createTokenReason(name: string, scopes: string[]): string | null {
  if (name.trim().length === 0) return t("Give the assistant a name.");
  if (scopes.length === 0) return t("Choose at least one permission.");
  return null;
}

/** Why "Save" for your own Google client is off, in words, or null. A saved client is saved as a pair. */
export function ownClientReason(clientId: string): string | null {
  return clientId.trim().length === 0 ? t("Enter the Client ID first. It is saved together with the client secret.") : null;
}

/** The label of an account's button, with the account named, so a list of "Disconnect" buttons is not a list of one name. */
export const forAccount = (action: string, email: string): string => `${action}, ${email}`;

/** Shown while the Client ID box differs from the saved one: the saved secret does not carry over, so both are entered again. */
export const clientIdChangedHint = (): string => t("The Client ID has changed. Choose Replace and enter the client secret again to save both.");
