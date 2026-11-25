// A start in flight (item 7 of the restyle pass): the bar says "Starting recording" and Stop is offered and enabled.

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { RecordingBar } from "./RecordingBar";
import type { RecordingControls } from "../../shell/redesign/useRecording";

const rec = (over: Partial<RecordingControls> = {}): RecordingControls => ({
  view: { state: "starting", meetingId: null, startedUnixMs: null, degraded: null, failure: null, processing: null, paused: false, resuming: false, pausedTotalMs: 0, pausedSinceUnixMs: null } as unknown as RecordingControls["view"],
  failed: false,
  readAt: 0,
  state: "starting",
  now: 0,
  busy: false,
  stop() {},
  pause() {},
  resume() {},
  reload() {},
  ...over,
});

describe("the recording bar while a recording starts", () => {
  it("says Starting recording and names the state that way", () => {
    const out = renderToStaticMarkup(<RecordingBar rec={rec()} />);
    expect(out).toContain("Starting recording");
    expect(out).toContain('aria-label="Starting recording"');
  });

  it("offers Stop, enabled, and keeps Pause disabled", () => {
    const out = renderToStaticMarkup(<RecordingBar rec={rec()} />);
    const buttons = [...out.matchAll(/<button[^>]*>/g)].map((m) => m[0]);
    const stop = buttons.find((b) => b.includes("w-button-primary"));
    expect(stop, out).toBeDefined();
    expect(stop).not.toContain("disabled");
    expect(buttons.find((b) => !b.includes("w-button-primary"))).toContain("disabled");
  });

  it("holds Stop while a command runs", () => {
    const out = renderToStaticMarkup(<RecordingBar rec={rec({ busy: true })} />);
    expect([...out.matchAll(/<button[^>]*>/g)].map((m) => m[0]).find((b) => b.includes("w-button-primary"))).toContain("disabled");
  });

  it("leaves the other states' words alone", () => {
    expect(renderToStaticMarkup(<RecordingBar rec={rec({ state: "resuming" })} />)).not.toContain("Starting recording");
  });
});
