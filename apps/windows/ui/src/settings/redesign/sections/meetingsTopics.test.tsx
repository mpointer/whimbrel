import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownloadView, MeetingModelsStatus } from "../../../generated/types";
import { Tree, text, type TNode } from "../../../testkit/tree";

vi.mock("react", async (actual) => (await import("../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.stubGlobal("window", { addEventListener: () => {}, removeEventListener: () => {} });
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });

const h = vi.hoisted(() => ({
  saveNotice: vi.fn(() => Promise.resolve()),
  notice: { enabled: true, text: "Hello" },
  cancel: vi.fn(),
  start: vi.fn(() => Promise.resolve(null)),
  models: { status: null as unknown, job: null as unknown, error: null as string | null },
  setTopicNames: vi.fn(),
  failSettings: false,
}));

vi.mock("../../../api", () => {
  const settings = {
    heuristicDetection: true,
    autoStart: false,
    liveTranscript: false,
    liveQuestionAlerts: false,
    hasNames: true,
    otherNames: "",
    rememberVoices: false,
    leadMinutes: 2,
    prepLeadMinutes: 15,
  };
  return {
    meetingSettings: () => (h.failSettings ? Promise.reject(new Error("read failed")) : Promise.resolve(settings)),
    meetingSnapshotSettings: () => Promise.resolve({ enabled: true, folder: "C:\\Slides", isDefault: true }),
    setMeetingSettings: (s: unknown) => Promise.resolve(s),
    setMeetingSnapshotsEnabled: () => Promise.resolve({}),
    chooseMeetingSnapshotsFolder: () => Promise.resolve({}),
    resetMeetingSnapshotsFolder: () => Promise.resolve({}),
    voicePeople: () => Promise.resolve([]),
    forgetVoice: () => Promise.resolve([]),
    setMeetingMultilingual: () => Promise.resolve({}),
    setTranscriptionQuality: () => Promise.resolve({}),
    dismissBetterOffer: () => Promise.resolve({}),
    profileGet: () => Promise.resolve({ name: "" }),
    setTopicNames: h.setTopicNames,
    topicNames: () => Promise.resolve({ names: ["Mike"], suggested: [] }),
    topicStepsState: () => Promise.resolve(null),
    topicStepsEstimate: () => Promise.resolve({}),
    setTopicStepsEnabled: () => Promise.resolve({}),
    startTopicSteps: () => Promise.resolve({}),
  };
});
vi.mock("../../ProfileCard", () => ({ PROFILE_SAVED_EVENT: "whimbrel-profile-saved" }));
vi.mock("../../../meetings/recordingNotice", () => ({
  useRecordingNotice: () => ({ settings: h.notice, dismissed: null }),
  saveNoticeSettings: h.saveNotice,
  noticeText: (custom: string) => custom.trim() || "I'm recording this meeting with Whimbrel, on my own computer, for my notes.",
}));
vi.mock("../../meetingModelsDownload", () => ({
  meetingModelsDownload: { use: () => h.models, refresh: () => Promise.resolve(), setStatus: () => {}, get: () => h.models, start: h.start, cancel: h.cancel },
}));
vi.mock("../../turboDownload", () => ({
  turboDownload: { use: () => ({ status: null, job: null, error: null }), refresh: () => Promise.resolve(), setStatus: () => {}, start: () => Promise.resolve(null), cancel: () => {} },
}));

const { MeetingsCardRedesign } = await import("./meetings/MeetingsCardRedesign");
const { MeetingModelsCardRedesign } = await import("./meetings/MeetingModelsCardRedesign");
const { TopicNamesCard } = await import("./topics/TopicNamesCard");
const { TopicEditor } = await import("./topics/TopicsListCard");
const { modelsView, downloadValueText } = await import("./meetings/meetingsRedesignModel");
const { topicNameProblem, ownNameProblem } = await import("./topics/topicsRedesignModel");

const BANNED = /memory database|Activity capture|Capture & Privacy|AI & routing|Never delete/i;

/** The row frame (class w-setting) that holds the label `label`. */
const rowOf = (tree: Tree, label: string): TNode =>
  tree.one((n) => n.tag === "div" && String(n.props.className ?? "").split(" ").includes("w-setting") && text(n).includes(label) && n.children.some((c) => typeof c !== "string" && c.props.className === "w-setting-text" && text(c).startsWith(label)), `row "${label}"`);
const switchIn = (row: TNode): TNode => {
  const found: TNode[] = [];
  const walk = (n: TNode) => (n.props.role === "switch" ? found.push(n) : n.children.forEach((c) => typeof c !== "string" && walk(c)));
  walk(row);
  return found[0];
};

const status = (missing: string[], over: Partial<MeetingModelsStatus> = {}): MeetingModelsStatus => ({ installed: missing.length === 0, missing, dir: "C:\\m", multilingual: false, ...over });
const job = (over: Partial<DownloadView>): DownloadView => ({ jobId: "1", fraction: 0.4, status: "Downloading…", done: false, cancelled: false, ...over });

beforeEach(() => {
  h.saveNotice.mockClear();
  h.cancel.mockClear();
  h.setTopicNames.mockReset();
  h.notice = { enabled: true, text: "Hello" };
});

describe("meetings models wording", () => {
  it("builds the old sentences for missing and failed models, and no banned name", () => {
    const missing = modelsView(status(["Transcription model", "Speaker embedding model"]), null);
    expect(missing.state).toBe("missing");
    expect(missing.needText).toContain("about 110 MB in all");
    expect(missing.buttonLabel).toBe("Download meeting models");
    const failed = modelsView(status(["Transcription model"]), job({ done: true, error: "HTTP 404" }));
    expect(failed.state).toBe("failed");
    expect(failed.buttonLabel).toBe("Try again");
    const multi = modelsView(status(["Multilingual transcription model"], { multilingual: true }), null);
    expect(multi.needText).toContain("Meetings in other languages need");
    expect(multi.needText).toContain("about 150 MB");
    for (const v of [missing, failed, multi]) expect(v.needText).not.toMatch(BANNED);
    expect(modelsView(status([]), null)).toEqual({ state: "ready", needText: null, buttonLabel: null });
  });
  it("writes the progress value as the status and a percent", () => {
    expect(downloadValueText({ status: "Downloading…", fraction: 0.4 })).toBe("Downloading… 40%");
    expect(downloadValueText({ status: "x", fraction: 3 })).toBe("x 100%");
  });
});

describe("recording notice and consent reminder", () => {
  it("are plain full-size rows, not behind a disclosure, and save the same arguments as the old card", async () => {
    const tree = await new Tree(() => <MeetingsCardRedesign />).render().settle();
    expect(tree.all((n) => n.tag === "details" || n.props.role === "button" && n.props["aria-expanded"] !== undefined)).toHaveLength(0);
    const reminder = rowOf(tree, "Remind me to tell people I'm recording");
    // The wording box is there while the reminder is on.
    const wording = tree.one((n) => n.tag === "textarea", "wording box");
    expect(wording.props.value).toBe("Hello");
    // Turning it off calls saveNoticeSettings({ enabled, text }).
    (switchIn(reminder).props.onClick as () => void)();
    expect(h.saveNotice).toHaveBeenCalledWith({ enabled: false, text: "Hello" });
    // Editing the wording and leaving the box saves enabled and the new text.
    (wording.props.onChange as (e: unknown) => void)({ target: { value: "Recording" } });
    tree.render();
    (tree.one((n) => n.tag === "textarea").props.onBlur as () => void)();
    expect(h.saveNotice).toHaveBeenLastCalledWith({ enabled: true, text: "Recording" });
  });
  it("draws the recording notice and the consent reminder as plain full-size rows, with the switch on and off", async () => {
    for (const enabled of [true, false]) {
      h.notice = { enabled, text: "Hello" };
      const tree = await new Tree(() => <MeetingsCardRedesign />).render().settle();
      for (const [id, words] of [
        ["meetings.recordingNotice", enabled ? "Hello" : "Hello"],
        ["meetings.consentReminder", "Some places require everyone's consent before a meeting is recorded."],
      ] as const) {
        const row = tree.one((n) => n.props["data-testid"] === id, id);
        expect(String(row.props.className)).toContain("w-setting");
        const body = tree.one((n) => n.tag === "p" && n.parent === row.children.find((c) => typeof c !== "string") && String(n.props.className) === "rs-meetops-plain", `${id} words`);
        expect(text(body)).toContain(words);
      }
      // The sentences are not the muted secondary description of the switch.
      const sw = rowOf(tree, "Remind me to tell people I'm recording");
      expect(sw.children.some((c) => typeof c !== "string" && String(c.props.className ?? "").includes("w-setting-desc"))).toBe(false);
      expect(tree.all((n) => String(n.props.className ?? "").includes("w-setting-note") && text(n).includes("Some places require"))).toHaveLength(0);
    }
  });
  it("hides the wording box while the reminder is off but keeps the switch", async () => {
    h.notice = { enabled: false, text: "" };
    const tree = await new Tree(() => <MeetingsCardRedesign />).render().settle();
    expect(tree.all((n) => n.tag === "textarea")).toHaveLength(0);
    expect(switchIn(rowOf(tree, "Remind me to tell people I'm recording")).props["aria-checked"]).toBe(false);
  });
});

describe("when the first read of the Meetings settings fails", () => {
  it("shows the error and Try again instead of Loading forever, and Try again reads again", async () => {
    h.failSettings = true;
    const tree = await new Tree(() => <MeetingsCardRedesign />).render().settle();
    expect(tree.has("Loading…")).toBe(false);
    expect(tree.has("read failed")).toBe(true);
    h.failSettings = false;
    (tree.button("Try again").props.onClick as () => void)();
    await tree.settle();
    expect(tree.has("read failed")).toBe(false);
    expect(tree.has("Live transcript during meetings")).toBe(true);
  });
});

describe("meeting models download on a progress row", () => {
  it("shows progress and a Cancel that calls the store's cancel", async () => {
    h.models = { status: status(["Transcription model"]), job: job({}), error: null };
    const tree = await new Tree(() => <MeetingModelsCardRedesign />).render().settle();
    const bar = tree.one((n) => n.props.role === "progressbar", "progress bar");
    expect(bar.props["aria-valuenow"]).toBe(40);
    expect(bar.props["aria-valuetext"]).toBe("Downloading… 40%");
    (tree.button("Cancel").props.onClick as () => void)();
    expect(h.cancel).toHaveBeenCalledTimes(1);
    // While it runs there is no Download button, and the multilingual switch says why it is off.
    expect(tree.hasButton("Download meeting models")).toBe(false);
    expect(tree.has("A download is running")).toBe(true);
  });
  it("offers the download when missing and starts it", async () => {
    h.models = { status: status(["Transcription model"]), job: null, error: null };
    const tree = await new Tree(() => <MeetingModelsCardRedesign />).render().settle();
    expect(tree.all((n) => n.props.role === "progressbar")).toHaveLength(0);
    (tree.button("Download meeting models").props.onClick as () => void)();
    expect(h.start).toHaveBeenCalled();
  });
});

describe("topics add and duplicate refusal", () => {
  it("topicNameProblem refuses another topic's name, ignoring case, but not the one being edited", () => {
    const seeds = [{ key: "a", name: "Acme" }, { key: "b", name: "Globex" }];
    expect(topicNameProblem("  acme ", seeds, null)).toBe("Acme is already a topic.");
    expect(topicNameProblem("acme", seeds, "a")).toBeNull();
    expect(topicNameProblem("Initech", seeds, null)).toBeNull();
    expect(topicNameProblem("", seeds, null)).toBeNull();
    expect(ownNameProblem("MIKE", ["Mike"])).toBe("Mike is already in the list.");
  });
  it("the editor keeps the text and says so instead of saving a repeat, and saves a new one with parsed aliases", () => {
    const onSave = vi.fn();
    const seeds = [{ key: "a", name: "Acme", aliases: [], itemCount: 1, kind: null }];
    const tree = new Tree(() => <TopicEditor seed={null} seeds={seeds} onCancel={() => {}} onSave={onSave} />).render();
    const nameBox = () => tree.all((n) => n.tag === "input")[0];
    (nameBox().props.onChange as (e: unknown) => void)({ target: { value: "acme" } });
    tree.render();
    (tree.one((n) => n.tag === "form").props.onSubmit as (e: unknown) => void)({ preventDefault: () => {} });
    tree.render();
    expect(onSave).not.toHaveBeenCalled();
    expect(tree.has("Acme is already a topic.")).toBe(true);
    (nameBox().props.onChange as (e: unknown) => void)({ target: { value: "Initech" } });
    (tree.all((n) => n.tag === "input")[1].props.onChange as (e: unknown) => void)({ target: { value: "IT, Init Corp ," } });
    tree.render();
    (tree.one((n) => n.tag === "form").props.onSubmit as (e: unknown) => void)({ preventDefault: () => {} });
    expect(onSave).toHaveBeenCalledWith({ name: "Initech", aliases: ["IT", "Init Corp"], kind: null });
  });
  it("Names I go by refuses a name already listed and saves a new one with the same command", async () => {
    h.setTopicNames.mockResolvedValue({ names: ["Mike", "Joe"], suggested: [] });
    const tree = await new Tree(() => <TopicNamesCard />).render().settle();
    const field = () => tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Add a name", "add field");
    const add = () => tree.button("Add");
    (field().props.onChange as (e: unknown) => void)({ target: { value: "mike" } });
    tree.render();
    await (add().props.onClick as () => Promise<void>)();
    tree.render();
    expect(h.setTopicNames).not.toHaveBeenCalled();
    expect(tree.has("mike is already in the list.")).toBe(true);
    (field().props.onChange as (e: unknown) => void)({ target: { value: "Joe" } });
    tree.render();
    await (add().props.onClick as () => Promise<void>)();
    expect(h.setTopicNames).toHaveBeenCalledWith(["Mike", "Joe"]);
  });
});
