import { useCallback, useEffect, useState } from "react";
import { mcpClients, mcpCreateClient, mcpRevokeClient, mcpServerState, mcpSetEnabled } from "../api";
import { describeError, isCommandError } from "../errors";
import type { McpClientView, McpNewClient, McpServerView } from "../generated/types";
import { MCP_INTRO, MCP_TOKEN_ONCE, SCOPES, canCreate, clientLine, connectHelp, revokeQuestion, serverLine, toggleScope } from "./mcpModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → AI assistants (M3.4 Task 5): the MCP server switch and the tokens that let a program read Whimbrel. */
export function McpCard() {
  const [server, setServer] = useState<McpServerView | null>(null);
  const [clients, setClients] = useState<McpClientView[]>([]);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [made, setMade] = useState<(McpNewClient & { name: string }) | null>(null);
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

  if (!server) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  return (
    <section className="card about" aria-labelledby="mcp-heading">
      <h2 id="mcp-heading" className="section-label">
        AI assistants (MCP)
      </h2>
      <p className="muted">{MCP_INTRO}</p>
      <label className="field-row">
        <input
          type="checkbox"
          checked={server.enabled}
          disabled={busy}
          onChange={(e) => run(() => mcpSetEnabled(e.target.checked).then(setServer))}
        />
        <span>Enable the MCP server</span>
      </label>
      <p className={server.status === "failed" ? undefined : "muted small"} role={server.status === "failed" ? "alert" : undefined}>
        {serverLine(server)}
      </p>
      {server.status === "running" && <p className="muted small">{connectHelp(server.address)}</p>}

      <h3 className="section-label">Add an assistant</h3>
      <div className="field-row">
        <input
          type="text"
          value={name}
          placeholder="Name, for example Claude Desktop"
          maxLength={60}
          aria-label="Assistant name"
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <ul className="rows">
        {SCOPES.map((s) => (
          <li key={s.id} className="field-row">
            <label>
              <input type="checkbox" checked={scopes.includes(s.id)} onChange={() => setScopes((cur) => toggleScope(cur, s.id))} />
              <strong> {s.label}</strong>
            </label>
            <span className="muted small">{s.description}</span>
          </li>
        ))}
      </ul>
      <div className="toolbar">
        <button
          className="mode mode--active"
          disabled={busy || !canCreate(name, scopes)}
          onClick={() =>
            run(async () => {
              const created = await mcpCreateClient(name.trim(), scopes);
              setMade({ ...created, name: name.trim() });
              setName("");
              setScopes([]);
              setClients(await mcpClients());
            })
          }
        >
          Create token
        </button>
      </div>

      {made && (
        <div role="dialog" aria-label="Your new token" className="card">
          <p>
            <strong>Token for {made.name}</strong>
          </p>
          <code className="mono" data-testid="mcp-token">
            {made.token}
          </code>
          <p className="muted small">{MCP_TOKEN_ONCE}</p>
          <div className="toolbar">
            <button className="mode" onClick={() => void navigator.clipboard?.writeText(made.token)}>
              Copy
            </button>
            <button className="mode mode--active" onClick={() => setMade(null)}>
              Done
            </button>
          </div>
        </div>
      )}

      <h3 className="section-label">Assistants</h3>
      {clients.length === 0 ? (
        <p className="muted small">None yet.</p>
      ) : (
        <ul className="rows">
          {clients.map((c) => (
            <li key={c.id} className="field-row">
              <span>{c.name}</span>
              <span className="muted small">{clientLine(c)}</span>
              {c.revokedUnixMs == null && (
                <button className="mode" disabled={busy} onClick={() => setRevoking(c)}>
                  Revoke…
                </button>
              )}
              {revoking?.id === c.id && (
                <span role="group" aria-label="Confirm revoking">
                  <span className="muted small">{revokeQuestion(c.name)}</span>
                  <button
                    className="mode mode--active"
                    disabled={busy}
                    onClick={() =>
                      run(async () => {
                        setClients(await mcpRevokeClient(c.id));
                        setRevoking(null);
                      })
                    }
                  >
                    Revoke
                  </button>
                  <button className="mode" onClick={() => setRevoking(null)}>
                    Cancel
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
