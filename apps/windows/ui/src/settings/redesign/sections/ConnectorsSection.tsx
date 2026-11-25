import type { SectionProps } from "../types";
import { AccountsCard } from "./connectors/AccountsCard";
import { GmailAccountsCard } from "./connectors/GmailAccountsCard";
import { McpCards } from "./connectors/McpCards";
import { OwnClientDisclosure } from "./connectors/OwnClientDisclosure";

/** Connectors: the Google accounts Whimbrel uses (Calendar, Gmail), your own Google Cloud project under Advanced, and the AI assistants that may read Whimbrel (MCP). Same commands as the old cards. */
export function ConnectorsSection(_props: SectionProps) {
  return (
    <>
      <AccountsCard />
      <GmailAccountsCard />
      <OwnClientDisclosure />
      <McpCards />
    </>
  );
}
