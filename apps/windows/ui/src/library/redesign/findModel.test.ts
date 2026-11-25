import { describe, expect, it } from "vitest";
import type { FoundCard, NumberedSourceView, RoutePreviewView } from "../../generated/types";
import { NO_FILTER } from "../../today/redesign/homeModel";
import {
  defaultOrder,
  askLabel,
  cardOfRow,
  cardWhen,
  centeredScrollTop,
  citedIn,
  countLabel,
  dayGroups,
  drewOnLine,
  evidenceOf,
  filterInput,
  filterKey,
  filterWords,
  openName,
  localOnlyFailure,
  openTarget,
  routeLine,
  sourceMarkName,
  sourceOfHref,
  staysLocal,
  withoutEvidence,
  answeredLine,
  appendUnique,
  askReadiness,
  effectiveQuery,
  searchedAfterEdit,
  searchesAgain,
  followFormShown,
  requestHeld,
  resolveSend,
  restoreText,
  singleFlight,
  placeRestored,
  promiseOf,
  samePress,
  type SendPress,
} from "./findModel";

const NO_FILTER_INPUT = { startUnixMs: null, endUnixMs: null, app: null, person: null, personKey: null };
const card = (over: Partial<FoundCard>): FoundCard => ({ kind: "capture", id: 1, found: true, title: "Window: Pricing v3.xlsx", tsUnixMs: Date.UTC(2026, 10, 13, 11, 2), ...over });
const preview = (over: Partial<RoutePreviewView>): RoutePreviewView => ({ destination: "local", local: true, mayFallBack: false, provider: "ollama", model: "gemma4:12b", reason: "", requireLocal: true, ...over });

describe("the answer's marks", () => {
  const valid = new Set([1, 2, 3]);
  it("names the source a link means, and no other link", () => {
    expect(sourceOfHref("#src-12")).toBe(12);
    expect(sourceOfHref("https://example.test/#src-1")).toBeNull();
    expect(sourceOfHref(undefined)).toBeNull();
  });
  it("lists the cited numbers once, in the order written, among the sources", () => {
    expect(citedIn("b [3] a [1] again [3] none [9]", valid)).toEqual([3, 1]);
  });
});

describe("cards", () => {
  it("names each Open by its time, so the lines of one meeting are told apart", () => {
    const line = (startCs: number) => card({ kind: "transcript", id: startCs, meetingId: 3, title: "Design sync", startCs });
    expect(openName(line(2900), "en-US")).toBe("Open Design sync at 0:29");
    expect(openName(line(6100), "en-US")).toBe("Open Design sync at 1:01");
    expect(openName(line(2900), "en-US")).not.toBe(openName(line(6100), "en-US"));
    expect(openName(card({ kind: "capture", title: "Pricing v3.xlsx", tsUnixMs: new Date(2026, 10, 13, 11, 2).getTime() }), "en-US")).toMatch(/^Open Pricing v3\.xlsx at 11:02\sAM$/);
    expect(openName(card({ kind: "meeting", title: "Design sync", tsUnixMs: null }), "en-US")).toBe("Open Design sync");
  });
  it("opens what the spec says opens, and nothing else", () => {
    expect(openTarget(card({ kind: "capture", id: 5 }))).toEqual({ kind: "capture", id: 5 });
    expect(openTarget(card({ kind: "transcript", id: 8, meetingId: 3 }))).toEqual({ kind: "meeting", id: 3, line: 8 });
    expect(openTarget(card({ kind: "snapshot", id: 9, meetingId: 3 }))).toEqual({ kind: "meeting", id: 3 });
    expect(openTarget(card({ kind: "episode", id: 4 }))).toEqual({ kind: "episode", id: 4 });
    expect(openTarget(card({ kind: "thread" }))).toBeNull();
    expect(openTarget(card({ kind: "routine_run" }))).toBeNull();
    expect(openTarget(card({ found: false }))).toBeNull();
  });
  it("names a source mark by its number, app and day and time", () => {
    const slack = card({ kind: "capture", app: "slack.exe", tsUnixMs: new Date(2026, 10, 12, 16, 10).getTime() });
    expect(sourceMarkName(2, slack, "en-US")).toBe("Source 2, Slack, Thursday 4:10 PM");
    expect(sourceMarkName(3, card({ found: false }), "en-US")).toBe("Source 3, no longer on this device");
    expect(sourceMarkName(4, undefined, "en-US")).toBe("Source 4");
  });
  it("says how far into the meeting a transcript line is", () => {
    const line = card({ kind: "transcript", startCs: 87_200, tsUnixMs: new Date(2026, 10, 10, 14, 32).getTime() });
    expect(cardWhen(line, new Date(2026, 10, 18), "en-US")).toBe("Tue, Nov 10 · 2:32 PM · 14:32 into the meeting");
  });
  it("draws a Recent row as the same card", () => {
    const row = { kind: "capture", id: 7, tsUnixMs: 5, app: "excel.exe", title: "", snippet: "Rate cell", source: null, device: { deviceId: "d", label: "This PC", thisDevice: true } } as const;
    const c = cardOfRow(row);
    expect(c).toMatchObject({ kind: "capture", id: 7, title: "Excel", text: "Rate cell", found: true });
  });
});

