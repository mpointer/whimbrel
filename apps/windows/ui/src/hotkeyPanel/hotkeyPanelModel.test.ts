import { describe, expect, it } from "vitest";
import type { ChatEvent, HotkeyPanelRead, WindowAskReply, WindowReadView } from "../generated/types";
import { describeError } from "../errors";
import { askErrorText, blockedReason, contextLine, destinationText, initialState, reduce, type PanelState } from "./hotkeyPanelModel";

const read = (over: Partial<Extract<WindowReadView, { kind: "read" }>> = {}): WindowReadView => ({
  kind: "read",
  appName: "Mail",
  title: "Re: Q3 rollout",
  isSelection: true,
  truncated: false,
  ...over,
});

describe("contextLine", () => {
  it("names the app, title and selected text", () => {
    expect(contextLine(read(), { kind: "local" }, false).reading).toBe("Reading: Mail, 'Re: Q3 rollout' (selected text)");
  });
  it("says visible text when nothing was selected", () => {
    expect(contextLine(read({ isSelection: false, title: "Inbox" }), { kind: "local" }, false).reading).toBe(
      "Reading: Mail, 'Inbox' (visible text)",
    );
  });
  it("marks a truncated read as first part only", () => {
    expect(contextLine(read(), { kind: "local" }, true).reading).toBe(
      "Reading: Mail, 'Re: Q3 rollout' (selected text) — first part only",
    );
  });
  it("leaves the title out when it is blank", () => {
    expect(contextLine(read({ title: "  " }), { kind: "local" }, false).reading).toBe("Reading: Mail (selected text)");
  });
  it("gives the destination line for local, local-only and hosted", () => {
    expect(contextLine(read(), { kind: "local" }, false).destination).toBe("This will be answered on this computer");
    expect(contextLine(read(), { kind: "local_only" }, false).destination).toBe("This will be answered on this computer");
    const hosted = {
      anthropic: "The text above will be sent to Anthropic",
      openai: "The text above will be sent to OpenAI",
      google: "The text above will be sent to Google",
      openrouter: "The text above will be sent to OpenRouter",
      whimbrel: "The text above will be sent to Anthropic, through Whimbrel's server",
    };
    for (const [provider, text] of Object.entries(hosted)) {
      expect(destinationText({ kind: "hosted", provider })).toBe(text);
    }
  });
  it("has no reading line when nothing was read", () => {
    expect(contextLine({ kind: "unreadable", appName: "Mail" }, { kind: "local" }, false).reading).toBe("");
  });
});

describe("blockedReason", () => {
  it("explains an excluded app", () => {
    expect(blockedReason({ kind: "excluded", appName: "Mail", reason: "user" })).toBe("This app is excluded from capture.");
  });
  it("explains an unreadable window", () => {
    expect(blockedReason({ kind: "unreadable", appName: "Mail" })).toBe("I couldn't read this window.");
  });
  it("is null for a window that was read", () => {
    expect(blockedReason(read())).toBeNull();
  });
  it("says there is no local model, never asks for an API key", () => {
    const text = blockedReason(read(), { kind: "local_model_missing" });
    expect(text).toBe("No local model is installed.");
    expect(text).not.toMatch(/key/i);
  });
  it("uses the provider's reason for other refusals", () => {
    expect(blockedReason(read(), { kind: "ollama_not_running" })).not.toBeNull();
  });
  it("in local-only mode says to start Ollama, never to add an API key", () => {
    const text = blockedReason(read(), { kind: "ollama_not_running" }, { kind: "local_only" });
    expect(text).toBe("Ollama isn't running on this computer. Start Ollama to answer here.");
    expect(text).not.toMatch(/key/i);
  });
  it("names the Ollama model that is missing", () => {
    expect(blockedReason(read(), { kind: "ollama_model_missing", model: "gemma3" }, { kind: "local_only" })).toMatch(/gemma3/);
  });
});

const panelRead = (requestId: number, over: Partial<HotkeyPanelRead> = {}): HotkeyPanelRead => ({
  requestId,
  window: read(),
  destination: { kind: "local" },
  canAnswer: true,
  cannotAnswer: null,
  ...over,
});
const started = (requestId: number): WindowAskReply => ({ kind: "started", requestId, destination: { kind: "local" }, truncated: false });

function ready(requestId = 3): PanelState {
  return reduce(initialState, { type: "read", read: panelRead(requestId) });
}

