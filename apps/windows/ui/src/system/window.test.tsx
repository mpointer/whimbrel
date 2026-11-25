import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WINDOW_KEYS, WMemoryControl, WMenuSection, WRecordingRow, WWindowFrame, arrowFocusIndex, arrowKeyAction, WINDOW_DESIGN, isCompactSpace, mainActionOnReturn, memoryControls, windowKeyAction, windowDescription, windowDocumentTitle, type SmallWindowKind } from ".";

const choices = [
  { id: "15m", label: "15 minutes" },
  { id: "1h", label: "1 hour" },
  { id: "tomorrow", label: "Until tomorrow" },
];
const noop = () => {};

describe("the small windows' keys", () => {
  it("Return does the main action and Escape dismisses, from wherever focus is not a control", () => {
    expect(windowKeyAction({ key: "Enter", target: { tag: "body" } })).toBe("primary");
    expect(windowKeyAction({ key: "Enter", target: { tag: "div" } })).toBe("primary");
    expect(windowKeyAction({ key: "Escape", target: { tag: "body" } })).toBe("dismiss");
    expect(windowKeyAction({ key: "Escape", target: { tag: "button" } })).toBe("dismiss");
  });

  it("Return on a focused button, link, checkbox, select or menu item presses that control and never the main action as well", () => {
    for (const target of [{ tag: "button" }, { tag: "a" }, { tag: "select" }, { tag: "input", type: "checkbox" }, { tag: "div", role: "menuitem" }, { tag: "div", role: "switch" }]) {
      expect(windowKeyAction({ key: "Enter", target }), JSON.stringify(target)).toBeNull();
    }
  });

  it("does nothing while an input method is open or another handler took the key", () => {
    expect(windowKeyAction({ key: "Enter", composing: true, target: { tag: "input" } })).toBeNull();
    expect(windowKeyAction({ key: "Escape", composing: true, target: { tag: "input" } })).toBeNull();
    expect(windowKeyAction({ key: "Escape", defaultPrevented: true, target: { tag: "body" } })).toBeNull();
  });

  it("ignores every other key", () => {
    expect(windowKeyAction({ key: "a", target: { tag: "body" } })).toBeNull();
    expect(windowKeyAction({ key: " ", target: { tag: "body" } })).toBeNull();
  });
});

describe("Return in each small window (WINDOW_KEYS)", () => {
  const run = (kind: SmallWindowKind) => {
    let ran = 0;
    const handler = mainActionOnReturn(kind, () => void ran++);
    handler?.();
    return { handler, ran };
  };

  it("never starts, resumes or stops a recording from a window that appears by itself", () => {
    for (const kind of ["meetingAlert", "stillHerePaused"] as const) {
      expect(WINDOW_KEYS[kind].returnRunsMainAction, kind).toBe(false);
      expect(run(kind).handler, kind).toBeUndefined();
    }
  });

  it("runs the main action on Return only where it is safe: open, raise the budget, free space, keep recording", () => {
    for (const kind of ["question", "budget", "disk", "stillHere", "popup"] as const) {
      expect(WINDOW_KEYS[kind].returnRunsMainAction, kind).toBe(true);
      expect(run(kind).ran, kind).toBe(1);
    }
  });

  it("lists every window, with its Escape answer", () => {
    expect(Object.keys(WINDOW_KEYS).sort()).toEqual(["budget", "disk", "meetingAlert", "popup", "question", "stillHere", "stillHerePaused"]);
    expect(WINDOW_KEYS.meetingAlert.escape).toBe("Not now");
    expect(WINDOW_KEYS.stillHerePaused.escape).toBe("Keep as now");
    expect(WINDOW_KEYS.stillHere.escape).toBe("Keep recording");
  });

  it("a frame with no Return handler adds no Return shortcut: the key reaches only the focused control", () => {
    // `useWindowKeys` calls preventDefault only when a handler exists, so a focused button keeps its own Return.
    expect(windowKeyAction({ key: "Enter", target: { tag: "body" } })).toBe("primary");
    expect(mainActionOnReturn("meetingAlert", () => {})).toBeUndefined();
  });
});

describe("the window frame's pinned sentence", () => {
  it("is drawn under the title, outside the scrolling lines, and the product name is always drawn", () => {
    const html = renderToStaticMarkup(
      <WWindowFrame title="Meeting detected" lines={["Planning"]} pinned="Record it? Audio stays on this PC." actions={<button>Record</button>} />,
    );
    const pinned = html.indexOf("Record it? Audio stays on this PC.");
    expect(pinned).toBeGreaterThan(html.indexOf("Meeting detected"));
    expect(pinned).toBeLessThan(html.indexOf("w-win-scrollwrap"));
    expect(html).toContain("w-win-pinned");
    expect(html).toContain("Whimbrel");
    expect(html).not.toContain("w-visually-hidden");
  });
});

