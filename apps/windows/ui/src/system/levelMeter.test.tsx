import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WLevelMeter, WRecordingIndicator, recordingName, recordingWord } from ".";

describe("level meter", () => {
  it("is decoration with a name that says whether audio is arriving", () => {
    expect(renderToStaticMarkup(<WLevelMeter level={0.4} arriving />)).toContain('aria-label="Audio is arriving"');
    expect(renderToStaticMarkup(<WLevelMeter level={0} arriving={false} />)).toContain('aria-label="No audio right now"');
    expect(renderToStaticMarkup(<WLevelMeter level={0.4} arriving idle />)).toContain('aria-label="No audio right now"');
  });
  it("draws the bars as plain elements, not announced as they move", () => {
    const out = renderToStaticMarkup(<WLevelMeter level={0.4} arriving bars={6} />);
    expect(out).toContain('role="img"');
    expect(out).not.toContain("aria-live");
    expect((out.match(/w-meter-bar/g) ?? []).length).toBe(6);
  });
});

describe("the new states of the recording indicator", () => {
  it("says Starting and Resuming, with no time", () => {
    expect(recordingWord("starting")).toBe("Starting");
    expect(recordingWord("resuming")).toBe("Resuming");
    expect(recordingName("resuming", 700)).toBe("Resuming");
  });
  it("keeps the time in a problem and names the reason", () => {
    expect(recordingName("problem", 848, "Some audio may be missing")).toBe("Problem, Some audio may be missing, 14 minutes 8 seconds");
  });
  it("draws the meter and the buttons only in the bar size", () => {
    const meter = <WLevelMeter level={0.2} arriving />;
    expect(renderToStaticMarkup(<WRecordingIndicator state="recording" meter={meter} actions={<button>Stop</button>} />)).toContain("w-rec-meter");
    expect(renderToStaticMarkup(<WRecordingIndicator state="recording" size="pill" meter={meter} actions={<button>Stop</button>} />)).not.toContain("w-rec-meter");
  });
});