describe("askErrorText", () => {
  it("words an ask-time Ollama failure for the destination", () => {
    const err = { kind: "provider_failure", reason: { kind: "ollama_not_running" } };
    expect(askErrorText(err, { kind: "local_only" })).toBe("Ollama isn't running on this computer. Start Ollama to answer here.");
    expect(askErrorText(err, { kind: "local" })).toMatch(/^Ollama isn't running.*API key/);
  });
  it("keeps other errors as describeError has them", () => {
    expect(askErrorText({ kind: "provider_failure", reason: { kind: "local_model_missing" } }, { kind: "local_only" })).toBe(
      "No local model is installed.",
    );
    expect(askErrorText(new Error("boom"), null)).toBe("Error: boom");
  });
});

describe("reduce", () => {
  it("blocks a local-only read whose Ollama isn't running, in local-only words", () => {
    const s = reduce(initialState, {
      type: "read",
      read: panelRead(3, { destination: { kind: "local_only" }, canAnswer: false, cannotAnswer: { kind: "ollama_not_running" } }),
    });
    expect(s.phase).toBe("blocked");
    expect(s.blocked).toBe("Ollama isn't running on this computer. Start Ollama to answer here.");
  });
  it("shows the lines and sends nothing until asked", () => {
    const s = ready();
    expect(s.phase).toBe("ready");
    expect(s.destination).toEqual({ kind: "local" });
  });
  it("blocks on an excluded window, and an ask does nothing", () => {
    let s = reduce(initialState, { type: "read", read: panelRead(3, { window: { kind: "excluded", appName: "Mail", reason: "user" } }) });
    expect(s.phase).toBe("blocked");
    expect(s.blocked).toBe("This app is excluded from capture.");
    s = reduce(s, { type: "ask" });
    expect(s.phase).toBe("blocked");
    expect(s.askSeq).toBe(0);
  });
  it("blocks when the destination cannot answer, with the local-model wording", () => {
    const s = reduce(initialState, {
      type: "read",
      read: panelRead(3, { canAnswer: false, cannotAnswer: { kind: "local_model_missing" } }),
    });
    expect(s.phase).toBe("blocked");
    expect(s.blocked).toBe("No local model is installed.");
  });
  it("ignores a read from an older request", () => {
    const s = reduce(ready(5), { type: "read", read: panelRead(4, { window: { kind: "unreadable", appName: "X" } }) });
    expect(s.requestId).toBe(5);
    expect(s.phase).toBe("ready");
  });
  it("streams deltas and completes into the answer", () => {
    let s = reduce(ready(), { type: "ask" });
    expect(s.phase).toBe("asking");
    const seq = s.askSeq;
    s = reduce(s, { type: "reply", askSeq: seq, reply: started(3) });
    s = reduce(s, { type: "event", askSeq: seq, event: { kind: "delta", text: "Hel" } });
    s = reduce(s, { type: "event", askSeq: seq, event: { kind: "delta", text: "lo" } });
    expect(s.answer).toBe("Hel" + "lo");
    s = reduce(s, { type: "event", askSeq: seq, event: { kind: "completed", answer: "Hello", citations: [], narrowed: false } });
    expect(s.phase).toBe("answered");
    expect(s.answer).toBe("Hello");
  });
  it("ignores events from a superseded ask", () => {
    let s = reduce(ready(), { type: "ask" });
    const old = s.askSeq;
    s = reduce(s, { type: "ask" });
    const ev: ChatEvent = { kind: "delta", text: "stale" };
    s = reduce(s, { type: "event", askSeq: old, event: ev });
    expect(s.answer).toBe("");
  });
  it("ignores a reply for another request id", () => {
    let s = reduce(ready(3), { type: "ask" });
    s = reduce(s, { type: "reply", askSeq: s.askSeq, reply: { kind: "destination_changed", requestId: 2, destination: { kind: "local_only" } } });
    expect(s.destination).toEqual({ kind: "local" });
    expect(s.phase).toBe("asking");
  });
  it("shows the new destination on DestinationChanged and needs Return again", () => {
    let s = reduce(ready(3), { type: "ask" });
    s = reduce(s, {
      type: "reply",
      askSeq: s.askSeq,
      reply: { kind: "destination_changed", requestId: 3, destination: { kind: "hosted", provider: "openai" } },
    });
    expect(s.phase).toBe("ready");
    expect(s.destination).toEqual({ kind: "hosted", provider: "openai" });
    expect(s.destinationChanged).toBe(true);
    s = reduce(s, { type: "ask" });
    expect(s.phase).toBe("asking");
  });
  it("shows a failure", () => {
    let s = reduce(ready(), { type: "ask" });
    s = reduce(s, {
      type: "event",
      askSeq: s.askSeq,
      event: { kind: "failed", partial: false, error: { kind: "store", message: "It broke." } },
    });
    expect(s.phase).toBe("failed");
    expect(s.error).toBe("It broke.");
  });
  it("a failed ask call shows its error", () => {
    let s = reduce(ready(), { type: "ask" });
    s = reduce(s, { type: "ask_error", askSeq: s.askSeq, message: "No." });
    expect(s.phase).toBe("failed");
    expect(s.error).toBe("No.");
  });
  it("a local-model-missing failure says so, with no key or Ollama hint", () => {
    const err = describeError({ kind: "provider_failure", reason: { kind: "local_model_missing" } });
    expect(err).toBe("No local model is installed.");
    let s = reduce(ready(), { type: "ask" });
    s = reduce(s, {
      type: "event",
      askSeq: s.askSeq,
      event: { kind: "failed", partial: false, error: { kind: "provider_failure", reason: { kind: "local_model_missing" } } },
    });
    expect(s.error).toBe("No local model is installed.");
    expect(s.error).not.toMatch(/key|Ollama/i);
  });
  it("an unchecked destination shows a neutral line, not 'no provider'", () => {
    const s = reduce(initialState, { type: "read", read: panelRead(3, { canAnswer: false, cannotAnswer: null }) });
    expect(s.phase).toBe("blocked");
    expect(s.blocked).toBe("Couldn't check the AI provider in time.");
  });
  it("an unknown hosted provider is named, never the local line", () => {
    expect(destinationText({ kind: "hosted", provider: "acme" })).toBe("The text above will be sent to acme");
  });
  it("a rejected read shows an error instead of reading forever", () => {
    const s = reduce(initialState, { type: "read_error", message: "boom" });
    expect(s.phase).toBe("failed");
    expect(s.error).toBe("boom");
  });
  it("a rejected ask shows the command error's sentence", () => {
    let s = reduce(ready(), { type: "ask" });
    s = reduce(s, { type: "ask_error", askSeq: s.askSeq, message: "x" });
    expect(s.phase).toBe("failed");
  });
});
