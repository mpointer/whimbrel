import { describe, expect, it, vi } from "vitest";
import type { DownloadView } from "../generated/types";

vi.mock("../api", () => ({}));
vi.mock("./turboDownload", () => ({ turboDownload: {} }));
vi.mock("./meetingModelsDownload", () => ({ meetingModelsDownload: {} }));
vi.mock("./searchModelDownload", () => ({ searchModelDownload: {} }));

const { footer, runSteps } = await import("./modelBootstrap");
type Step = Parameters<typeof runSteps>[0][number];

const job = (over: Partial<DownloadView>): DownloadView => ({
  jobId: "1",
  fraction: 0.4,
  status: "Downloading…",
  done: false,
  cancelled: false,
  ...over,
});

/** A step whose status says `missing`, and whose download ends with `end`. */
function step(missing: boolean | null, end: DownloadView | null = job({ done: true, fraction: 1 })) {
  const start = vi.fn(async () => end);
  const s: Step = {
    store: { refresh: async () => {}, get: () => ({ status: missing === null ? null : { missing }, job: null, error: null }), start },
    missing: (status: { missing: boolean }) => status.missing,
  };
  return { s, start };
}

describe("runSteps", () => {
  it("downloads only what's missing, meeting models before the search model", async () => {
    const meeting = step(true);
    const search = step(true);
    expect(await runSteps([meeting.s, search.s])).toBe("ready");
    expect(meeting.start).toHaveBeenCalledOnce();
    expect(search.start).toHaveBeenCalledOnce();
    expect(meeting.start.mock.invocationCallOrder[0]).toBeLessThan(search.start.mock.invocationCallOrder[0]);
  });

  it("runs a step's follow-up once its models are there, downloaded or not, and not after a failure", async () => {
    const after = vi.fn(async () => {});
    const present = { ...step(false).s, after };
    const downloaded = { ...step(true).s, after };
    const failed = { ...step(true, job({ done: true, error: "timed out" })).s, after };
    await runSteps([present, downloaded]);
    expect(after).toHaveBeenCalledTimes(2);
    await runSteps([failed]);
    expect(after).toHaveBeenCalledTimes(2);
  });

  it("starts nothing when everything is on disk", async () => {
    const meeting = step(false);
    const search = step(false);
    expect(await runSteps([meeting.s, search.s])).toBe("ready");
    expect(meeting.start).not.toHaveBeenCalled();
    expect(search.start).not.toHaveBeenCalled();
  });

  it("stops at a failure, so Try again starts from the first missing model", async () => {
    const meeting = step(true, job({ done: true, error: "timed out" }));
    const search = step(true);
    expect(await runSteps([meeting.s, search.s])).toBe("failed");
    expect(search.start).not.toHaveBeenCalled();
    // A command that failed (no job at all) is a failure too.
    expect(await runSteps([step(true, null).s])).toBe("failed");
  });

  it("stops quietly at a cancel from Settings, and when a status can't be read", async () => {
    const search = step(true);
    expect(await runSteps([step(true, job({ done: true, cancelled: true })).s, search.s])).toBe("cancelled");
    expect(await runSteps([step(null).s, search.s])).toBe("cancelled");
    expect(search.start).not.toHaveBeenCalled();
  });
});

describe("footer", () => {
  it("shows whichever download is running, wherever it was started", () => {
    expect(footer(job({ fraction: 0.2 }), null, false)).toEqual({ kind: "downloading", fraction: 0.2 });
    expect(footer(job({ done: true }), job({ fraction: 0.7 }), false)).toEqual({ kind: "downloading", fraction: 0.7 });
    expect(footer(null, job({}), true).kind).toBe("downloading");
  });

  it("shows a failure until Try again, and nothing otherwise", () => {
    expect(footer(job({ done: true, error: "timed out" }), null, true)).toEqual({ kind: "failed" });
    expect(footer(job({ done: true }), job({ done: true }), false)).toEqual({ kind: "hidden" });
    expect(footer(null, null, false)).toEqual({ kind: "hidden" });
  });
});
