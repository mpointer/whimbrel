import { describe, expect, it, vi } from "vitest";
import type { ChatLine, ImageCandidateView, SkinTurnResult } from "../generated/types";
import {
  chatReducer,
  composer,
  FAILED_LINE,
  initialChat,
  placeholder,
  providerLine,
  runTurn,
  SPEND_CAP_LINE,
  type ChatEvent,
  type ChatState,
  type SkinChatApi,
} from "./skinChatModel";

const said = (role: "user" | "whimbrel", text: string): ChatLine => ({ role, text, date: 1 });
const texts = (state: ChatState) => state.turns.map((t) => t.text);
const image = (url: string): ImageCandidateView => ({ url, thumbnailUrl: url, title: url, source: "web" });

function result(over: Partial<SkinTurnResult> = {}): SkinTurnResult {
  return {
    intent: "new",
    notes: ["Navy page, gold accents"],
    lines: ["Here is a stadium skin."],
    stored: true,
    chatSkinId: "user-stadium-1",
    savedSkinId: "user-stadium-1",
    imageQueries: [],
    imagePrompts: [],
    ...over,
  };
}

function fakeApi(over: Partial<SkinChatApi> = {}): SkinChatApi {
  return {
    turn: vi.fn(async () => result()),
    stop: vi.fn(async () => true),
    chat: vi.fn(async () => [said("user", "a stadium skin"), said("whimbrel", "Here is a stadium skin.")]),
    search: vi.fn(async (q: string) => [image(`https://img/${q}/1`), image(`https://img/${q}/2`)]),
    generate: vi.fn(async (p: string) => ({ ...image(`generated:${p}`), source: "generated" })),
    post: vi.fn(async (_id: string, text: string) => ({ text, stored: true })),
    ...over,
  };
}

/** A store the turn dispatches into, as the component's reducer would hold it. */
function harness(skinId = "whimbrel") {
  let state = initialChat(skinId);
  const dispatch = (e: ChatEvent) => {
    state = chatReducer(state, e);
  };
  const send = (text: string, providerLineText: string | null = null) => {
    dispatch({ type: "send", text, providerLine: providerLineText });
    return { run: state.run, runId: `run-${state.run}`, skinId: state.skinId, request: text };
  };
  return { get: () => state, dispatch, send };
}

describe("a turn", () => {
  it("status lines appear in order", async () => {
    const h = harness();
    const api = fakeApi({ turn: vi.fn(async () => result({ imageQueries: ["stadium lights"], imagePrompts: ["a stadium at night"] })) });
    const turn = h.send("a stadium photo skin", providerLine({ provider: "anthropic", chat: true, search: true, generate: true }));
    expect(h.get().running).toBe(true);
    await runTurn(api, turn, h.dispatch, () => h.get().run === turn.run);
    expect(h.get().status).toEqual([
      "Reading your request…",
      "Sent to Anthropic: your request, this skin's chat, its design and the list of allowed values",
      "Navy page, gold accents",
      "Searching the web for 'stadium lights'…",
      "Making an image: 'a stadium at night'…",
    ]);
    expect(h.get().running).toBe(false);
    // The saved skin's chat, as the shell stored it.
    expect(h.get().skinId).toBe("user-stadium-1");
    expect(texts(h.get())).toEqual(["a stadium skin", "Here is a stadium skin."]);
    expect(h.get().candidates.map((c) => c.url)).toEqual(["https://img/stadium lights/1", "https://img/stadium lights/2", "generated:a stadium at night"]);
    expect(api.turn).toHaveBeenCalledWith("whimbrel", "a stadium photo skin", "run-1");
  });

  it("names no provider for Ollama, which stays on this PC", () => {
    expect(providerLine({ provider: "ollama", chat: true, search: false, generate: false })).toBeNull();
  });

  it("stop discards late reply", async () => {
    const h = harness();
    let reply!: (r: SkinTurnResult) => void;
    const api = fakeApi({ turn: () => new Promise((resolve) => (reply = resolve)) });
    const turn = h.send("a stadium skin");
    const running = runTurn(api, turn, h.dispatch, () => h.get().run === turn.run);
    h.dispatch({ type: "stop" });
    reply(result({ imageQueries: ["lights"] }));
    await running;
    expect(h.get().status).toEqual(["Reading your request…", "Stopped."]);
    expect(h.get().skinId).toBe("whimbrel");
    expect(texts(h.get())).toEqual(["a stadium skin"]);
    expect(api.search).not.toHaveBeenCalled();
    // A stop when nothing runs changes nothing.
    expect(chatReducer(h.get(), { type: "stop" })).toBe(h.get());
  });

  it("a question back on a built-in is shown, not stored", async () => {
    const h = harness();
    const api = fakeApi({ turn: vi.fn(async () => result({ intent: "clarify", lines: ["Which team?"], stored: false, chatSkinId: "whimbrel", savedSkinId: null })) });
    const turn = h.send("sports");
    await runTurn(api, turn, h.dispatch, () => true);
    expect(texts(h.get())).toEqual(["sports", "Which team?"]);
    expect(api.chat).not.toHaveBeenCalled();
  });

  it("a failed turn says nothing changed and stops", async () => {
    const h = harness();
    const turn = h.send("a skin");
    await runTurn(fakeApi({ turn: vi.fn(async () => Promise.reject({ kind: "provider_failure" })) }), turn, h.dispatch, () => true);
    expect(texts(h.get())).toEqual(["a skin", FAILED_LINE]);
    expect(h.get().running).toBe(false);
  });

  it("cap reached message stops", async () => {
    const h = harness();
    const cap = { kind: "spend_cap", scope: "user", spent_cents: 200, cap_cents: 200 };
    const turn = h.send("a skin");
    await runTurn(fakeApi({ turn: vi.fn(async () => Promise.reject(cap)) }), turn, h.dispatch, () => true);
    expect(texts(h.get()).at(-1)).toBe(SPEND_CAP_LINE);
    expect(h.get().running).toBe(false);
    // Mid-search: the line is posted to the skin's chat, and no image is made after it.
    const h2 = harness();
    const api = fakeApi({
      turn: vi.fn(async () => result({ imageQueries: ["a", "b"], imagePrompts: ["c"] })),
      search: vi.fn(async () => Promise.reject(cap)),
    });
    const turn2 = h2.send("a photo skin");
    await runTurn(api, turn2, h2.dispatch, () => h2.get().run === turn2.run);
    expect(api.search).toHaveBeenCalledTimes(1);
    expect(api.generate).not.toHaveBeenCalled();
    expect(api.post).toHaveBeenCalledWith("user-stadium-1", SPEND_CAP_LINE);
    expect(texts(h2.get()).at(-1)).toBe(SPEND_CAP_LINE);
    expect(h2.get().running).toBe(false);
  });

  it("offers at most six found images", async () => {
    const h = harness();
    const many = (q: string) => Array.from({ length: 5 }, (_, i) => image(`https://img/${q}/${i}`));
    const api = fakeApi({ turn: vi.fn(async () => result({ imageQueries: ["a", "b", "c"] })), search: vi.fn(async (q: string) => many(q)) });
    const turn = h.send("a skin");
    await runTurn(api, turn, h.dispatch, () => true);
    expect(h.get().candidates).toHaveLength(6);
    expect(api.search).toHaveBeenCalledTimes(2);
  });
});

