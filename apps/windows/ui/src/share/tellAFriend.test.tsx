import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, text, type TNode } from "../testkit/tree";

// Tell a friend: the message, the Settings card and the tray popup's row. The shell is faked at its one door, `invoke`; the
// clipboard is faked on `navigator`. What is checked is the words copied, that "Copied" shows only after a copy that worked, and
// that nothing but the clipboard is touched (no command is sent: no account, no code, no server).
vi.mock("react", async (actual) => {
  const stubbed = (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>());
  const useContext = (c: { _currentValue: unknown }) => c._currentValue;
  return { ...stubbed, useContext, default: { ...(stubbed as unknown as { default: object }).default, useContext } };
});

const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], clip: [] as string[], refuse: false }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    // The popup reads the launch stage on open; every other read answers nothing.
    return cmd === "launch_state" ? { stage: { kind: "ready" } } : null;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ emit: async () => {}, listen: async () => () => {} }));

const { COPIED_MS, TELL_FRIEND_LINK, tellAFriendMessage } = await import("./tellAFriend");
const { TellAFriendCard } = await import("../settings/redesign/sections/devices/TellAFriendCard");
const { RedesignedPopup } = await import("../popup/RedesignedPopup");

beforeEach(() => {
  h.calls = [];
  h.clip = [];
  h.refuse = false;
  vi.useFakeTimers();
  vi.stubGlobal("navigator", {
    clipboard: {
      writeText: async (s: string) => {
        if (h.refuse) throw new Error("denied");
        h.clip.push(s);
      },
    },
  });
  vi.stubGlobal("window", { addEventListener() {}, removeEventListener() {}, setInterval: () => 0, clearInterval() {}, dispatchEvent: () => true, innerWidth: 320, innerHeight: 480 });
  vi.stubGlobal("document", { activeElement: null, getElementById: () => null, addEventListener() {}, removeEventListener() {} });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const settle = async (tree: Tree) => {
  for (let i = 0; i < 3; i++) {
    await vi.advanceTimersByTimeAsync(0);
    tree.render();
  }
  return tree;
};
const click = (b: TNode) => (b.props.onClick as () => void)();
const buttonsNamed = (tree: Tree, name: string) => tree.all((n) => n.tag === "button" && text(n).trim() === name);

describe("the message", () => {
  it("is a short line about Whimbrel and one plain link, with no code and no reward", () => {
    const message = tellAFriendMessage();
    expect(message).toBe("I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: https://whimbrelapp.com/?utm_source=share");
    expect(message.split(TELL_FRIEND_LINK)).toHaveLength(2);
    expect(message.toLowerCase()).not.toMatch(/code|reward|discount/);
    expect(message).not.toMatch(/[–—]/);
  });

  it("is the plain home page with the one campaign tag, nothing per person", () => {
    const url = new URL(TELL_FRIEND_LINK);
    expect(url.origin).toBe("https://whimbrelapp.com");
    expect(url.pathname).toBe("/");
    expect([...url.searchParams.entries()]).toEqual([["utm_source", "share"]]);
  });

  it("keeps the link once at the end when a translation lost its placeholder", () => {
    expect(tellAFriendMessage(() => "Uso Whimbrel.")).toBe(`Uso Whimbrel. ${TELL_FRIEND_LINK}`);
    expect(tellAFriendMessage(() => "Uso Whimbrel. {link}", "L")).toBe("Uso Whimbrel. L");
    expect(tellAFriendMessage((_key, args) => `Mira ${args?.link} ahora.`, "L")).toBe("Mira L ahora.");
  });
});

describe("Settings search", () => {
  it("finds the card, in Sync & Devices, by its title and by share", async () => {
    const { searchRedesign } = await import("../settings/redesign/search");
    for (const q of ["tell a friend", "share", "friend"]) {
      const hit = searchRedesign(q).find((r) => r.id === "devices.tellAFriend");
      expect(hit?.section, q).toBe("devices");
    }
  });
});

describe("the Settings card", () => {
  const card = async () => settle(new Tree(() => <TellAFriendCard />).render());

  it("is a card with an anchor, its words and a Copy message row with one Copy button", async () => {
    const tree = await card();
    expect(tree.all((n) => n.props.id === "devices.tellAFriend")).toHaveLength(1);
    expect(tree.has("Tell a friend")).toBe(true);
    expect(tree.has("Nothing is sent from here.")).toBe(true);
    expect(buttonsNamed(tree, "Copy")).toHaveLength(1);
  });

  it("copies the message, says Copied for a few seconds, then goes back", async () => {
    const tree = await card();
    click(buttonsNamed(tree, "Copy")[0]);
    await settle(tree);
    expect(h.clip).toEqual([tellAFriendMessage()]);
    expect(buttonsNamed(tree, "Copied")).toHaveLength(1);
    expect(tree.all((n) => n.props.role === "status").map(text)).toContain("Copied");
    await vi.advanceTimersByTimeAsync(COPIED_MS + 1);
    tree.render();
    expect(buttonsNamed(tree, "Copy")).toHaveLength(1);
    expect(buttonsNamed(tree, "Copied")).toHaveLength(0);
  });

  it("does not say Copied when the clipboard refuses, and says so in words", async () => {
    h.refuse = true;
    const tree = await card();
    click(buttonsNamed(tree, "Copy")[0]);
    await settle(tree);
    expect(h.clip).toEqual([]);
    expect(buttonsNamed(tree, "Copied")).toHaveLength(0);
    expect(tree.has("Could not copy. Try again.")).toBe(true);
  });

  it("sends no command to the shell: nothing is sent anywhere", async () => {
    const tree = await card();
    click(buttonsNamed(tree, "Copy")[0]);
    await settle(tree);
    expect(h.calls).toEqual([]);
  });
});

describe("the tray popup", () => {
  const popup = async () => settle(new Tree(() => <RedesignedPopup />).render());

  it("has a quiet Tell a friend row, above Open Whimbrel and Quit", async () => {
    const tree = await popup();
    const row = buttonsNamed(tree, "Tell a friend");
    expect(row).toHaveLength(1);
    expect(row[0].props.className).toContain("w-button-quiet");
    const labels = tree.all((n) => n.tag === "button").map((n) => text(n).trim());
    expect(labels.indexOf("Tell a friend")).toBeGreaterThan(-1);
    expect(labels.indexOf("Tell a friend")).toBeLessThan(labels.indexOf("Open Whimbrel"));
    expect(labels.indexOf("Open Whimbrel")).toBeLessThan(labels.indexOf("Quit"));
    // It says what it does to a screen reader, and the hint is tied to the button.
    const describedBy = row[0].props["aria-describedby"] as string;
    expect(tree.all((n) => n.props.id === describedBy).map(text)).toEqual(["Copies a short message and a link to Whimbrel. Nothing is sent."]);
  });

  it("copies the message and says Copied in the row, and opens nothing", async () => {
    const tree = await popup();
    h.calls = [];
    click(buttonsNamed(tree, "Tell a friend")[0]);
    await settle(tree);
    expect(h.clip).toEqual([tellAFriendMessage()]);
    expect(buttonsNamed(tree, "Copied")).toHaveLength(1);
    expect(buttonsNamed(tree, "Tell a friend")).toHaveLength(0);
    // Nothing was opened, closed or quit by the copy.
    expect(h.calls.map(([c]) => c)).not.toContain("popup_open_main");
    expect(h.calls.map(([c]) => c)).not.toContain("popup_close");
    expect(h.calls.map(([c]) => c)).not.toContain("popup_quit");
    await vi.advanceTimersByTimeAsync(COPIED_MS + 1);
    tree.render();
    expect(buttonsNamed(tree, "Tell a friend")).toHaveLength(1);
  });
});
