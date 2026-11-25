import { beforeEach, describe, expect, it, vi } from "vitest";

const settings = vi.fn();
vi.mock("../../api", () => ({ aiSettings: () => settings(), whimbrelAiStatus: () => Promise.resolve(null), chatRoutePreview: vi.fn() }));
vi.mock("../../today/todayModel", () => ({ askAvailable: (routing: { ok: boolean }) => routing.ok, askModelLabel: () => "Model" }));

import { createFocusRefresh, refreshAskState, resetAskState } from "./askState";
import { routePreviews } from "./routePreview";

beforeEach(() => {
  resetAskState();
  settings.mockReset();
});

describe("whether Ask can answer", () => {
  it("shares one read among callers asking at once", async () => {
    settings.mockResolvedValue({ routing: { ok: true }, keys: {} });
    await Promise.all([refreshAskState(), refreshAskState()]);
    expect(settings).toHaveBeenCalledTimes(1);
  });
  it("forgets the route previews kept before the settings were read again", async () => {
    settings.mockResolvedValue({ routing: { ok: true }, keys: {} });
    const clear = vi.spyOn(routePreviews, "clear");
    await refreshAskState();
    expect(clear).toHaveBeenCalled();
  });
  it("does not wait forever when the settings cannot be read", async () => {
    settings.mockRejectedValue(new Error("down"));
    await refreshAskState();
    // A second, successful read replaces the stand-in.
    settings.mockResolvedValue({ routing: { ok: true }, keys: {} });
    await refreshAskState();
    expect(settings).toHaveBeenCalledTimes(2);
  });
});

describe("refreshing when the window is focused", () => {
  function fakeWindow() {
    const on = new Set<() => void>();
    return { on, addEventListener: (_t: "focus", l: () => void) => on.add(l), removeEventListener: (_t: "focus", l: () => void) => on.delete(l) };
  }
  it("keeps one listener for the header and the page together, and keeps it while either is mounted", () => {
    const win = fakeWindow();
    const refresh = vi.fn();
    const focus = createFocusRefresh(win, refresh);
    const header = focus.retain();
    const page = focus.retain();
    expect(win.on.size).toBe(1);
    // The page unmounts (another tab): the header still refreshes on focus.
    page();
    expect(win.on.size).toBe(1);
    win.on.forEach((l) => l());
    expect(refresh).toHaveBeenCalledTimes(1);
    header();
    expect(win.on.size).toBe(0);
  });
  it("counts a release once, however many times it is called", () => {
    const win = fakeWindow();
    const focus = createFocusRefresh(win, () => {});
    const a = focus.retain();
    focus.retain();
    a();
    a();
    expect(focus.count()).toBe(1);
    expect(win.on.size).toBe(1);
  });
  it("is harmless with no window", () => {
    const focus = createFocusRefresh(null, () => {});
    focus.retain()();
    expect(focus.count()).toBe(0);
  });
});
