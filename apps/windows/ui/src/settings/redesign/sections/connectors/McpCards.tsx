import { useCallback, useEffect, useState } from "react";
import { mcpClients, mcpCreateClient, mcpRevokeClient, mcpServerState, mcpSetEnabled } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { McpClientView, McpServerView } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { WButton, WInlineConfirm, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsRow, WSettingsStatusRow, WSettingsToggleRow } from "../../../../system";
import { canCreate, clientLine, connectHelp, mcpIntro, revokeQuestion, scopes as scopeList, toggleScope } from "../../../mcpModel";
import { mcpToken } from "./mcpTokenStore";
import { createTokenReason, forAccount, mcpServerChip, mcpServerError } from "./connectorsModel";
import "../devices/devices.css";
import "./connectors.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** AI assistants (MCP): the server switch, adding an assistant (a token shown once, in McpTokenHost's dialog), and the assistants that have one (revoke asks first). */
export function McpCards() {
  const { locale } = useL10n();
  const [server, setServer] = useState<McpServerView | null>(null);
  const [clients, setClients] = useState<McpClientView[]>([]);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [revoking, setRevoking] = useState<McpClientView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [state, list] = await Promise.all([mcpServerState(), mcpClients()]);
      setServer(state);
      setClients(list);
    } catch (e) {
      setError(message(e));
    }
  }, []);

  useEffect(() => {
    void load();
    // A server that stops on its own should show up without reopening Settings.
    const timer = window.setInterval(() => {
      void mcpServerState().then(setServer).catch(() => {});
    }, 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  const run = (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    work()
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const chip = server ? mcpServerChip(server) : null;
  const reason = createTokenReason(name, scopes);
  return (
    <>
      <WSettingsCard id="connectors.mcp" title={t("AI assistants (MCP)")} headingLevel={3} description={mcpIntro()} loading={!server && !error} error={[server ? mcpServerError(server) : null, error]}>
        {server && chip && (
          <>
            <WSettingsToggleRow label={t("Enable the MCP server")} checked={server.enabled} disabled={busy} onToggle={(next) => run(() => mcpSetEnabled(next).then(setServer))} />
            <WSettingsStatusRow label={t("Status")} status={{ tone: chip.tone, children: chip.text }} />
            {server.status === "running" && <WSettingsNote>{connectHelp(server.address)}</WSettingsNote>}
          </>
        )}
      </WSettingsCard>
      {server && (
        <>
          <WSettingsCard title={t("Add an assistant")} headingLevel={3}>
            <WSettingsControlRow label={t("Assistant name")} stacked>
              <input
                id="rs-mcp-name"
                className="w-input"
                type="text"
                value={name}
                placeholder={t("Name, for example Claude Desktop")}
                maxLength={60}
                autoComplete="off"
                aria-label={t("Assistant name")}
                onChange={(e) => setName(e.target.value)}
              />
            </WSettingsControlRow>
            {scopeList().map((s) => (
              <WSettingsToggleRow key={s.id} kind="checkbox" label={s.label} description={s.description} checked={scopes.includes(s.id)} onToggle={() => setScopes((cur) => toggleScope(cur, s.id))} />
            ))}
            <WSettingsRow
              variant="button"
              label={t("New token")}
              actionLabel={t("Create token")}
              disabled={busy || !canCreate(name, scopes)}
              disabledReason={reason ?? undefined}
              onPress={() =>
                run(async () => {
                  const created = await mcpCreateClient(name.trim(), scopes);
                  // Held above the section switch (McpTokenHost), so the dialog survives a change of section.
                  mcpToken.hold({ ...created, name: name.trim() });
                  setName("");
                  setScopes([]);
                  setClients(await mcpClients());
                })
              }
            />
          </WSettingsCard>
          <WSettingsCard title={t("Assistants")} headingLevel={3}>
            {clients.length === 0 ? (
              <WSettingsNote>{t("None yet.")}</WSettingsNote>
            ) : (
              clients.map((c) => (
                <WSettingsControlRow key={c.id} label={c.name} description={clientLine(c, locale)} stacked>
                  {c.revokedUnixMs == null &&
                    (revoking?.id === c.id ? (
                      <WInlineConfirm
                        question={revokeQuestion(c.name)}
                        confirmLabel={t("Revoke")}
                        busy={busy}
                        onConfirm={() =>
                          run(async () => {
                            setClients(await mcpRevokeClient(c.id));
                            setRevoking(null);
                          })
                        }
                        onCancel={() => setRevoking(null)}
                      />
                    ) : (
                      <WButton aria-label={forAccount(t("Revoke…"), c.name)} disabled={busy} onClick={() => setRevoking(c)}>
                        {t("Revoke…")}
                      </WButton>
                    ))}
                </WSettingsControlRow>
              ))
            )}
          </WSettingsCard>
        </>
      )}
    </>
  );
}
