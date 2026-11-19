import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { WActionItemRow, WConsentBanner, WRecordingIndicator, WSourceCard, WSourceMark, WNoteBlock, recordingName, spokenDuration, formatClock, WCaptureState, WSettingsRow, WSearchBar } from ".";

const html = (node: ReactNode) => renderToStaticMarkup(<>{node}</>);

/** Finds elements in an unrendered tree (hook-free components only). */
function find(node: ReactNode, test: (e: ReactElement<Record<string, unknown>>) => boolean, out: ReactElement<Record<string, unknown>>[] = []) {
  if (Array.isArray(node)) node.forEach((n) => find(n, test, out));
  else if (isValidElement(node)) {
    const el = node as ReactElement<Record<string, unknown>>;
    if (test(el)) out.push(el);
    find(el.props.children as ReactNode, test, out);
  }
  return out;
}

describe("recording indicator", () => {
  it("names each state, with the time spoken in full", () => {
    expect(recordingName("recording", 848)).toBe("Recording, 14 minutes 8 seconds");
    expect(recordingName("paused", 61)).toBe("Paused, 1 minute 1 second");
    expect(recordingName("paused", 3600)).toBe("Paused, 1 hour");
    expect(recordingName("processing")).toBe("Processing");
    expect(recordingName("off")).toBe("Not recording");
    expect(recordingName("problem", undefined, "No microphone found")).toBe("Problem, No microphone found");
  });

  it("puts that name on a polite live region so a change is announced", () => {
    const out = html(<WRecordingIndicator state="recording" elapsedSeconds={848} />);
    expect(out).toContain('role="status"');
    expect(out).toContain('aria-label="Recording, 14 minutes 8 seconds"');
  });

  it("draws the state as a shape, not only a color", () => {
    for (const state of ["recording", "paused", "processing", "off", "problem"] as const) {
      expect(html(<WRecordingIndicator state={state} size="pill" />)).toContain("<svg");
    }
    expect(html(<WRecordingIndicator state="recording" />)).toContain("Recording");
  });

  it("formats the clock and the spoken duration", () => {
    expect(formatClock(848)).toBe("14:08");
    expect(formatClock(3723)).toBe("1:02:03");
    expect(spokenDuration(0)).toBe("0 seconds");
  });
});

describe("consent banner", () => {
  it("says what to do and shows no Copied before a copy", () => {
    const out = html(<WConsentBanner onCopy={() => {}} onDone={() => {}} />);
    expect(out).toContain("You are recording. Tell the others before you go on.");
    expect(out).toContain("Copy notice");
    expect(out).toContain("Done");
    expect(out).not.toContain(">Copied<");
  });

  it("shows Copied after a copy", () => {
    expect(html(<WConsentBanner onCopy={() => {}} onDone={() => {}} copied />)).toContain(">Copied<");
  });

  it("never claims anyone was told or agreed", () => {
    for (const copied of [false, true]) {
      const text = html(<WConsentBanner onCopy={() => {}} onDone={() => {}} copied={copied} />).toLowerCase();
      for (const word of ["notified", "consented", "agreed", "informed", "approved", "told them"]) expect(text).not.toContain(word);
    }
  });
});

describe("action items", () => {
  it("draws every control at rest: checkbox, owner, due date and the source time", () => {
    const out = html(<WActionItemRow text="Send the quote." owner="Sam Okafor" due="Due Thu, Nov 19" sourceTime="28:50" done={false} onToggle={() => {}} onOpenSource={() => {}} />);
    expect(out).toContain('type="checkbox"');
    expect(out).toContain("Sam Okafor");
    expect(out).toContain("Due Thu, Nov 19");
    expect(out).toContain('aria-label="Open source at 28:50"');
    expect(out).not.toMatch(/ hidden[ >=]|display:\s*none|opacity:\s*0/);
  });

  it("names the checkbox by its text and marks done", () => {
    const out = html(<WActionItemRow text="Send the quote." done onToggle={() => {}} />);
    expect(out).toContain('aria-label="Send the quote."');
    expect(out).toContain("w-action-done");
    expect(out).toContain("checked");
  });

  it("reports the toggle", () => {
    const onToggle = vi.fn();
    const row = WActionItemRow({ text: "x", done: false, onToggle });
    const box = find(row, (e) => e.props.type === "checkbox")[0];
    (box.props.onChange as (e: unknown) => void)({ target: { checked: true } });
    expect(onToggle).toHaveBeenCalledWith(true);
  });
});

describe("source marks", () => {
  it("selects its own source when pressed", () => {
    const onSelect = vi.fn();
    const mark = WSourceMark({ number: 3, onSelect });
    expect(mark.type).toBe("button");
    (mark.props.onClick as () => void)();
    expect(onSelect).toHaveBeenCalledWith(3);
  });

  it("is a named toggle button that shows which one is selected", () => {
    const out = html(<WSourceMark number={2} selected onSelect={() => {}} />);
    expect(out).toContain('aria-label="Source 2"');
    expect(out).toContain('aria-pressed="true"');
  });

  it("carries the right numbers from a note line to the source cards", () => {
    const out = html(<WNoteBlock kind="paragraph" author="whimbrel" text="Line." sources={[4, 5]} selectedSource={5} onSelectSource={() => {}} />);
    expect(out).toContain('aria-label="Source 4"');
    expect(out).toContain('aria-label="Source 5"');
    expect(out).toContain("w-note-ai");
    expect(html(<WNoteBlock kind="paragraph" author="you" text="Mine." sources={[1]} onSelectSource={() => {}} />)).not.toContain("w-mark");
  });

  it("gives the source card an Open button with the caller's words", () => {
    const onOpen = vi.fn();
    const card = WSourceCard({ number: 1, appName: "Excel", appIcon: "table", kind: "Seen on screen", time: "09:41", openLabel: "Open in Excel", onOpen });
    const open = find(card, (e) => e.props.className === "w-source-open")[0];
    (open.props.onClick as () => void)();
    expect(onOpen).toHaveBeenCalled();
    expect(html(card)).toContain("Open in Excel");
  });
});

describe("other controls", () => {
  it("opens the pause menu only when asked, and lists the choices", () => {
    const base = { state: "on" as const, choices: [{ id: "15", label: "Pause for 15 minutes" }], onChoose: () => {}, onWhatIsCaptured: () => {} };
    expect(html(<WCaptureState {...base} open={false} />)).not.toContain('role="menu"');
    const open = html(<WCaptureState {...base} open />);
    expect(open).toContain("Pause for 15 minutes");
    expect(open).toContain("What is captured");
    expect(html(<WCaptureState {...base} state="paused" pausedUntil="10:52 AM" open={false} />)).toContain("Paused until 10:52 AM");
  });

  it("gives a toggle row the switch role with its state", () => {
    expect(html(<WSettingsRow variant="toggle" label="Work memory" checked onToggle={() => {}} />)).toMatch(/role="switch"[^>]*aria-checked="true"/);
  });

  it("clears a search from a named button", () => {
    expect(html(<WSearchBar value="abc" onChange={() => {}} />)).toContain('aria-label="Clear search"');
  });
});