describe("compact layout", () => {
  const design = WINDOW_DESIGN.alert;
  it("is chosen from the CSS space the window has, not from the root font size", () => {
    // The shell opens a window at its design size times the Windows text scale, so it has the full design space.
    expect(isCompactSpace({ width: 360, height: 230 }, design)).toBe(false);
    // A window capped to the screen at 200% text has half of it (a quarter of the area).
    expect(isCompactSpace({ width: 180, height: 115 }, design)).toBe(true);
  });

  it("is compact when either side is short, and tolerates the rounding of a physical size", () => {
    expect(isCompactSpace({ width: 360, height: 150 }, design)).toBe(true);
    expect(isCompactSpace({ width: 300, height: 230 }, design)).toBe(true);
    expect(isCompactSpace({ width: 359.4, height: 229.6 }, design)).toBe(false);
    expect(isCompactSpace({ width: 400, height: 300 }, design)).toBe(false);
  });

  it("is never compact for a window with no design size", () => {
    expect(isCompactSpace({ width: 10, height: 10 })).toBe(false);
  });

  it("knows the design size of every small window", () => {
    expect(Object.keys(WINDOW_DESIGN).sort()).toEqual(["alert", "budget", "disk", "hotkeyPanel", "meetingBar", "popup", "question", "stillHere"]);
  });
});

describe("arrow keys between controls", () => {
  it("moves on and back, wrapping at both ends", () => {
    expect(arrowFocusIndex(4, 0, "ArrowDown")).toBe(1);
    expect(arrowFocusIndex(4, 3, "ArrowDown")).toBe(0);
    expect(arrowFocusIndex(4, 0, "ArrowUp")).toBe(3);
    expect(arrowFocusIndex(4, 2, "ArrowLeft")).toBe(1);
    expect(arrowFocusIndex(4, 2, "ArrowRight")).toBe(3);
  });

  it("starts from the first or last control when nothing is focused, and Home and End jump", () => {
    expect(arrowFocusIndex(3, -1, "ArrowDown")).toBe(0);
    expect(arrowFocusIndex(3, -1, "ArrowUp")).toBe(2);
    expect(arrowFocusIndex(3, 1, "Home")).toBe(0);
    expect(arrowFocusIndex(3, 1, "End")).toBe(2);
  });

  it("lets a focused scrolling body scroll: Up, Down, Page Up, Page Down, Home and End are the browser's, Tab leaves", () => {
    for (const key of ["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"]) expect(arrowKeyAction(true, key), key).toBe("scroll");
    // Left and Right do not scroll a column: they still move between controls. Tab is never ours.
    expect(arrowKeyAction(true, "ArrowLeft")).toBe("move");
    expect(arrowKeyAction(true, "ArrowRight")).toBe("move");
    expect(arrowKeyAction(true, "Tab")).toBeNull();
    expect(arrowKeyAction(true, "Enter")).toBeNull();
    // Anywhere else the arrows, Home and End move focus as before; Page Up and Page Down are not ours.
    for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]) expect(arrowKeyAction(false, key), key).toBe("move");
    expect(arrowKeyAction(false, "PageDown")).toBeNull();
    expect(arrowKeyAction(false, "Tab")).toBeNull();
  });

  it("leaves other keys and an empty window alone", () => {
    expect(arrowFocusIndex(3, 1, "Tab")).toBeNull();
    expect(arrowFocusIndex(3, 1, "a")).toBeNull();
    expect(arrowFocusIndex(0, -1, "ArrowDown")).toBeNull();
  });
});

