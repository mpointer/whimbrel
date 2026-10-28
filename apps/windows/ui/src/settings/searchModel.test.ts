import { describe, expect, it } from "vitest";
import type { DownloadView, ModelStatus } from "../generated/types";
import { percent, searchModelState } from "./searchModel";

const missing: ModelStatus = { installed: false, loaded: false, dir: "C:\\k\\models\\minilm" };
const job = (over: Partial<DownloadView>): DownloadView => ({
  jobId: "models-1-0",
  fraction: 0.4,
  status: "Downloading Search model weights…",
  done: false,
  cancelled: false,
  ...over,
});

describe("searchModelState", () => {
  it("walks missing → downloading → restart → ready", () => {
    expect(searchModelState(missing, null)).toBe("missing");
    expect(searchModelState(missing, job({}))).toBe("downloading");
    // Done, before the status refetch lands.
    expect(searchModelState(missing, job({ done: true, fraction: 1 }))).toBe("restart");
    expect(searchModelState({ ...missing, installed: true }, null)).toBe("restart");
    expect(searchModelState({ ...missing, installed: true, loaded: true }, null)).toBe("ready");
  });

  it("reports a failure, and a cancel goes back to missing", () => {
    expect(searchModelState(missing, job({ done: true, error: "Failed to download Search model weights: timed out" }))).toBe(
      "failed",
    );
    expect(searchModelState(missing, job({ done: true, cancelled: true }))).toBe("missing");
  });

  it("clamps progress", () => {
    expect(percent(0.456)).toBe("46%");
    expect(percent(1.2)).toBe("100%");
  });
});
