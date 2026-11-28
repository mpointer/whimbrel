import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, press, text, type TNode } from "../../../../testkit/tree";
import { hasBannedPhrase } from "./aiRedesignModel";

vi.mock("react", async (actual) => (await import("../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });
vi.stubGlobal("window", { addEventListener: () => {}, removeEventListener: () => {} });
vi.stubGlobal("setInterval", () => 0);
vi.stubGlobal("clearInterval", () => {});

const SECRET = "sk-example-not-a-key";

const h = vi.hoisted(() => {
  const routing = {
    preferredProvider: "ollama",
    localOnly: false,
    useLocalModel: true,
    cheapFirst: false,
    googleAiBillingConfirmed: false,
    remoteOllamaGoogleData: false,
    ollamaModel: "llama",
    anthropicModel: "a1",
    openaiModel: "o1",
    googleModel: "g1",
    openrouterModel: "",
    whimbrelModel: "claude-sonnet-5-5",
    userDailyCapCents: null,
    globalDailyCapCents: 500,
  };
  const info = (id: string, name: string) => ({ id, name, cheapModel: "", keyPageUrl: `https://example.com/${id}`, policyUrl: null, dataTerms: null });
  const make = () => ({
    budgets: [{ provider: "openai", cents: 1000 }],
    imageSearchKey: null as string | null,
    keys: [
      { provider: "anthropic", masked: null as string | null, syncEnabled: false },
      { provider: "openai", masked: "sk-example-not-a-key" as string | null, syncEnabled: false },
    ],
    modelWarnings: [],
    prompts: [{ surface: "ask", text: "Be brief." }],
    providers: [info("anthropic", "Anthropic"), info("openai", "OpenAI")],
    routing: { ...routing },
    spend: { globalCents: 120, userCents: 0 },
    summaryInstructions: "Use bullets.",
    usage: { byProvider: [], last30DaysCents: 0, monthByProvider: [{ provider: "openai", cents: 250 }], monthToDateCents: 250, todayCents: 0 },
  });
  const api = {
    state: make(),
    aiSettings: vi.fn(),
    keyCheck: vi.fn(),
    localModelRemove: vi.fn(),
    openExternal: vi.fn(),
    providerModels: vi.fn(),
    setApiKey: vi.fn(),
    setApiKeySync: vi.fn(),
    setImageSearchKey: vi.fn(),
    setPrompt: vi.fn(),
    setProviderBudget: vi.fn(),
    setRoutingSettings: vi.fn(),
    setSummaryInstructions: vi.fn(),
    usageRows: vi.fn(),
    whimbrelAiBuy: vi.fn(),
    whimbrelAiRefresh: vi.fn(),
    whimbrelAiStatus: vi.fn(),
    setTypesafeKey: vi.fn(),
    setTopicSortEnabled: vi.fn(),
    startTopicSort: vi.fn(),
    topicSortEstimate: vi.fn(),
    topicSortState: vi.fn(),
    make,
  };
  return api;
});

vi.mock("../../../../api", () => h);
vi.mock("../../jump", () => ({ useJump: () => false }));
vi.mock("../../../../l10n", async (actual) => ({ ...(await actual<typeof import("../../../../l10n")>()), useL10n: () => ({ locale: "en-US" }) }));
vi.mock("../../../../chat/localModel", () => ({
  localModelDownload: { use: () => ({ status: { installed: false, canOffer: true }, job: null, error: null }), setStatus: () => {} },
}));
vi.mock("../../../searchModelDownload", () => ({
  searchModelDownload: {
    use: () => ({ status: { dir: "C:\\Models\\search", installed: false, loaded: false }, job: null, error: null }),
    refresh: () => Promise.resolve(),
    start: () => Promise.resolve(null),
    cancel: () => {},
  },
}));

const { AiSection } = await import("../AiSection");

const props = { onSection: () => {}, onRetentionSaved: () => {}, onOpenAutomations: () => {} };
const call = (node: TNode, prop: string, ...args: unknown[]) => (node.props[prop] as (...a: unknown[]) => void)(...args);
const change = (node: TNode, value: string) => call(node, "onChange", { target: { value } });

/** Everything the page says or carries: words, and every string, number and boolean in every attribute. */
function everything(tree: Tree): string {
  const parts: string[] = [tree.text()];
  for (const n of tree.all(() => true)) {
    for (const [k, v] of Object.entries(n.props)) if (k !== "children" && (typeof v === "string" || typeof v === "number")) parts.push(`${k}=${v}`);
  }
  return parts.join("\n");
}

async function open(): Promise<Tree> {
  const tree = new Tree(() => <AiSection {...props} />).render();
  return tree.settle();
}

/** The Save button of the row that holds `field`. */
function saveOf(field: TNode): TNode {
  for (let n: TNode | undefined = field.parent; n; n = n.parent) {
    const saves: TNode[] = [];
    const walk = (c: TNode | string) => {
      if (typeof c === "string") return;
      if (c.tag === "button" && text(c).trim() === "Save") saves.push(c);
      c.children.forEach(walk);
    };
    walk(n);
    if (saves.length === 1) return saves[0];
  }
  throw new Error("no Save button");
}

beforeEach(() => {
  h.state = h.make();
  for (const v of Object.values(h)) if (vi.isMockFunction(v)) v.mockReset();
  h.aiSettings.mockImplementation(() => Promise.resolve(JSON.parse(JSON.stringify(h.state))));
  h.providerModels.mockResolvedValue([]);
  h.usageRows.mockResolvedValue([]);
  h.whimbrelAiStatus.mockResolvedValue({ signedIn: false, low: false, plus: false, allowanceMicros: 0 });
  h.whimbrelAiRefresh.mockResolvedValue({ signedIn: false, low: false, plus: false, allowanceMicros: 0 });
  h.topicSortState.mockResolvedValue({ enabled: false, key: null, keyRejected: false, localOnly: false, overBudget: false, started: false });
  h.setRoutingSettings.mockResolvedValue(undefined);
  h.keyCheck.mockResolvedValue({ message: "Key works.", models: [], status: "valid" });
  h.setProviderBudget.mockResolvedValue([]);
});

describe("Local-only mode comes first", () => {
  it("is the first control in the section, a switch named Local-only mode, with the anchor on its row once", async () => {
    const tree = await open();
    const controls = tree.all((n) => ["button", "input", "select", "textarea"].includes(n.tag));
    const first = controls[0];
    expect(first.props.role).toBe("switch");
    const name = tree.one((n) => n.props.id === first.props["aria-labelledby"], "the switch's name");
    expect(text(name)).toBe("Local-only mode");
    expect(tree.all((n) => n.props.id === "ai.localOnly")).toHaveLength(1);
  });

  it("writes the toggle through the same command, keeping the rest of the routing", async () => {
    const tree = await open();
    call(tree.all((n) => n.props.role === "switch")[0], "onClick");
    expect(h.setRoutingSettings).toHaveBeenCalledWith({ ...h.state.routing, localOnly: true });
  });

  it("has every anchor of the section in the page once", async () => {
    const tree = await open();
    for (const id of ["ai.localOnly", "ai.provider", "ai.whimbrelAI", "ai.keys", "ai.instructions", "ai.usage", "ai.typesafe", "ai.semanticSearch"]) {
      expect(tree.all((n) => n.props.id === id), id).toHaveLength(1);
    }
  });
});

describe("a saved key is never drawn", () => {
  it("is not in the page on first read (the shell's masked form is not shown either)", async () => {
    h.state.imageSearchKey = SECRET;
    const tree = await open();
    expect(everything(tree)).not.toContain(SECRET);
    // It says a key is held, and offers Replace and Remove.
    expect(tree.all((n) => n.tag === "button" && text(n).trim() === "Replace").length).toBeGreaterThan(0);
    expect(tree.all((n) => n.tag === "button" && text(n).trim() === "Remove").length).toBeGreaterThan(0);
  });

  it("Save sends the trimmed value to the same command, then the key is nowhere, and a reload still shows none", async () => {
    h.setApiKey.mockImplementation((provider: string) => {
      h.state.keys = h.state.keys.map((k) => (k.provider === provider ? { ...k, masked: SECRET } : k));
      return Promise.resolve(h.state.keys);
    });
    const tree = await open();
    const field = tree.field("Anthropic API key");
    expect(field.props.type).toBe("password");
    change(field, `  ${SECRET} `);
    tree.render();
    call(saveOf(tree.field("Anthropic API key")), "onClick");
    expect(h.setApiKey).toHaveBeenCalledWith("anthropic", SECRET);
    await tree.settle();
    expect(h.keyCheck).toHaveBeenCalledWith("anthropic");
    expect(everything(tree)).not.toContain(SECRET);
    expect(tree.has("Key works.")).toBe(true);
    // After a reload the shell answers with the key held: still not drawn.
    const again = await open();
    expect(everything(again)).not.toContain(SECRET);
  });

  it("Replace, then Save: a refused save keeps what was typed; a taken one closes the box and the key is nowhere in the page", async () => {
    const NEW = "sk-new-example-not-a-key";
    const tree = await open();
    const isOpen = () => {
      try {
        tree.field("OpenAI API key");
        return true;
      } catch {
        return false;
      }
    };
    const box = () => tree.field("OpenAI API key");
    expect(isOpen()).toBe(false);
    call(tree.all((n) => n.tag === "button" && text(n).trim() === "Replace")[0], "onClick");
    tree.render();
    expect(isOpen()).toBe(true);
    change(box(), NEW);
    tree.render();
    h.setApiKey.mockRejectedValueOnce(new Error("refused"));
    call(saveOf(box()), "onClick");
    await tree.settle();
    expect(h.setApiKey).toHaveBeenCalledTimes(1);
    expect(box().props.value).toBe(NEW);
    h.setApiKey.mockResolvedValueOnce(h.state.keys);
    call(saveOf(box()), "onClick");
    await tree.settle();
    expect(h.setApiKey).toHaveBeenLastCalledWith("openai", NEW);
    expect(isOpen()).toBe(false);
    expect(everything(tree)).not.toContain(NEW);
  });

  it("Return in the box saves it too, and only a non-empty value", async () => {
    h.setApiKey.mockResolvedValue(h.state.keys);
    const tree = await open();
    const field = tree.field("Anthropic API key");
    press(field, "Enter", { tag: "input", type: "password" });
    expect(h.setApiKey).not.toHaveBeenCalled();
    change(field, SECRET);
    tree.render();
    press(tree.field("Anthropic API key"), "Enter", { tag: "input", type: "password" });
    expect(h.setApiKey).toHaveBeenCalledWith("anthropic", SECRET);
  });

  it("the remove question names the provider, and says the key also goes from other computers only when sync is on for it", async () => {
    h.state.keys = h.state.keys.map((k) => (k.provider === "openai" ? { ...k, syncEnabled: true } : k));
    const tree = await open();
    call(tree.all((n) => n.tag === "button" && text(n).trim() === "Remove")[0], "onClick");
    tree.render();
    expect(tree.one((n) => n.props.role === "group" && String(n.props["aria-label"]).startsWith("Remove the")).props["aria-label"]).toBe(
      "Remove the OpenAI key from this PC? Sync is on for this key, so it is also removed from your other computers.",
    );
  });

  it("Remove asks first, then calls the same command with no key", async () => {
    h.setApiKey.mockResolvedValue(h.state.keys.map((k) => ({ ...k, masked: null })));
    const tree = await open();
    const removes = () => tree.all((n) => n.tag === "button" && text(n).trim() === "Remove");
    call(removes()[0], "onClick");
    tree.render();
    expect(h.setApiKey).not.toHaveBeenCalled();
    expect(tree.one((n) => n.props.role === "group" && n.props["aria-label"] === "Remove the OpenAI key from this PC?")).toBeTruthy();
    const confirm = tree.one((n) => n.tag === "button" && String(n.props.className).includes("destructive") && text(n).trim() === "Remove");
    call(confirm, "onClick");
    expect(h.setApiKey).toHaveBeenCalledWith("openai", null);
  });
});

describe("number rows keep the old rules", () => {
  it("a bad monthly budget says why and calls nothing", async () => {
    const tree = await open();
    const field = () => tree.field("Monthly budget");
    for (const bad of ["abc", "-3", "1.234"]) {
      change(field(), bad);
      tree.render();
      call(field(), "onBlur");
      tree.render();
      expect(h.setProviderBudget, bad).not.toHaveBeenCalled();
      expect(tree.has("dollar amount") || tree.has("Enter a number"), bad).toBe(true);
    }
  });

  it("a good budget writes dollars; 0 clears it", async () => {
    const tree = await open();
    const field = () => tree.field("Monthly budget");
    change(field(), "12.5");
    tree.render();
    call(field(), "onBlur");
    expect(h.setProviderBudget).toHaveBeenCalledWith("openai", 12.5);
    change(field(), "0");
    tree.render();
    call(field(), "onBlur");
    // 0 means no budget, which removes a limit: it asks first.
    expect(h.setProviderBudget).toHaveBeenCalledTimes(1);
    tree.render();
    call(tree.button("Remove the budget"), "onClick");
    expect(h.setProviderBudget).toHaveBeenLastCalledWith("openai", null);
  });

  it("ticking No budget asks first, and Cancel leaves the budget alone", async () => {
    const tree = await open();
    const tick = () => tree.one((n) => n.tag === "input" && n.props.type === "checkbox" && n.parent?.tag === "label" && text(n.parent).includes("No budget"), "No budget");
    call(tick(), "onChange", { target: { checked: true } });
    tree.render();
    expect(h.setProviderBudget).not.toHaveBeenCalled();
    expect(tree.has("no monthly limit on spending for OpenAI")).toBe(true);
    call(tree.button("Cancel"), "onClick");
    tree.render();
    expect(h.setProviderBudget).not.toHaveBeenCalled();
    call(tick(), "onChange", { target: { checked: true } });
    tree.render();
    call(tree.button("Remove the budget"), "onClick");
    expect(h.setProviderBudget).toHaveBeenCalledWith("openai", null);
  });

  it("money rows refuse 1,000 instead of saving 1.00", async () => {
    const tree = await open();
    const field = () => tree.field("Monthly budget");
    change(field(), "1,000");
    tree.render();
    call(field(), "onBlur");
    tree.render();
    expect(h.setProviderBudget).not.toHaveBeenCalled();
    expect(tree.has("dollar amount")).toBe(true);
    change(field(), "$30");
    tree.render();
    call(field(), "onBlur");
    expect(h.setProviderBudget).toHaveBeenCalledWith("openai", 30);
  });

  it("the daily cap choice changes only on Space or Return: an arrow moves focus and writes nothing", async () => {
    const tree = await open();
    const group = tree.one((n) => n.props.role === "radiogroup" && n.props["aria-label"] === "Your daily cap");
    const builtin = group.children.find((c) => typeof c !== "string" && text(c).trim() === "Built-in") as TNode;
    press(builtin, "ArrowLeft");
    press(builtin, "ArrowRight");
    expect(h.setRoutingSettings).not.toHaveBeenCalled();
    expect(tree.all((n) => n.props.role === "radio" && n.parent === group && n.props["aria-checked"] === true).map((n) => text(n).trim())).toEqual(["Built-in"]);
  });

  it("a daily cap: built-in writes null, off writes 0, a bad amount writes nothing", async () => {
    const tree = await open();
    const radio = (group: string, label: string) =>
      tree.one((n) => n.tag === "button" && n.props.role === "radio" && text(n).trim() === label && n.parent?.props["aria-label"] === group, `${label} of ${group}`);
    call(radio("Your daily cap", "Off"), "onClick");
    // Off removes the limit on spending: nothing is written until the question is answered.
    expect(h.setRoutingSettings).not.toHaveBeenCalled();
    tree.render();
    expect(tree.has("no daily limit on spending")).toBe(true);
    call(tree.button("Turn off the cap"), "onClick");
    expect(h.setRoutingSettings).toHaveBeenLastCalledWith({ ...h.state.routing, userDailyCapCents: 0 });
    h.setRoutingSettings.mockClear();
    // The app-wide cap holds 5.00: choose an amount, type junk, then a real one.
    expect(radio("App-wide daily cap", "Set an amount").props["aria-checked"]).toBe(true);
    const amount = () => tree.field("App-wide daily cap amount");
    change(amount(), "2.505");
    tree.render();
    call(amount(), "onBlur");
    tree.render();
    expect(h.setRoutingSettings).not.toHaveBeenCalled();
    expect(tree.has("dollar amount")).toBe(true);
    change(amount(), "7.25");
    tree.render();
    call(amount(), "onBlur");
    expect(h.setRoutingSettings).toHaveBeenCalledWith({ ...h.state.routing, userDailyCapCents: 0, globalDailyCapCents: 725 });
    call(radio("App-wide daily cap", "Built-in"), "onClick");
    expect(h.setRoutingSettings).toHaveBeenLastCalledWith(expect.objectContaining({ globalDailyCapCents: null }));
  });
});

describe("typing 0 as a daily cap amount", () => {
  it.each(["0", "0.00", "$0"])("%s asks the no-daily-limit question first; Cancel keeps the cap, Confirm turns it off", async (typed) => {
    const tree = await open();
    const amount = () => tree.field("App-wide daily cap amount");
    change(amount(), typed);
    tree.render();
    call(amount(), "onBlur");
    expect(h.setRoutingSettings).not.toHaveBeenCalled();
    tree.render();
    expect(tree.has("no daily limit on spending")).toBe(true);
    call(tree.button("Cancel"), "onClick");
    tree.render();
    expect(h.setRoutingSettings).not.toHaveBeenCalled();
    expect(tree.has("no daily limit on spending")).toBe(false);
    // The previous cap (5.00) is still what the row shows and what is stored.
    expect(tree.field("App-wide daily cap amount").props.value).toBe("5");
    change(amount(), typed);
    tree.render();
    call(amount(), "onBlur");
    tree.render();
    call(tree.button("Turn off the cap"), "onClick");
    expect(h.setRoutingSettings).toHaveBeenLastCalledWith({ ...h.state.routing, globalDailyCapCents: 0 });
  });
});

describe("a provider's month against its budget", () => {
  it("under budget: the bar and the amount, no problem chip", async () => {
    const tree = await open();
    expect(tree.has("$2.50 of $10.00 (25%)")).toBe(true);
    expect(tree.has("Over budget")).toBe(false);
    expect(tree.all((n) => String(n.props.className ?? "").includes("w-progress-over"))).toHaveLength(0);
  });
  it("at or over budget: a problem chip with words, and the bar says so too", async () => {
    h.state.usage.monthByProvider = [{ provider: "openai", cents: 1200 }];
    const tree = await open();
    expect(tree.has("$12.00 of $10.00 (120%)")).toBe(true);
    expect(tree.has("Over budget")).toBe(true);
    expect(tree.has("This month's spending has reached the budget.")).toBe(true);
    expect(tree.all((n) => String(n.props.className ?? "").includes("w-status-problem"))).toHaveLength(1);
    expect(tree.all((n) => String(n.props.className ?? "").includes("w-progress-over"))).toHaveLength(1);
  });
});

describe("the model box belongs to its provider", () => {
  it("after a blank-model refusal, switching provider shows the new provider's own model and no stale error", async () => {
    const tree = await open();
    const box = () => tree.field("Ollama (on this PC) model");
    change(box(), "");
    tree.render();
    call(box(), "onBlur");
    tree.render();
    expect(tree.has("Enter a model name.")).toBe(true);
    const select = tree.one((n) => n.tag === "select" && n.props.value === "ollama", "provider select");
    call(select, "onChange", { target: { value: "anthropic" } });
    await tree.settle();
    expect(tree.has("Enter a model name.")).toBe(false);
    expect(tree.field("Anthropic model").props.value).toBe("a1");
  });
});

describe("the instructions", () => {
  it("save the summary text through the same command only when it changed", async () => {
    const tree = await open();
    const field = () => tree.field("Every summary");
    call(field(), "onBlur");
    expect(h.setSummaryInstructions).not.toHaveBeenCalled();
    h.setSummaryInstructions.mockResolvedValue(undefined);
    change(field(), "Use bullets. Name owners.");
    tree.render();
    call(field(), "onBlur");
    expect(h.setSummaryInstructions).toHaveBeenCalledWith("Use bullets. Name owners.");
  });
});

describe("the words", () => {
  it("name the section AI and show no banned name", async () => {
    h.state.keys = h.state.keys.map((k) => ({ ...k, masked: null }));
    const tree = await open();
    const all = tree.text();
    expect(hasBannedPhrase(all)).toBe(false);
    expect(all).toContain("API keys");
    expect(tree.all((n) => /^h[2-4]$/.test(n.tag)).map((n) => text(n))).toContain("AI");
    // With nothing set up, the on-ramp says what works without a model, in the new names.
    expect(tree.has("Pick where AI runs.")).toBe(true);
    expect(all).not.toMatch(/capture/i);
  });
});
