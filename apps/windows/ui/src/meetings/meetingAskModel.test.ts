import { describe, expect, it } from "vitest";
import type { ChatEvent } from "../generated/types";
import {
  ASK_PRESETS,
  applyMeetingEvent,
  disabledReason,
  historyOf,
  matchingPreset,
  newExchange,
  transcriptCitationIds,
  type MeetingExchange,
} from "./meetingAskModel";

const run = (exchange: MeetingExchange, events: ChatEvent[]) => events.reduce(applyMeetingEvent, exchange);

const answered = (question: string, answer: string, narrowed = false) =>
  run(newExchange(question), [
    { kind: "started", runId: "r", provider: "ollama", model: "m", context: [] },
    { kind: "delta", text: answer },
    { kind: "completed", answer, citations: [], conversationId: null, narrowed },
  ]);

describe("matchingPreset", () => {
  it("matches a label ignoring case, space and end punctuation", () => {
    expect(matchingPreset("  bring me up to speed. ")?.id).toBe("bringMeUpToSpeed");
    expect(matchingPreset("Did anyone ask me a question")?.id).toBe("anyoneAskMe");
    expect(matchingPreset("Make me sound smart!")?.id).toBe("makeMeSoundSmart");
    expect(matchingPreset("Make me sound smarter")).toBeNull();
    expect(matchingPreset("  ")).toBeNull();
  });
});

describe("newExchange", () => {
  it("sends a preset's prompt and shows its label", () => {
    const e = newExchange("bring me up to speed");
    expect(e.question).toBe("Bring me up to speed");
    expect(e.prompt).toBe(ASK_PRESETS[1].prompt);
  });

  it("sends a typed question as it is", () => {
    const e = newExchange("Who owns the budget?");
    expect([e.question, e.prompt]).toEqual(["Who owns the budget?", "Who owns the budget?"]);
  });
});

describe("transcriptCitationIds", () => {
  it("keeps transcript lines, first cited first, once each; live ids are negative", () => {
    expect(
      transcriptCitationIds([
        { kind: "transcript", id: -4 },
        { kind: "capture", id: 1 },
        { kind: "transcript", id: 9 },
        { kind: "transcript", id: -4 },
      ]),
    ).toEqual([-4, 9]);
  });
});

describe("disabledReason", () => {
  const after = { recording: false, liveLines: 0, processing: false, hasTranscript: true };
  it("waits for the first live lines during the call", () => {
    expect(disabledReason({ ...after, recording: true })).toMatch(/^Listening/);
    expect(disabledReason({ ...after, recording: true, liveLines: 2, hasTranscript: false })).toBeNull();
  });
  it("waits for transcription, and needs a transcript, after the call", () => {
    expect(disabledReason({ ...after, processing: true })).toMatch(/^Transcribing/);
    expect(disabledReason({ ...after, hasTranscript: false })).toMatch(/no transcript/);
    expect(disabledReason(after)).toBeNull();
  });
});

describe("applyMeetingEvent and historyOf", () => {
  it("keeps narrowed from the completed event", () => {
    expect(answered("q", "a", true).narrowed).toBe(true);
    expect(answered("q", "a").narrowed).toBe(false);
  });

  it("sends only answered exchanges as history, with what the model was asked", () => {
    const failed = run(newExchange("broken?"), [{ kind: "failed", error: { kind: "provider_failure", reason: null }, partial: false }]);
    const history = historyOf([answered("Bring me up to speed", "Here is where things are."), failed, answered("Who?", "Ana.")]);
    expect(history).toEqual([
      { query: ASK_PRESETS[1].prompt, answer: "Here is where things are." },
      { query: "Who?", answer: "Ana." },
    ]);
  });
});