describe("the time line", () => {
  const now = new Date(2026, 10, 18, 10, 22);
  const at = (d: number, h: number, id: number) => card({ id, tsUnixMs: new Date(2026, 10, d, h).getTime() });
  it("groups by day, newest day and newest time first, and puts rows with no time last", () => {
    const groups = dayGroups([at(10, 9, 1), at(12, 9, 2), at(12, 15, 3), card({ id: 4, tsUnixMs: null })], now, "en-US", "newest");
    expect(groups.map((g) => g.cards.map((c) => c.id))).toEqual([[3, 2], [1], [4]]);
    expect(groups[2].label).toBe("No date");
  });
  it("orders sources oldest first", () => {
    const groups = dayGroups([at(12, 9, 2), at(10, 9, 1)], now, "en-US", "oldest");
    expect(groups.map((g) => g.cards[0].id)).toEqual([1, 2]);
  });
  it("shows an answer's cited sources first in time order and leaves them out of the results", () => {
    const sources: NumberedSourceView[] = [
      { n: 1, card: at(12, 9, 11) },
      { n: 2, card: at(10, 9, 12) },
      { n: 3, card: at(13, 9, 13) },
    ];
    const evidence = evidenceOf(sources, [3, 1]);
    expect(evidence.map((e) => e.n)).toEqual([1, 3]);
    expect(evidenceOf(sources, null)).toEqual([]);
    expect(withoutEvidence([at(12, 9, 11), at(14, 9, 20)], evidence).map((c) => c.id)).toEqual([20]);
  });
});

describe("filters", () => {
  it("builds the core's filter from the chips", () => {
    const now = new Date(2026, 10, 18, 10, 22).getTime();
    const f = filterInput({ person: "Sam Okafor", app: "excel.exe", dates: { kind: "today" } }, now);
    expect(f).toMatchObject({ app: "excel.exe", person: "Sam Okafor", startUnixMs: new Date(2026, 10, 18).getTime(), endUnixMs: new Date(2026, 10, 19).getTime() });
    expect(filterKey(filterInput(NO_FILTER, now))).toBe("[null,null,null,null]");
  });
  it("says what each filter does, by the spec's table", () => {
    expect(filterWords({ person: "Sam Okafor", app: null, dates: null })).toContain("listed apart");
    expect(filterWords({ person: null, app: "excel.exe", dates: null })).toContain("hidden while an app is chosen");
    expect(filterWords(NO_FILTER)).toBe("");
  });
  it("names the dates an answer drew on", () => {
    const f = filterInput({ person: null, app: null, dates: { kind: "span", start: new Date(2026, 10, 9).getTime(), end: new Date(2026, 10, 16).getTime(), label: "x" } }, 0);
    expect(drewOnLine(f, "en-US")).toBe("From your work memory, Nov 9 to Nov 15");
    expect(drewOnLine({}, "en-US")).toBe("From your work memory");
  });
  it("counts results and marks a capped search", () => {
    expect(countLabel(null)).toBeNull();
    expect(countLabel({ total: 1, mentionsTotal: 0, capped: false })).toBe("1 result");
    expect(countLabel({ total: 3, mentionsTotal: 2, capped: false })).toBe("5 results");
    expect(countLabel({ total: 400, mentionsTotal: 0, capped: true })).toBe("400+ results");
  });
});

