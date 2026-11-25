import { describe, expect, it } from "vitest";
import type { McpServerView } from "../../../../generated/types";
import { clientIdChangedHint, createTokenReason, forAccount, mcpServerChip, mcpServerError, ownClientReason } from "./connectorsModel";

const server = (over: Partial<McpServerView> = {}): McpServerView => ({ address: "127.0.0.1:4777", enabled: true, status: "running", error: null, ...over });

describe("the MCP server row", () => {
  it("says Running at its address, good", () => {
    expect(mcpServerChip(server())).toEqual({ tone: "good", text: "Running at 127.0.0.1:4777" });
    expect(mcpServerError(server())).toBeNull();
  });
  it("says Not running in the chip and the reason in the card's alert when it failed", () => {
    const failed = server({ status: "failed", error: "port in use" });
    expect(mcpServerChip(failed)).toEqual({ tone: "problem", text: "Not running." });
    expect(mcpServerError(failed)).toBe("Not running: port in use");
    expect(mcpServerError(server({ status: "failed" }))).toBe("Not running: it stopped and did not say why.");
  });
  it("is neutral when it is off", () => {
    expect(mcpServerChip(server({ status: "stopped", enabled: false }))).toEqual({ tone: "neutral", text: "Not running." });
  });
});

describe("why Create token is off", () => {
  it("asks for a name first, then a permission, then nothing", () => {
    expect(createTokenReason("  ", [])).toBe("Give the assistant a name.");
    expect(createTokenReason("Claude Desktop", [])).toBe("Choose at least one permission.");
    expect(createTokenReason("Claude Desktop", ["memory"])).toBeNull();
  });
});

describe("your own Google client", () => {
  it("needs a Client ID before a secret is saved, and says the two are saved together", () => {
    expect(ownClientReason("")).toMatch(/Client ID first/);
    expect(ownClientReason(" id ")).toBeNull();
    expect(clientIdChangedHint()).toMatch(/Replace/);
  });
  it("names the account in a button", () => {
    expect(forAccount("Disconnect", "a@example.test")).toBe("Disconnect, a@example.test");
  });
});
