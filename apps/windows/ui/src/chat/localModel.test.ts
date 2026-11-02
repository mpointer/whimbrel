import { describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({ localModelStatus: vi.fn(), localModelDownloadStart: vi.fn(), modelDownloadPoll: vi.fn(), modelDownloadCancel: vi.fn() }));
const { cardState } = await import("./localModel");

const can = { installed: false, canOffer: true };
const running = { jobId: "j", fraction: 0.4, status: "", done: false, cancelled: false };

describe("the local model card on the empty Ask screen", () => {
  it("shows only when nothing is set up and the model isn't here", () => {
    expect(cardState(false, can, null)).toBe("hidden");
    expect(cardState(true, null, null)).toBe("hidden");
    expect(cardState(true, { installed: true, canOffer: true }, null)).toBe("hidden");
    expect(cardState(true, can, null)).toBe("offer");
  });

  it("offers the download only on a PC that can run it", () => {
    expect(cardState(true, { installed: false, canOffer: false }, null)).toBe("setup-only");
  });

  it("follows the download, and offers it again after a failure", () => {
    expect(cardState(true, can, running)).toBe("downloading");
    expect(cardState(true, can, { ...running, done: true, error: "network" })).toBe("failed");
  });
});