describe("the Ask button", () => {
  it("says on this PC only when the preview said the answer is made here", () => {
    expect(askLabel(preview({}))).toBe("Ask, on this PC");
    expect(askLabel(preview({ destination: "unknown", local: false, requireLocal: false }))).toBe("Ask");
    expect(askLabel(preview({ destination: "refused", local: false, requireLocal: false }))).toBe("Ask");
    expect(askLabel(null)).toBe("Ask");
  });
  it("names a provider only when nothing else can take the answer over", () => {
    const hosted = { destination: "hosted", local: false, requireLocal: false, provider: "anthropic", model: "m" };
    expect(askLabel(preview({ ...hosted, mayFallBack: false }))).toBe("Ask Anthropic");
    expect(askLabel(preview({ ...hosted, mayFallBack: true }))).toBe("Ask");
  });
  it("asks with require-local exactly when the label promised a local answer", () => {
    expect(staysLocal(preview({}))).toBe(true);
    expect(staysLocal(preview({ destination: "hosted", local: false, requireLocal: false }))).toBe(false);
    expect(staysLocal(null)).toBe(false);
    expect(routeLine(preview({}))).toBe("Answers are written on this PC by gemma4:12b.");
    expect(routeLine(null)).toBeNull();
  });
});


describe("loading more", () => {
  it("adds only the cards not already listed, by kind and id", () => {
    const a = card({ kind: "capture", id: 1 });
    const b = card({ kind: "capture", id: 2 });
    const sameIdOtherKind = card({ kind: "meeting", id: 2 });
    expect(appendUnique([a, b], [b, sameIdOtherKind, card({ kind: "capture", id: 3 })]).map((c) => `${c.kind}-${c.id}`)).toEqual(["capture-1", "capture-2", "meeting-2", "capture-3"]);
  });
});

describe("what the page searches for", () => {
  const none = filterInput(NO_FILTER, 0);
  const some = filterInput({ ...NO_FILTER, app: "excel.exe" }, 0);
  it("lists a filter's holdings when words are empty and a filter is on, and searches nothing when neither is", () => {
    expect(effectiveQuery("", true)).toBe("");
    expect(effectiveQuery("", false)).toBeNull();
    expect(effectiveQuery(null, true)).toBe("");
    expect(effectiveQuery(null, false)).toBeNull();
    expect(effectiveQuery("pricing", false)).toBe("pricing");
    expect(some).not.toEqual(none);
  });
});

describe("clearing the field", () => {
  it("forgets the search, so the page goes back to its empty state (or a filter's list), and Enter on the empty field shows no old results", () => {
    expect(searchedAfterEdit("", "pricing")).toBeNull();
    expect(searchedAfterEdit("   ", "pricing")).toBeNull();
    expect(effectiveQuery(searchedAfterEdit("", "pricing"), false)).toBeNull();
    expect(effectiveQuery(searchedAfterEdit("", "pricing"), true)).toBe("");
  });
  it("keeps the results for the words searched while new words are typed, until Enter", () => {
    expect(searchedAfterEdit("pric", "pricing")).toBe("pricing");
    expect(searchedAfterEdit("", null)).toBeNull();
  });
});

describe("asking before the AI state is known", () => {
  it("tells unknown from ready from unavailable", () => {
    expect(askReadiness(null)).toBe("unknown");
    expect(askReadiness({ available: true, model: "m" })).toBe("ready");
    expect(askReadiness({ available: false, model: "" })).toBe("unavailable");
  });
  it("holds an ask request, and only an ask request, until the state is known", () => {
    expect(requestHeld("ask", "unknown")).toBe(true);
    expect(requestHeld("ask", "ready")).toBe(false);
    expect(requestHeld("ask", "unavailable")).toBe(false);
    expect(requestHeld("search", "unknown")).toBe(false);
  });
});

describe("the follow-up field", () => {
  it("stays after a failure or a stop, and goes only while an answer is on its way", () => {
    const turn = (status: string) => ({ status }) as never;
    expect(followFormShown(undefined)).toBe(false);
    expect(followFormShown(turn("done"))).toBe(true);
    expect(followFormShown(turn("failed"))).toBe(true);
    expect(followFormShown(turn("stopped"))).toBe(true);
    expect(followFormShown(turn("starting"))).toBe(false);
    expect(followFormShown(turn("waiting"))).toBe(false);
    expect(followFormShown(turn("streaming"))).toBe(false);
    expect(followFormShown(turn("blocking"))).toBe(false);
  });
});

