import { describe, expect, it } from "vitest";
import type { DownloadView, MeetingModelsStatus } from "../generated/types";
import { downloadSizeMb, meetingModelsState, missingList, MULTILINGUAL_MODEL } from "./meetingModels";

const status = (missing: string[]): MeetingModelsStatus => ({
  installed: missing.length === 0,
  missing,
  dir: "C:\\m",
  multilingual: false,
});
const job = (over: Partial<DownloadView>): DownloadView => ({
  jobId: "1",
  fraction: 0.5,
  status: "Downloading…",
  done: false,
  cancelled: false,
  ...over,
});

describe("meetingModelsState", () => {
  it("is ready once every model is on disk, with no restart step", () => {
    expect(meetingModelsState(status([]), null)).toBe("ready");
    expect(meetingModelsState(status([]), job({ done: true, fraction: 1 }))).toBe("ready");
  });

  it("shows a running download over anything else", () => {
    expect(meetingModelsState(status(["Transcription model"]), job({}))).toBe("downloading");
  });

  it("reports a failed download until it's retried", () => {
    expect(meetingModelsState(status(["Transcription model"]), job({ done: true, error: "HTTP 404" }))).toBe("failed");
  });

  it("is missing after a cancel", () => {
    expect(meetingModelsState(status(["Transcription model"]), job({ done: true, cancelled: true }))).toBe("missing");
  });
});

describe("downloadSizeMb", () => {
  it("adds the multilingual model to the English set only when it's missing", () => {
    expect(downloadSizeMb(["Transcription model", "Speaker embedding model"])).toBe(110);
    expect(downloadSizeMb([MULTILINGUAL_MODEL])).toBe(150);
    expect(downloadSizeMb(["Transcription model", MULTILINGUAL_MODEL])).toBe(260);
    expect(downloadSizeMb([])).toBe(0);
  });
});

describe("missingList", () => {
  it("names one, two or three models in a sentence", () => {
    expect(missingList(["Transcription model"])).toBe("the transcription model");
    expect(missingList(["Transcription model", "Speaker embedding model"])).toBe(
      "the transcription model and the speaker embedding model",
    );
    expect(missingList(["A model", "B model", "C model"])).toBe("the a model, the b model and the c model");
  });
});
