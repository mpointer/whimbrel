import { describe, expect, it } from "vitest";
import type { DownloadView, TranscriptionQualityStatus } from "../generated/types";
import { needsTurboDownload, offerState, qualityChoices } from "./transcriptionQuality";

const status = (over: Partial<TranscriptionQualityStatus>): TranscriptionQualityStatus => ({
  quality: "fast",
  offer: false,
  turboInstalled: false,
  turboSpeed: null,
  ...over,
});
const job = (over: Partial<DownloadView>): DownloadView => ({ jobId: "t", fraction: 0.3, status: "…", done: false, cancelled: false, ...over });

describe("offerState", () => {
  it("offers only when Rust says this PC is fast enough and it wasn't dismissed", () => {
    expect(offerState(null, null)).toBe("hidden");
    expect(offerState(status({ offer: false }), null)).toBe("hidden");
    expect(offerState(status({ offer: true }), null)).toBe("offer");
  });

  it("shows the download in the offer's place, then nothing once it's done or cancelled", () => {
    expect(offerState(status({}), job({}))).toBe("downloading");
    expect(offerState(status({}), job({ done: true, error: "timed out" }))).toBe("failed");
    expect(offerState(status({ turboInstalled: true }), job({ done: true, fraction: 1 }))).toBe("hidden");
    expect(offerState(status({}), job({ done: true, cancelled: true }))).toBe("hidden");
  });
});

describe("the Quality picker", () => {
  it("lists Automatic, Better and Fast, each with a line saying what it means", () => {
    expect(qualityChoices().map((c) => c.id)).toEqual(["automatic", "better", "fast"]);
    expect(qualityChoices().every((c) => c.detail.length > 0)).toBe(true);
  });

  it("downloads large-v3-turbo only for Better, and only when it isn't there", () => {
    expect(needsTurboDownload("better", status({}))).toBe(true);
    expect(needsTurboDownload("better", status({ turboInstalled: true }))).toBe(false);
    expect(needsTurboDownload("automatic", status({}))).toBe(false);
  });
});