describe("words put back in a field", () => {
  it("never overwrite what the person typed since", () => {
    expect(restoreText("", "the lost question")).toBe("the lost question");
    expect(restoreText("   ", "the lost question")).toBe("the lost question");
    expect(restoreText("something new", "the lost question")).toBe("something new");
  });
});

describe("the route read just before a question goes out", () => {
  const local = (over: Partial<RoutePreviewView> = {}): RoutePreviewView => ({ destination: "local", local: true, requireLocal: true, mayFallBack: false, provider: "ollama", model: "gemma", ...over }) as RoutePreviewView;
  const hosted = (provider = "anthropic", over: Partial<RoutePreviewView> = {}): RoutePreviewView => ({ destination: "hosted", local: false, requireLocal: false, mayFallBack: false, provider, model: "m", ...over }) as RoutePreviewView;
  const fallsBack = hosted("anthropic", { mayFallBack: true });
  it("names the promise a label makes, and nothing for a plain Ask", () => {
    expect(promiseOf(local())).toBe("local");
    expect(promiseOf(hosted())).toBe("hosted:anthropic");
    expect(promiseOf(fallsBack)).toBeNull();
    expect(promiseOf(null)).toBeNull();
  });
  it("refuses when the button promised this PC and the route has moved", async () => {
    expect(await resolveSend(local(), () => Promise.resolve(hosted()))).toEqual({ go: false, why: "route" });
  });
  it("refuses when the button named a provider and the route is another provider, this PC, or may fall back", async () => {
    const named = hosted("anthropic");
    expect(await resolveSend(named, () => Promise.resolve(hosted("openai")))).toEqual({ go: false, why: "route" });
    expect(await resolveSend(named, () => Promise.resolve(local()))).toEqual({ go: false, why: "route" });
    expect(await resolveSend(named, () => Promise.resolve(fallsBack))).toEqual({ go: false, why: "route" });
    expect(await resolveSend(named, () => Promise.resolve(null))).toEqual({ go: false, why: "route" });
  });
  it("goes when the promise still holds", async () => {
    expect(await resolveSend(hosted(), () => Promise.resolve(hosted()))).toEqual({ go: true, requireLocal: false });
    expect(await resolveSend(local({ requireLocal: false }), () => Promise.resolve(local({ requireLocal: false })))).toEqual({ go: true, requireLocal: false });
  });
  it("refuses when the button promised this PC without insisting on it and the route is now hosted", async () => {
    expect(await resolveSend(local({ requireLocal: false }), () => Promise.resolve(hosted()))).toEqual({ go: false, why: "route" });
  });
  it("goes with the fresh route's word when the button promised nothing", async () => {
    expect(await resolveSend(null, () => Promise.resolve(local()))).toEqual({ go: true, requireLocal: true });
    expect(await resolveSend(null, () => Promise.resolve(hosted()))).toEqual({ go: true, requireLocal: false });
    expect(await resolveSend(fallsBack, () => Promise.resolve(hosted("openai")))).toEqual({ go: true, requireLocal: false });
  });
  it("follows what the button showed when the fresh read fails: this PC goes (the call fails rather than leave), a provider does not", async () => {
    expect(await resolveSend(local(), () => Promise.reject(new Error("down")))).toEqual({ go: true, requireLocal: true });
    expect(await resolveSend(null, () => Promise.reject(new Error("down")))).toEqual({ go: true, requireLocal: false });
    expect(await resolveSend(hosted(), () => Promise.reject(new Error("down")))).toEqual({ go: false, why: "route" });
  });
  describe("what was pressed", () => {
    const pressed: SendPress = { words: "what did Sam say", filterKey: "k1", conversationId: 5 };
    it("compares the words, the filters and the conversation", () => {
      expect(samePress(pressed, { ...pressed })).toBe(true);
      expect(samePress(pressed, { ...pressed, words: "other" })).toBe(false);
      expect(samePress(pressed, { ...pressed, filterKey: "k2" })).toBe(false);
      expect(samePress(pressed, { ...pressed, conversationId: null })).toBe(false);
    });
    it("does not send when the filters changed while the route was being read", async () => {
      let now = pressed;
      const read = () => {
        now = { ...pressed, filterKey: "k2" };
        return Promise.resolve(hosted());
      };
      expect(await resolveSend(null, read, pressed, () => now)).toEqual({ go: false, why: "changed" });
    });
    it("does not send when the words or the conversation changed meanwhile", async () => {
      let now = pressed;
      expect(await resolveSend(null, () => ((now = { ...pressed, words: "else" }), Promise.resolve(hosted())), pressed, () => now)).toEqual({ go: false, why: "changed" });
      expect(await resolveSend(null, () => ((now = { ...pressed, conversationId: 6 }), Promise.resolve(hosted())), pressed, () => now)).toEqual({ go: false, why: "changed" });
    });
    it("says the press changed before it says the route did", async () => {
      expect(await resolveSend(local(), () => Promise.resolve(hosted()), pressed, () => ({ ...pressed, words: "else" }))).toEqual({ go: false, why: "changed" });
    });
    it("sends when nothing moved", async () => {
      expect(await resolveSend(null, () => Promise.resolve(hosted()), pressed, () => ({ ...pressed }))).toEqual({ go: true, requireLocal: false });
    });
  });
});

