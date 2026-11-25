import { describe, expect, it, vi } from "vitest";
import { Tree, type TNode } from "../../../../testkit/tree";
import { setInstalledSkins, type SkinDto } from "../../../../skin";

// The tree's hooks, plus useContext reading each context's default (no provider is drawn) and the three-argument useReducer.
vi.mock("react", async (actual) => {
  const stubbed = (await import("../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>());
  const useContext = (ctx: { _currentValue: unknown }) => ctx._currentValue;
  const base = stubbed.useReducer as (reducer: unknown, init: unknown) => unknown;
  const useReducer = (reducer: unknown, arg: unknown, init?: (a: unknown) => unknown) => base(reducer, init ? init(arg) : arg);
  return { ...stubbed, useContext, useReducer, default: { ...(stubbed as unknown as { default: object }).default, useContext, useReducer } };
});
const { never } = vi.hoisted(() => ({ never: () => new Promise<never>(() => {}) }));
vi.mock("../../../../api", () => ({
  skinSources: () => Promise.resolve({ chat: false, generate: false, provider: "ollama", search: false }),
  skinChatLoad: () => Promise.resolve([]),
  skinChatPost: vi.fn(),
  skinImageAddFile: vi.fn(),
  skinImageApprove: vi.fn(),
  skinImageGenerate: vi.fn(),
  skinImageReject: vi.fn(),
  skinImageSearch: vi.fn(),
  skinImageThumbnail: vi.fn(),
  skinImageUnshown: vi.fn(),
  skinTurn: vi.fn(),
  skinTurnStop: vi.fn(),
  skinUndo: vi.fn(),
  skinsList: vi.fn(),
  skinsRevision: vi.fn(),
  generalSettings: never,
  languageSettings: never,
  updateCheckStatus: never,
  appStatus: never,
}));

const getElementById = vi.fn(() => null);
vi.stubGlobal("document", { activeElement: null, getElementById });
vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
vi.stubGlobal("window", { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }), addEventListener() {}, removeEventListener() {}, setInterval, clearInterval });

const { GeneralSection } = await import("../GeneralSection");
const { SkinChat } = await import("../../../SkinChat");
const { themes } = await import("../../../../themes").then((m) => ({ themes: m.THEMES }));

const base = Object.values(import.meta.glob("../../../../../../../../skins/whimbrel.json", { import: "default", eager: true }))[0] as SkinDto["skin"];
const whimbrel: SkinDto = { skin: base, builtIn: true, palettes: themes.slice(0, 1).map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })) };

const setUpButton = (tree: Tree): TNode => tree.one((n) => n.tag === "button" && n.children.some((c) => typeof c === "string" && c.includes("Set up an AI provider")), "Set up an AI provider button");

describe("Set up an AI provider in General", () => {
  it("opens the AI section from the redesigned General section", async () => {
    setInstalledSkins([whimbrel]);
    const onSection = vi.fn();
    const tree = new Tree(() => <GeneralSection onSection={onSection} onRetentionSaved={() => {}} onOpenAutomations={() => {}} />);
    await tree.render().settle();
    (setUpButton(tree).props.onClick as () => void)();
    expect(onSection).toHaveBeenCalledTimes(1);
    expect(onSection).toHaveBeenCalledWith("ai");
    expect(getElementById).not.toHaveBeenCalledWith("ai-heading");
    // The cards are in the Mac's order, with Make a skin hosting the chat.
    const headings = tree.all((n) => n.tag === "h3" && n.props.className === "w-scard-title").map((n) => (n.children[0] as string));
    expect(headings).toEqual(["General", "Language", "Appearance", "Skin", "Make a skin", "About this PC"]);
    setInstalledSkins([]);
  });

  it("keeps the old behaviour when the chat is given no onSetUp: it looks for the AI heading", async () => {
    setInstalledSkins([whimbrel]);
    getElementById.mockClear();
    const tree = new Tree(() => <SkinChat />);
    await tree.render().settle();
    (setUpButton(tree).props.onClick as () => void)();
    expect(getElementById).toHaveBeenCalledWith("ai-heading");
    setInstalledSkins([]);
  });
});
