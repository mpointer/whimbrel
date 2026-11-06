import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { TranscriptTurn } from "../generated/types";
import { shownSpeaker, SpeakerMenu, speakerMenuItems } from "./SpeakerMenu";

const turn = (over: Partial<TranscriptTurn> = {}): TranscriptTurn => ({
  speaker: "Speaker 2",
  label: "Speaker 2",
  guess: false,
  speakerId: 1,
  personKey: null,
  timestamp: "10:00:01",
  text: "Hello.",
  ...over,
});
const guess = { label: "Ana Ruiz", guess: true, personKey: "ana@x.test" };
const invitees = [
  { name: "Ana Ruiz", email: "ana@x.test" },
  { name: "Sam", email: null },
];
const none = () => {};

describe("speaker names", () => {
  it("shows a guess with a question mark and an unnamed speaker as its label", () => {
    expect(shownSpeaker(turn(guess))).toBe("Ana Ruiz?");
    expect(shownSpeaker(turn({ ...guess, guess: false }))).toBe("Ana Ruiz");
    expect(shownSpeaker(turn())).toBe("Speaker 2");
    expect(shownSpeaker(turn({ speaker: "Me", label: "Me", speakerId: null }))).toBe("Me");
  });

  it("lists the invitees and Someone else, with Yes first only for a guess", () => {
    expect(speakerMenuItems(turn(), invitees).map((i) => i.kind)).toEqual(["invitee", "invitee", "someoneElse"]);
    const guessed = speakerMenuItems(turn(guess), invitees);
    expect(guessed[0]).toEqual({ kind: "confirm", name: "Ana Ruiz", personKey: "ana@x.test" });
    expect(guessed).toHaveLength(4);
    expect(speakerMenuItems(turn({ ...guess, guess: false }), invitees).some((i) => i.kind === "confirm")).toBe(false);
    expect(speakerMenuItems(turn(), [])).toEqual([{ kind: "someoneElse" }]);
  });

  it("is plain text when naming isn't offered, and a button in the weaker style for a guess", () => {
    const plain = renderToStaticMarkup(<SpeakerMenu turn={turn()} invitees={invitees} offered={false} onConfirm={none} onPick={none} />);
    expect(plain).not.toContain("<button");
    expect(plain).toContain("Speaker 2");
    const offered = renderToStaticMarkup(<SpeakerMenu turn={turn(guess)} invitees={invitees} offered={true} onConfirm={none} onPick={none} />);
    expect(offered).toContain("<button");
    expect(offered).toContain("transcript-speaker--guess");
    expect(offered).toContain("Ana Ruiz?");
    expect(offered).not.toContain('role="menu"');
  });
});