describe("pressing Ask twice", () => {
  it("runs once while the first is still waiting, and again after it ends", async () => {
    let release: () => void = () => {};
    let runs = 0;
    const send = singleFlight(
      () =>
        new Promise<void>((resolve) => {
          runs += 1;
          release = resolve;
        }),
    );
    const first = send();
    void send();
    expect(runs).toBe(1);
    release();
    await first;
    void send();
    expect(runs).toBe(2);
  });
  it("lets the next one through after a failure", async () => {
    let runs = 0;
    const send = singleFlight(() => {
      runs += 1;
      return Promise.reject(new Error("no"));
    });
    await send().catch(() => {});
    await send().catch(() => {});
    expect(runs).toBe(2);
  });
  it("never drops a press of another control because this one is busy, and ignores only a double press of the same one", async () => {
    const started: string[] = [];
    const releases: Array<() => void> = [];
    const send = singleFlight(
      (origin: string) =>
        new Promise<void>((resolve) => {
          started.push(origin);
          releases.push(resolve);
        }),
      (origin) => origin,
    );
    void send("header");
    void send("follow");
    void send("header");
    void send("follow");
    expect(started).toEqual(["header", "follow"]);
    releases[0]();
    await Promise.resolve();
    await Promise.resolve();
    void send("header");
    expect(started).toEqual(["header", "follow", "header"]);
  });
});

describe("words that go back to a field", () => {
  it("are put in an empty field, and left alone when the field already holds them", () => {
    expect(placeRestored("", "q", false)).toBe("put");
    expect(placeRestored("  ", "q", true)).toBe("put");
    expect(placeRestored("q", "q", true)).toBe("already");
  });
  it("never replace newer words, but a question that was waiting is offered instead of being lost", () => {
    expect(placeRestored("newer", "q", false)).toBe("already");
    expect(placeRestored("newer", "q", true)).toBe("offer");
  });
});

describe("a source the core could not read", () => {
  it("is not called gone", () => {
    const gone = card({ found: false });
    expect(sourceMarkName(2, gone, "en-US")).toBe("Source 2, no longer on this device");
    expect(sourceMarkName(2, gone, "en-US", true)).toBe("Source 2, could not be read just now");
  });
});

describe("what the answer drew on", () => {
  it("uses the range the core says, when there is one, over the filter's", () => {
    const f = { startUnixMs: new Date(2026, 10, 1).getTime(), endUnixMs: new Date(2026, 10, 30).getTime(), app: null, person: null, personKey: null };
    expect(drewOnLine(f, "en-US", { startUnixMs: new Date(2026, 10, 9).getTime(), endUnixMs: new Date(2026, 10, 16).getTime() })).toBe("From your work memory, Nov 9 to Nov 15");
    expect(drewOnLine(NO_FILTER_INPUT, "en-US", { startUnixMs: new Date(2026, 10, 9).getTime(), endUnixMs: new Date(2026, 10, 16).getTime() })).toBe("From your work memory, Nov 9 to Nov 15");
    expect(drewOnLine(NO_FILTER_INPUT, "en-US", null)).toBe("From your work memory");
  });
});

describe("the line under a finished answer", () => {
  const turn = (over: object) => ({ answeredLocally: true, routedLocally: false, excludedClient: false, saved: false, ...over }) as never;
  it("says nothing when a hosted model answered", () => {
    expect(answeredLine(turn({ answeredLocally: false }))).toBeNull();
  });
  it("names the client rule only while the answer is live", () => {
    expect(answeredLine(turn({ excludedClient: true }))).toContain("excluded");
    expect(answeredLine(turn({}))).toBe("Answered on this PC.");
  });
  it("makes no claim on a reopened answer that its saved data cannot support", () => {
    const saved = answeredLine(turn({ saved: true, excludedClient: false }));
    expect(saved).toBe("Answered by a local model.");
    expect(answeredLine(turn({ saved: true, excludedClient: true }))).toBe(saved);
  });
});