describe("the chat", () => {
  it("switching skin shows that skin's chat", () => {
    let state = initialChat("user-a-1", [said("user", "make it red")]);
    state = chatReducer(state, { type: "found", run: 0, candidates: [image("x")] });
    state = { ...state, candidates: [image("x")], status: ["Navy page"] };
    state = chatReducer(state, { type: "show", skinId: "user-b-1", turns: [said("user", "make it blue")] });
    expect(state.skinId).toBe("user-b-1");
    expect(texts(state)).toEqual(["make it blue"]);
    expect(state.candidates).toEqual([]);
    expect(state.status).toEqual([]);
    // Picking another skin while a turn runs stops it there: its late reply and images change nothing.
    let running = chatReducer(initialChat("user-a-1"), { type: "send", text: "make it red", providerLine: null });
    const run = running.run;
    running = chatReducer(running, { type: "show", skinId: "user-b-1", turns: [] });
    expect(running.running).toBe(false);
    expect(running.status.at(-1)).toBe("Stopped.");
    const late = chatReducer(running, { type: "replied", run, result: result(), turns: null });
    expect(late).toBe(running);
    expect(chatReducer(late, { type: "found", run, candidates: [image("z")] }).candidates).toEqual([]);
    // The same skin again only refreshes its turns.
    const again = chatReducer({ ...state, candidates: [image("y")] }, { type: "show", skinId: "user-b-1", turns: [] });
    expect(again.candidates).toHaveLength(1);
  });

  it("a line for another skin's chat is not shown here", () => {
    const state = chatReducer(initialChat("user-a-1"), { type: "line", skinId: "user-b-1", text: "Added the image to the header." });
    expect(state.turns).toEqual([]);
  });

  it("undo restores previous and reloads", () => {
    // The page posts the shell's line and then reloads the skins; the line shows in this chat.
    const state = chatReducer(initialChat("user-a-1"), { type: "line", skinId: "user-a-1", text: "Undone: this skin is back to its previous version." });
    expect(texts(state)).toEqual(["Undone: this skin is back to its previous version."]);
  });

  it("found-these strip shows candidates, reject forgets", () => {
    let state = { ...initialChat("user-a-1"), running: true, run: 1 };
    state = chatReducer(state, { type: "found", run: 1, candidates: [image("a"), image("b")] });
    state = chatReducer(state, { type: "found", run: 1, candidates: [image("b"), image("c")] });
    expect(state.candidates.map((c) => c.url)).toEqual(["a", "b", "c"]);
    // Found by a stopped run: dropped.
    expect(chatReducer(state, { type: "found", run: 0, candidates: [image("d")] }).candidates).toHaveLength(3);
    state = chatReducer(state, { type: "forget", url: "b" });
    expect(state.candidates.map((c) => c.url)).toEqual(["a", "c"]);
  });

  it("no provider shows set-up state", () => {
    expect(composer(undefined)).toBe("loading");
    expect(composer(null)).toBe("set-up");
    expect(composer({ provider: "anthropic", chat: false, search: true, generate: true })).toBe("set-up");
    expect(composer({ provider: "ollama", chat: true, search: false, generate: false })).toBe("compose");
  });

  it("asks for a change on your skins and a skin on the built-ins", () => {
    expect(placeholder("user-a-1")).toBe("Ask for a change…");
    expect(placeholder("game-day")).toBe("Describe a skin…");
  });
});
