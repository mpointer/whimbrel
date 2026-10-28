import { describe, expect, it } from "vitest";
import {
  MCP_INTRO,
  MCP_TOKEN_ONCE,
  SCOPES,
  canCreate,
  clientLine,
  connectHelp,
  revokeQuestion,
  serverLine,
  toggleScope,
} from "./mcpModel";

describe("what each permission says it opens", () => {
  const by = (id: string) => SCOPES.find((s) => s.id === id)!;

  it("offers exactly the three the server understands, in the card's order", () => {
    expect(SCOPES.map((s) => s.id)).toEqual(["memory", "meetings", "transcripts"]);
  });

  it("names the email summaries under memory, because Gmail makes them readable here", () => {
    expect(by("memory").description).toMatch(/email thread/i);
    expect(by("memory").description).toMatch(/screen/i);
  });

  it("calls meeting transcripts the most sensitive", () => {
    expect(by("transcripts").description).toMatch(/most sensitive/);
    expect(by("transcripts").description).toMatch(/word for word|raw/i);
  });

  it("never says Mac, which only describes the other app", () => {
    for (const s of SCOPES) expect(s.description + s.label).not.toMatch(/\bMac\b/);
    expect(MCP_INTRO + MCP_TOKEN_ONCE).not.toMatch(/\bMac\b/);
  });
});

describe("the explanations", () => {
  it("says only programs holding a token can read, and that the token is shown once", () => {
    expect(MCP_INTRO).toMatch(/only on this PC/);
    expect(MCP_INTRO).toMatch(/token/);
    expect(MCP_TOKEN_ONCE).toMatch(/only time/);
    expect(MCP_TOKEN_ONCE).toMatch(/cannot show it again/);
  });

  it("warns that the token ends up in the assistant's own settings", () => {
    expect(MCP_TOKEN_ONCE).toMatch(/plain text/);
  });

  it("asks before revoking, names the client, and says access ends at once", () => {
    const q = revokeQuestion("Claude Desktop");
    expect(q).toMatch(/Claude Desktop/);
    expect(q).toMatch(/at once/);
    expect(q).toMatch(/new token/);
  });
});

describe("the server line", () => {
  const view = (status: string, error: string | null = null) => ({ enabled: true, status, address: "http://127.0.0.1:4319/mcp", error });

  it("gives the address when it is running", () => {
    expect(serverLine(view("running"))).toBe("Running at http://127.0.0.1:4319/mcp");
  });

  it("says plainly when it is off", () => {
    expect(serverLine({ ...view("stopped"), enabled: false })).toBe("Not running.");
  });

  it("shows why it failed", () => {
    expect(serverLine(view("failed", "port 4319 is already in use"))).toBe("Not running: port 4319 is already in use");
  });

  it("does not leave a failed server with no reason", () => {
    expect(serverLine(view("failed", null))).toBe("Not running: it stopped and did not say why.");
  });
});

describe("making a client", () => {
  it("needs a name and at least one permission", () => {
    expect(canCreate("Claude Desktop", ["memory"])).toBe(true);
    expect(canCreate("   ", ["memory"])).toBe(false);
    expect(canCreate("Claude Desktop", [])).toBe(false);
  });

  it("adds and removes a permission, keeping the card's order", () => {
    expect(toggleScope([], "transcripts")).toEqual(["transcripts"]);
    expect(toggleScope(["transcripts"], "memory")).toEqual(["memory", "transcripts"]);
    expect(toggleScope(["memory", "transcripts"], "memory")).toEqual(["transcripts"]);
  });
});

describe("the client list", () => {
  it("lists what it may read and when it was made", () => {
    const line = clientLine({ id: 1, name: "a", scopes: ["memory", "meetings"], createdUnixMs: new Date(2026, 9, 16, 9, 0).getTime(), revokedUnixMs: null });
    expect(line).toMatch(/^Can read: memory, meetings · made /);
  });

  it("says when access was revoked", () => {
    const line = clientLine({ id: 1, name: "a", scopes: ["memory"], createdUnixMs: 1, revokedUnixMs: new Date(2026, 9, 16, 9, 0).getTime() });
    expect(line).toMatch(/^Revoked /);
  });
});

describe("connecting an assistant", () => {
  it("gives the address and the header, and does not claim to know a recipe for any one program", () => {
    const help = connectHelp("http://127.0.0.1:4319/mcp");
    expect(help).toMatch(/http:\/\/127\.0\.0\.1:4319\/mcp/);
    expect(help).toMatch(/Authorization: Bearer/);
    expect(help).not.toMatch(/npx|claude_desktop_config/);
  });
});