describe("bringing a source card into view", () => {
  const box = { scrollTop: 400, scrollHeight: 3000, top: 100, height: 600 };
  it("scrolls the page's own container so the card sits in its middle", () => {
    // The card is 900 px below the container's top edge, 200 px tall: its middle lands at the container's middle.
    expect(centeredScrollTop(box, { top: 1000, height: 200 })).toBe(400 + 900 - 200);
  });
  it("shows the top of a card taller than the container", () => {
    expect(centeredScrollTop(box, { top: 1000, height: 800 })).toBe(400 + 900);
  });
  it("never scrolls past either end", () => {
    expect(centeredScrollTop(box, { top: -500, height: 50 })).toBe(0);
    expect(centeredScrollTop(box, { top: 5000, height: 50 })).toBe(3000 - 600);
  });
});

describe("a question that had to stay on this PC", () => {
  const noOllama = { kind: "provider_failure", reason: { kind: "ollama_not_running" } } as never;
  const NOT_SENT = "Not sent: this question was to be answered on this PC, and no model on this PC could answer it. Set one up in Settings → AI.";
  it("says it was not sent when no model on this PC could answer, never suggesting a hosted one", () => {
    expect(localOnlyFailure(noOllama, true)).toBe(NOT_SENT);
    expect(localOnlyFailure({ kind: "provider_failure", reason: { kind: "local_model_missing" } } as never, true)).toBe(NOT_SENT);
    expect(localOnlyFailure({ kind: "provider_failure", reason: { kind: "no_provider_configured" } } as never, true)).toBe(NOT_SENT);
  });
  it("leaves an ordinary question, and any other failure, to the usual message", () => {
    expect(localOnlyFailure(noOllama, false)).toBeNull();
    expect(localOnlyFailure({ kind: "provider_failure", reason: { kind: "rate_limited", provider: "anthropic" } } as never, true)).toBeNull();
    expect(localOnlyFailure({ kind: "store", message: "x" } as never, true)).toBeNull();
  });
  it("names this PC on the button when the route must stay here but no model here can answer, and sends it as such", () => {
    for (const reason of ["local_only_no_local_model", "excluded_client_no_local_model", "local_only_cloud_or_unknown_model"]) {
      const refused = preview({ destination: "refused", local: false, requireLocal: false, reason, provider: null, model: null });
      expect(askLabel(refused)).toBe("Ask, on this PC");
      expect(staysLocal(refused)).toBe(true);
      expect(promiseOf(refused)).toBe("local");
    }
    const spent = preview({ destination: "refused", local: false, requireLocal: false, reason: "spend_cap" });
    expect(askLabel(spent)).toBe("Ask");
    expect(staysLocal(spent)).toBe(false);
  });
});

describe("pressing Search again with the same words", () => {
  it("searches again, so a rerun never stays on an older answer", () => {
    expect(searchesAgain("bake sale", "bake sale")).toBe(true);
    expect(searchesAgain("bake sale", "plant sale")).toBe(false);
    expect(searchesAgain(null, null)).toBe(false);
    expect(searchesAgain("bake sale", null)).toBe(false);
  });
});

describe("defaultOrder", () => {
  it("opens a question on best matches first", () => {
    expect(defaultOrder("Where is the kite festival?")).toBe("rank");
    expect(defaultOrder("who owns the budget")).toBe("rank");
    expect(defaultOrder("budget approved?")).toBe("rank");
    expect(defaultOrder("¿Dónde es el festival")).toBe("rank");
    expect(defaultOrder("Quand est la réunion")).toBe("rank");
    expect(defaultOrder("Wann ist das Treffen")).toBe("rank");
  });
  it("opens plain words by day", () => {
    expect(defaultOrder("kite festival")).toBe("time");
    expect(defaultOrder("fundraising")).toBe("time");
    expect(defaultOrder("whose")).toBe("rank");
    expect(defaultOrder("whoever")).toBe("time");
    expect(defaultOrder("")).toBe("time");
    expect(defaultOrder(null)).toBe("time");
  });
});