describe("the window frame", () => {
  const html = renderToStaticMarkup(
    <WWindowFrame
      title="Meeting detected"
      lines={["Northwind security review", "Audio stays on this PC."]}
      countdown={<span>Recording starts in 8 s</span>}
      actions={
        <>
          <button>Record</button>
          <button>Not now</button>
        </>
      }
    />,
  );

  it("is the focus target: a named dialog that can take focus from code but is not in the Tab order", () => {
    expect(html).toMatch(/<main[^>]*role="alertdialog"/);
    expect(html).toMatch(/<main[^>]*tabindex="-1"/);
    const labelled = /aria-labelledby="([^"]+)"/.exec(html)?.[1];
    const described = /aria-describedby="([^"]+)"/.exec(html)?.[1];
    expect(labelled).toBeTruthy();
    expect(described).toBeTruthy();
    expect(html).toContain(`<h1 class="w-win-title" id="${labelled}"`);
    // The description is hidden (never read a second time in the page) and holds the lines, not the countdown.
    expect(html).toContain(`<span id="${described}" hidden="">Northwind security review. Audio stays on this PC.</span>`);
    const description = /<span id="[^"]+" hidden="">([^<]*)<\/span>/.exec(html)?.[1] ?? "";
    expect(description).not.toContain("Recording starts");
    expect(description).not.toContain("8 s");
  });

  it("names the window by its title, with the app mark and name above it", () => {
    expect(html).toContain("<h1");
    expect(html).toContain("Meeting detected");
    expect(html).toContain("Whimbrel");
    expect(html).toContain("<svg");
    expect(html).toMatch(/aria-labelledby="[^"]+"/);
  });

  it("puts the lines, then the countdown, then the actions, main action first", () => {
    const at = (s: string) => html.indexOf(s);
    expect(at("Northwind security review")).toBeGreaterThan(at("Meeting detected"));
    expect(at("Recording starts in 8 s")).toBeGreaterThan(at("Audio stays on this PC."));
    expect(at("Record</button>")).toBeGreaterThan(at("Recording starts in 8 s"));
    expect(at("Record</button>")).toBeLessThan(at("Not now"));
  });

  it("does not announce the countdown's seconds: only the lines are a status", () => {
    const status = html.match(/role="status"[^>]*>/g) ?? [];
    expect(status).toHaveLength(1);
    expect(html.slice(html.indexOf("w-win-countdown") - 20, html.indexOf("w-win-countdown"))).not.toContain("role");
  });

  it("leaves out the lines and the countdown when there are none", () => {
    const bare = renderToStaticMarkup(<WWindowFrame title="T" actions={<button>OK</button>} />);
    expect(bare).not.toContain("w-win-lines");
    expect(bare).not.toContain("w-win-countdown");
  });
});

describe("the recording row", () => {
  it("says the state and the recorded time, and the problem in words", () => {
    const html = renderToStaticMarkup(<WRecordingRow state="problem" elapsedSeconds={848} reason="The microphone isn't delivering audio." actions={<button>Stop</button>} />);
    expect(html).toContain("Problem");
    expect(html).toContain("14:08");
    expect(html).toContain("The microphone isn&#x27;t delivering audio.");
    expect(html).toContain("<svg");
    expect(html).toContain("Stop");
  });

  it("says a problem in addition to the recording state when asked: word, time, then the problem line", () => {
    const html = renderToStaticMarkup(<WRecordingRow state="paused" word="Recording paused" problem elapsedSeconds={728} reason="The microphone isn't delivering audio." actions={<button>Resume</button>} />);
    expect(html).toContain("Recording paused");
    expect(html).toContain("12:08");
    expect(html).toContain("Recording problem");
    expect(html).toContain("The microphone isn&#x27;t delivering audio.");
    expect(html.indexOf("Recording paused")).toBeLessThan(html.indexOf("Recording problem"));
    // The reason is its own line under the label, not text run on after it.
    expect(html).toContain("</strong><span class=\"w-recrow-problem-reason\">The microphone isn&#x27;t delivering audio.</span>");
    expect(html).not.toContain("Recording problem The");
  });

  it("shows a reason only for a problem", () => {
    const html = renderToStaticMarkup(<WRecordingRow state="recording" elapsedSeconds={5} reason="x" />);
    expect(html).not.toContain("w-recrow-reason");
  });
});

