import { describe, expect, it } from "vitest";
import type { ChatEvent, ContextEntry } from "../generated/types";
import { applyEvent, citedEntries, isBusy, newTurn, promptToSend, savedTurn, spendLine, statusLine, type Turn } from "./chatModel";

const run = (events: ChatEvent[], start: Turn = newTurn("what did I plan?")) => events.reduce(applyEvent, start);

const capture: ContextEntry = { item: { kind: "capture", id: 4 }, label: "Plan · notepad.exe", tsUnixMs: 1, open: { kind: "capture", id: 4 } };
const transcript: ContextEntry = { item: { kind: "transcript", id: 9 }, label: 'Meeting "Sync", Speaker 1', open: { kind: "meeting", id: 2 } };
const thread: ContextEntry = { item: { kind: "thread", id: 4 }, label: "Re: plan", open: null };

describe("applyEvent", () => {
  it("streams text, then completes with citations", () => {
    const streaming = run([
      { kind: "started", runId: "run-1", provider: "ollama", model: "gemma4:12b", context: [capture] },
      { kind: "delta", text: "The falcon " },
      { kind: "delta", text: "migration [1]." },
    ]);
    expect(streaming.status).toBe("streaming");
    expect(streaming.text).toBe("The falcon migration [1].");
    expect(isBusy(streaming)).toBe(true);

    const done = applyEvent(streaming, { kind: "completed", answer: "The falcon migration [1].", citations: [{ kind: "capture", id: 4 }], conversationId: 3 });
    expect(done.status).toBe("done");
    expect(done.runId).toBeNull();
    expect(citedEntries(done.citations, done.context)).toEqual([capture]);
    expect(isBusy(done)).toBe(false);
  });

  it("shows a spinner on the blocking path and starts over after a fallback", () => {
    const blocking = run([{ kind: "started", provider: "anthropic", model: "claude-haiku-4-5", context: [] }]);
    expect(blocking.status).toBe("blocking");
    expect(statusLine(blocking)).toBe("Asking claude-haiku-4-5…");

    const fellBack = run([
      { kind: "started", runId: "run-2", provider: "ollama", model: "gemma4:12b", context: [] },
      { kind: "fell_back" },
    ]);
    // The failed stream's model must not be credited with the fallback's answer.
    expect(fellBack).toMatchObject({ status: "blocking", runId: null, text: "", provider: null, model: null });
    expect(statusLine(fellBack)).toBe("Asking the model…");
  });

  it("keeps streamed text after a partial failure and drops it otherwise", () => {
    const started: ChatEvent = { kind: "started", runId: "run-3", provider: "google", model: "gemini-2.5-flash", context: [] };
    const partial = run([started, { kind: "delta", text: "So far" }, { kind: "failed", error: { kind: "provider_failure" }, partial: true }]);
    expect(partial).toMatchObject({ status: "failed", text: "So far" });
    expect(partial.error).toMatch(/^No model provider could answer/);

    const capped = run([{ kind: "failed", error: { kind: "spend_cap", scope: "user", spent_cents: 205, cap_cents: 200 }, partial: false }]);
    expect(capped.error).toBe("You've reached today's spend cap (205¢ of 200¢). It resets at midnight UTC, or raise it in Settings.");
    expect(capped.text).toBe("");
  });

  it("marks a stop as cancelled and ignores titles", () => {
    const stopped = run([{ kind: "started", runId: "run-4", provider: "ollama", model: "m", context: [] }, { kind: "cancelled" }]);
    expect(stopped).toMatchObject({ status: "cancelled", runId: null });
    const turn = newTurn("q");
    expect(applyEvent(turn, { kind: "titled", conversationId: 1, title: "Falcon plan" })).toBe(turn);
  });
});

describe("citedEntries", () => {
  it("matches kind and id, keeps citation order, and drops duplicates and unknowns", () => {
    const context = [capture, transcript, thread];
    expect(
      citedEntries(
        [
          { kind: "transcript", id: 9 },
          { kind: "thread", id: 4 },
          { kind: "transcript", id: 9 },
          { kind: "episode", id: 4 },
        ],
        context,
      ),
    ).toEqual([transcript, thread]);
  });
});

describe("spendLine and statusLine", () => {
  it("formats spend against the cap, or without one", () => {
    expect(spendLine({ userCents: 12, capCents: 200 })).toBe("12¢ / 200¢ today");
    expect(spendLine({ userCents: 12, capCents: null })).toBe("12¢ today · no cap");
  });

  it("says what is happening before text arrives", () => {
    expect(statusLine(newTurn("q"))).toBe("Finding context…");
    const connecting = run([{ kind: "started", runId: "r", provider: "ollama", model: "gemma4:12b", context: [] }]);
    expect(statusLine(connecting)).toBe("Connecting to gemma4:12b…");
    expect(statusLine(applyEvent(connecting, { kind: "progress", label: "Loading model" }))).toBe("Loading model");
  });
});

describe("savedTurn", () => {
  it("brings back a saved answer's citations and model", () => {
    const entry = { item: { kind: "episode", id: 7 }, label: "Figma", tsUnixMs: 1, open: { kind: "episode", id: 7 } } as const;
    const turn = savedTurn({ query: "q", answer: "a", createdAtUnixMs: 0, citations: [entry], model: "llama3" });
    expect(citedEntries(turn.citations, turn.context)).toEqual([entry]);
    expect(turn.model).toBe("llama3");
    expect(turn.status).toBe("done");
  });
});

describe("promptToSend", () => {
  it("sends a pending request's trimmed prompt once", () => {
    const request = { id: 3, prompt: "  what changed this week?  " };
    expect(promptToSend(request, null)).toBe("what changed this week?");
    expect(promptToSend(request, 2)).toBe("what changed this week?");
    expect(promptToSend(request, 3)).toBeNull();
  });

  it("sends a newer request even after an older one was handled", () => {
    expect(promptToSend({ id: 4, prompt: "again" }, 3)).toBe("again");
  });

  it("sends nothing for no request or a blank prompt", () => {
    expect(promptToSend(null, null)).toBeNull();
    expect(promptToSend(undefined, null)).toBeNull();
    expect(promptToSend({ id: 1, prompt: "   " }, null)).toBeNull();
  });
});