describe("the memory control", () => {
  const draw = (state: "on" | "paused" | "pausedByApp" | "off" | "stopped", pausedUntil?: string) =>
    renderToStaticMarkup(<WMemoryControl state={state} pausedUntil={pausedUntil} choices={choices} onPause={noop} onResumeNow={noop} onTurnOff={noop} onTurnOn={noop} onWhatIsCaptured={noop} />);

  it("offers what each state allows", () => {
    expect(memoryControls("on")).toEqual({ pause: true, resumeNow: false, turnOff: true, turnOn: false, openSettings: false });
    expect(memoryControls("paused")).toEqual({ pause: false, resumeNow: true, turnOff: true, turnOn: false, openSettings: false });
    expect(memoryControls("off")).toEqual({ pause: false, resumeNow: false, turnOff: false, turnOn: true, openSettings: false });
    expect(memoryControls("stopped")).toEqual({ pause: false, resumeNow: false, turnOff: false, turnOn: false, openSettings: true });
  });

  it("on: the words, the three choices as buttons in a named group, turn off", () => {
    const html = draw("on");
    expect(html).toContain("Memory on");
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Pause memory"');
    for (const c of choices) expect(html).toContain(`>${c.label}</span>`);
    expect(html).toContain("Turn off memory");
    expect(html).not.toContain("Resume now");
    expect(html).not.toContain("Turn on memory");
  });

  it("paused: says until when, with Resume now and no choices", () => {
    const html = draw("paused", "3:10 PM");
    expect(html).toContain("Paused until 3:10 PM");
    expect(html).toContain("Resume now");
    expect(html).not.toContain("Pause memory");
    expect(html).not.toContain("Until tomorrow");
  });

  it("off: says so, with turn on only", () => {
    const html = draw("off");
    expect(html).toContain("Memory off");
    expect(html).toContain("Turn on memory");
    expect(html).not.toContain("Turn off memory");
  });

  it("stopped: says so and offers nothing to change, except where to look", () => {
    const html = draw("stopped");
    expect(html).toContain("Memory stopped");
    expect(html).not.toContain("Turn on memory");
    expect(html).not.toContain("Turn off memory");
    expect(memoryControls("stopped").openSettings).toBe(true);
    const withSettings = renderToStaticMarkup(
      <WMemoryControl state="stopped" notes={["Whimbrel could not start memory."]} onOpenSettings={noop} choices={choices} onPause={noop} onResumeNow={noop} onTurnOff={noop} onTurnOn={noop} onWhatIsCaptured={noop} />,
    );
    expect(withSettings).toContain("Open Settings");
    expect(withSettings).toContain("Whimbrel could not start memory.");
  });

  it("paused by the app: says so, offers no timed choices and no Resume now, and can be turned off", () => {
    expect(memoryControls("pausedByApp")).toEqual({ pause: false, resumeNow: false, turnOff: true, turnOn: false, openSettings: false });
    expect(draw("pausedByApp")).toContain("Memory paused");
  });

  it("Turn off memory is a button with a border, as Turn on memory is, not plain text", () => {
    const html = draw("on");
    expect(html).toMatch(/class="w-button w-button-secondary"><span>Turn off memory/);
    expect(html).not.toMatch(/w-button-quiet[^>]*><span>Turn off memory/);
    expect(draw("off")).toMatch(/class="w-button w-button-secondary"><span>Turn on memory/);
  });

  it("can draw the state line and the rest apart, for a popup that pins one and scrolls the other", () => {
    const state = renderToStaticMarkup(<WMemoryControl state="on" choices={choices} onPause={noop} onResumeNow={noop} onTurnOff={noop} onTurnOn={noop} onWhatIsCaptured={noop} part="state" />);
    const details = renderToStaticMarkup(<WMemoryControl state="on" choices={choices} onPause={noop} onResumeNow={noop} onTurnOff={noop} onTurnOn={noop} onWhatIsCaptured={noop} part="details" />);
    expect(state).toContain("Memory on");
    expect(state).not.toContain("What is captured");
    expect(details).toContain("What is captured");
    expect(details).toContain("Pause memory");
    expect(details).not.toContain("Memory on");
  });

  it("always offers What is captured, and draws the state as a shape beside the words", () => {
    for (const s of ["on", "paused", "off", "stopped"] as const) {
      const html = draw(s, "3:10 PM");
      expect(html).toContain("What is captured");
      expect(html).toContain("<svg");
    }
  });
});

describe("a menu section", () => {
  it("is named by its heading, which can be hidden from the eye but not from a screen reader", () => {
    expect(renderToStaticMarkup(<WMenuSection label="Memory">x</WMenuSection>)).toContain('class="w-menu-label"');
    expect(renderToStaticMarkup(<WMenuSection label="Recording" labelHidden>x</WMenuSection>)).toContain('class="w-visually-hidden"');
  });
});

describe("what a small window is called and described", () => {
  it("names the page by the window's title, then the product, never the product name alone", () => {
    expect(windowDocumentTitle("Still in a meeting?")).toBe("Still in a meeting? - Whimbrel");
    expect(windowDocumentTitle("  Raise budget ")).toBe("Raise budget - Whimbrel");
    expect(windowDocumentTitle("")).toBe("Whimbrel");
  });

  it("describes with the pinned sentence and the lines in order, each as its own sentence", () => {
    expect(windowDescription(["No one has spoken for 15 minutes."])).toBe("No one has spoken for 15 minutes.");
    expect(windowDescription(["Record it? Audio stays on this PC.", "Northwind security review", "Zoom"])).toBe("Record it? Audio stays on this PC. Northwind security review. Zoom.");
  });

  it("leaves out empty and non-text parts and tidies spacing", () => {
    expect(windowDescription([null, undefined, "  ", "Free  space\n to keep recording"])).toBe("Free space to keep recording.");
    expect(windowDescription([])).toBe("");
    expect(windowDescription(undefined)).toBe("");
  });

  it("has no description element when there is nothing to describe", () => {
    const bare = renderToStaticMarkup(<WWindowFrame title="Meeting detected" actions={<button>Ok</button>} />);
    expect(bare).not.toContain("aria-describedby");
  });
});
