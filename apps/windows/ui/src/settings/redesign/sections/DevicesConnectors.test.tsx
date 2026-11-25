import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, text, type TNode } from "../../../testkit/tree";

// The Sync & Devices and Connectors sections, drawn from the testkit's stand-in tree. The shell is faked at its one
// door, `invoke`, so what is checked is the Tauri commands and their arguments, which must be what the old cards send.
vi.mock("react", async (actual) => {
  const mod = (await import("../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>());
  // useContext reads the context's default: the frame's JumpContext is null here, so nothing is "jumped to".
  const useContext = (c: { _currentValue: unknown }) => c._currentValue;
  return { ...mod, useContext, default: { ...(mod as unknown as { default: object }).default, useContext } };
});
vi.mock("../../../assets/mobile-beta-qr.svg", () => ({ default: "qr.svg" }));

type Answer = unknown | ((args: unknown) => unknown);
const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], answers: {} as Record<string, unknown>, clip: [] as string[] }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    if (!(cmd in h.answers)) return null;
    const a = h.answers[cmd] as Answer;
    return typeof a === "function" ? (a as (x: unknown) => unknown)(args) : a;
  },
}));

const { DevicesSection } = await import("./DevicesSection");
const { ConnectorsSection } = await import("./ConnectorsSection");
const { McpTokenHost } = await import("./connectors/McpTokenHost");
const { mcpToken } = await import("./connectors/mcpTokenStore");
const props = { onSection: () => {}, onRetentionSaved: () => {}, onOpenAutomations: () => {} };

const ME = { id: "d-me", publicKey: "PUBKEY-ME-0000000", name: "Studio PC", status: "approved", thisDevice: true, platform: "windows", enrolledAtUnixMs: 1, comparisonCode: null };
const LAPTOP = { id: "d-lap", publicKey: "PUBKEY-LAP-000000", name: "Laptop", status: "approved", thisDevice: false, platform: "windows", enrolledAtUnixMs: 2, comparisonCode: null };
const NEW = { id: "d-new", publicKey: "PUBKEY-NEW-000000", name: "New tablet", status: "pending", thisDevice: false, platform: "windows", enrolledAtUnixMs: 3, comparisonCode: "4821" };
const NEW2 = { ...NEW, id: "d-new2", publicKey: "PUBKEY-NEW2-00000", name: "Other tablet" };
const READY = { state: "ready", message: null, deviceId: "k", deviceLimit: false, planUnknown: false, plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false } };

beforeEach(() => {
  mcpToken.clear();
  h.calls = [];
  h.clip = [];
  h.answers = {
    sync_status: READY,
    sync_refresh_plan: READY,
    sync_devices: [ME, LAPTOP, NEW],
    routines_cloud: { cloud_ui_enabled: false, runner_last_run_ms: null },
    client_pending_devices: { pending_devices: 0 },
    client_pending_device_ids: { devices: [] },
    calendar_accounts: { accessIssues: [], blocked: null, emails: ["a@example.test"], signInExpired: [], upcoming: 0 },
    gmail_accounts: { accounts: [{ email: "g@example.test", needsReconnect: false, sendOnly: false, signInExpired: false }], blocked: null },
    google_client_status: { clientId: null },
    google_removal_revokes: true,
    mcp_server_state: { enabled: true, status: "running", address: "127.0.0.1:4777", error: null },
    mcp_clients: [],
  };
  vi.stubGlobal("window", { addEventListener() {}, removeEventListener() {}, setInterval: () => 0, clearInterval() {}, dispatchEvent: () => true });
  vi.stubGlobal("setInterval", () => 0);
  vi.stubGlobal("clearInterval", () => {});
  vi.stubGlobal("document", { activeElement: null, getElementById: () => null });
  vi.stubGlobal("navigator", { clipboard: { writeText: (s: string) => void h.clip.push(s) } });
});

const settleAll = async (tree: Tree) => {
  for (let i = 0; i < 4; i++) await tree.settle();
  return tree;
};
const show = async (C: () => unknown) => settleAll(new Tree(C as () => unknown).render());
const commands = () => h.calls.map(([c]) => c);
const sent = (cmd: string) => h.calls.filter(([c]) => c === cmd).map(([, a]) => a);
const click = (b: TNode) => (b.props.onClick as () => void)();
/** The button with this exact accessible name, or this exact text when it has none. */
const named = (tree: Tree, name: string) => tree.one((n) => n.tag === "button" && (n.props["aria-label"] ?? text(n).trim()) === name, `button "${name}"`);
const ids = (tree: Tree, id: string) => tree.all((n) => n.props.id === id);
/** True when `needle` is anywhere the page could show it: a text, a value, an attribute. */
const holds = (tree: Tree, needle: string) =>
  tree.text().includes(needle) || tree.all((n) => Object.values(n.props).some((v) => typeof v === "string" && v.includes(needle))).length > 0;
const dialog = (tree: Tree) => tree.all((n) => n.props.role === "alertdialog")[0];

describe("anchors", () => {
  it("every search anchor of Sync & Devices is on the page once, with sync on", async () => {
    const tree = await show(() => <DevicesSection {...props} />);
    for (const id of ["devices.sync", "devices.list", "devices.phone"]) expect(ids(tree, id), id).toHaveLength(1);
  });

  it("devices.phone is still there once a phone is paired, and says so", async () => {
    h.answers.sync_devices = [ME, { ...LAPTOP, platform: "ios" }];
    const tree = await show(() => <DevicesSection {...props} />);
    expect(ids(tree, "devices.phone")).toHaveLength(1);
    expect(tree.has("Phone: connected")).toBe(true);
    expect(tree.has("Join the beta")).toBe(false);
  });

  it("with sync off there is the sync card and the phone card, and no device list (as the old tab)", async () => {
    h.answers.sync_status = { ...READY, state: "notSignedIn" };
    h.answers.sync_refresh_plan = { ...READY, state: "notSignedIn" };
    const tree = await show(() => <DevicesSection {...props} />);
    expect(ids(tree, "devices.sync")).toHaveLength(1);
    expect(ids(tree, "devices.phone")).toHaveLength(1);
    expect(ids(tree, "devices.list")).toHaveLength(0);
    expect(tree.hasButton("Enable Sync (sign in with Google)")).toBe(true);
    expect(tree.has("Off")).toBe(true);
  });

  it("every search anchor of Connectors is on the page once", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    for (const id of ["connectors.accounts", "connectors.gmail", "connectors.ownClient", "connectors.mcp"]) expect(ids(tree, id), id).toHaveLength(1);
  });

  it("the own-client block sits inside a disclosure titled Advanced", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    const disc = tree.one((n) => n.props.className === "w-disc");
    expect(text(disc)).toContain("Advanced");
    expect(tree.all((n) => n.props.id === "connectors.ownClient" && n.parent !== undefined)).toHaveLength(1);
    const owner = ids(tree, "connectors.ownClient")[0];
    let p: TNode | undefined = owner;
    while (p && p !== disc) p = p.parent;
    expect(p).toBe(disc);
  });
});

describe("Sync", () => {
  it("signs in with the same command", async () => {
    h.answers.sync_status = { ...READY, state: "notSignedIn" };
    h.answers.sync_refresh_plan = { ...READY, state: "notSignedIn" };
    const tree = await show(() => <DevicesSection {...props} />);
    click(tree.button("Enable Sync (sign in with Google)"));
    expect(commands()).toContain("sync_sign_in");
  });

  it("at the device limit offers the same upgrade commands", async () => {
    h.answers.sync_status = { ...READY, state: "error", deviceLimit: true };
    h.answers.sync_refresh_plan = { ...READY, state: "error", deviceLimit: true };
    const tree = await show(() => <DevicesSection {...props} />);
    expect(tree.has("Upgrade to Cloud for unlimited devices.")).toBe(true);
    click(tree.button("Upgrade to Cloud: $5 a month"));
    click(tree.button("Upgrade to Cloud: $50 a year"));
    expect(sent("sync_upgrade")).toEqual([{ interval: "month" }, { interval: "year" }]);
  });
});

describe("devices", () => {
  it("revoke asks first, in the old words, then sends the same command", async () => {
    const tree = await show(() => <DevicesSection {...props} />);
    click(named(tree, "Revoke…, Laptop"));
    tree.render();
    expect(commands()).not.toContain("sync_revoke_device");
    expect(tree.has("Revoking signs every device out of sync until it re-authenticates.")).toBe(true);
    // The safe button has the focus's place and Keep puts everything back without a command.
    click(tree.button("Keep"));
    tree.render();
    expect(commands()).not.toContain("sync_revoke_device");
    click(named(tree, "Revoke…, Laptop"));
    tree.render();
    click(tree.button("Revoke"));
    expect(sent("sync_revoke_device")).toEqual([{ id: "d-lap" }]);
  });

  it("removing Whimbrel Cloud asks with its own words", async () => {
    h.answers.sync_devices = [ME, { ...LAPTOP, id: "d-run", name: null, platform: "cloud" }];
    const tree = await show(() => <DevicesSection {...props} />);
    click(named(tree, "Remove, Whimbrel Cloud"));
    tree.render();
    expect(commands()).not.toContain("sync_revoke_device");
    expect(tree.has("Remove Whimbrel Cloud?")).toBe(true);
    click(tree.button("Remove"));
    expect(sent("sync_revoke_device")).toEqual([{ id: "d-run" }]);
  });

  it("renames this PC with the old command and name", async () => {
    const tree = await show(() => <DevicesSection {...props} />);
    const box = tree.one((n) => n.tag === "input" && n.props["aria-label"] === "This PC's name");
    expect(box.props.value).toBe("Studio PC");
    (box.props.onChange as (e: unknown) => void)({ target: { value: "Desk" } });
    tree.render();
    const form = tree.one((n) => n.tag === "form");
    (form.props.onSubmit as (e: unknown) => void)({ preventDefault() {} });
    expect(sent("sync_set_device_name")).toEqual([{ name: "Desk" }]);
  });

  it("approves a pending device after showing its code, and says what to check", async () => {
    const tree = await show(() => <DevicesSection {...props} />);
    expect(tree.has("Check this matches the code on the new device: 4821")).toBe(true);
    click(named(tree, "Approve, New tablet"));
    expect(sent("sync_approve_device")).toEqual([{ id: "d-new" }]);
  });

  it("does not let two pending devices with one code be approved, and says why", async () => {
    h.answers.sync_devices = [ME, NEW, NEW2];
    const tree = await show(() => <DevicesSection {...props} />);
    const approve = named(tree, "Approve, New tablet");
    expect(approve.props.disabled).toBe(true);
    // Not the default-looking button while the codes collide, and tied to the warning that says why.
    expect(String(approve.props.className)).not.toContain("w-button-primary");
    expect(tree.has("Another pending device shows the same code. Do not approve it.")).toBe(true);
    const warning = tree.one((n) => n.props.id === approve.props["aria-describedby"], "warning");
    expect(String(warning.props.className)).toBe("w-lrow-detail");
  });

  it("shows the comparison code at the primary size, not in the muted caption", async () => {
    const tree = await show(() => <DevicesSection {...props} />);
    const code = tree.one((n) => n.tag === "span" && String(n.props.className) === "w-lrow-detail" && text(n).includes("4821"), "code line");
    expect(text(code)).toBe("Check this matches the code on the new device: 4821");
    expect(tree.all((n) => String(n.props.className) === "w-lrow-caption" && text(n).includes("4821"))).toEqual([]);
    expect(String(named(tree, "Approve, New tablet").props.className)).toContain("w-button-primary");
  });

  it("removing a retired device that still needs to update asks first", async () => {
    h.answers.client_pending_devices = { pending_devices: 1 };
    h.answers.client_pending_device_ids = { devices: ["PUBKEY-LAP-000000"] };
    h.answers.client_retire_device = { ok: true, data: { pending_devices: 0 } };
    const tree = await show(() => <DevicesSection {...props} />);
    click(tree.button("Remove this device…"));
    tree.render();
    expect(commands()).not.toContain("client_retire_device");
    click(tree.button("Remove"));
    expect(sent("client_retire_device")).toEqual([{ deviceId: "PUBKEY-LAP-000000" }]);
  });

  it("the phone card opens the same page with the same command", async () => {
    h.answers.sync_devices = [ME];
    const tree = await show(() => <DevicesSection {...props} />);
    expect(tree.all((n) => n.tag === "img" && n.props.alt === "QR code for the phone beta page")).toHaveLength(1);
    const link = tree.one((n) => n.tag === "a" && String(n.props.href).startsWith("https://whimbrelapp.com/mobile-beta"));
    (link.props.onClick as (e: unknown) => void)({ preventDefault() {} });
    expect(sent("open_external")).toEqual([{ url: "https://whimbrelapp.com/mobile-beta/" }]);
  });
});

describe("Connectors accounts", () => {
  it("disconnecting a calendar asks first and waits for the check of what removal does", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    click(named(tree, "Disconnect, a@example.test"));
    tree.render();
    expect(dialog(tree)).toBeDefined();
    expect(commands()).not.toContain("calendar_disconnect");
    expect(sent("google_removal_revokes")).toEqual([{ service: "calendar", email: "a@example.test" }]);
    await settleAll(tree);
    expect(tree.has("revokes Whimbrel's access to the account with Google")).toBe(true);
    click(tree.button("Cancel"));
    tree.render();
    expect(commands()).not.toContain("calendar_disconnect");
  });

  it("disconnecting sends the same command after the confirmation", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    click(named(tree, "Disconnect, a@example.test"));
    await settleAll(tree);
    click(tree.button("Remove"));
    expect(sent("calendar_disconnect")).toEqual([{ email: "a@example.test" }]);
  });

  it("deleting stored Gmail mail asks, and offers delete, delete and disconnect, or cancel", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    click(named(tree, "Delete stored mail…, g@example.test"));
    tree.render();
    expect(commands()).not.toContain("gmail_delete_stored_mail");
    expect(tree.has("Delete all mail Whimbrel stored from g@example.test?")).toBe(true);
    click(tree.button("Delete mail and disconnect"));
    await settleAll(tree);
    expect(commands().filter((c) => c === "gmail_delete_stored_mail" || c === "gmail_disconnect")).toEqual(["gmail_delete_stored_mail", "gmail_disconnect"]);
  });

  it("a blocked connect says why in words and is off", async () => {
    h.answers.gmail_accounts = { accounts: [], blocked: "No Google client is set up." };
    const tree = await show(() => <ConnectorsSection {...props} />);
    expect(tree.has("No Google client is set up.")).toBe(true);
    expect(tree.button("Connect Gmail").props.disabled).toBe(true);
  });
});

const SECRET = "GOCSPX-never-draw-me-123";
describe("your own Google Cloud project", () => {
  const typeInto = (tree: Tree, field: TNode, value: string) => {
    (field.props.onChange as (e: unknown) => void)({ target: { value } });
    tree.render();
  };

  it("saves the pair with the old command, and the secret is nowhere on the page afterwards", async () => {
    h.answers.google_client_save = { clientId: "abc.apps.example.test" };
    const tree = await show(() => <ConnectorsSection {...props} />);
    // Nothing can be saved without the Client ID, and the row says why.
    expect(tree.has("Enter the Client ID first")).toBe(true);
    typeInto(tree, tree.field("Client ID"), "abc.apps.example.test");
    (tree.field("Client ID").props.onBlur as () => void)();
    tree.render();
    const secret = tree.one((n) => n.tag === "input" && n.props.type === "password");
    typeInto(tree, secret, SECRET);
    expect(holds(tree, SECRET)).toBe(true); // while it is being typed, it is in the box
    click(tree.button("Save"));
    await settleAll(tree);
    expect(sent("google_client_save")).toEqual([{ clientId: "abc.apps.example.test", clientSecret: SECRET }]);
    expect(holds(tree, SECRET)).toBe(false);
    expect(tree.all((n) => n.tag === "input" && n.props.type === "password")).toHaveLength(0);
    expect(tree.has("Saved")).toBe(true);
    expect(tree.hasButton("Replace")).toBe(true);
    expect(tree.hasButton("Remove")).toBe(true);
  });

  it("a saved client never draws its secret, and Remove clears with the old command", async () => {
    h.answers.google_client_status = { clientId: "abc.apps.example.test" };
    h.answers.google_client_clear = { clientId: null };
    const tree = await show(() => <ConnectorsSection {...props} />);
    expect(tree.all((n) => n.tag === "input" && n.props.type === "password")).toHaveLength(0);
    expect(tree.has("Saved")).toBe(true);
    click(tree.button("Remove"));
    // It signs every connected Google account out, so it asks first.
    tree.render();
    expect(commands()).not.toContain("google_client_clear");
    expect(tree.has("Every connected Google account is signed out")).toBe(true);
    click(tree.one((n) => n.tag === "button" && String(n.props.className).includes("destructive") && text(n).trim() === "Remove"));
    expect(commands()).toContain("google_client_clear");
  });

  it("is open when a client is saved and closed when none is", async () => {
    const closed = await show(() => <ConnectorsSection {...props} />);
    expect(closed.one((n) => n.props.className === "w-disc").children.some((c) => typeof c !== "string" && c.tag === "button" && c.props["aria-expanded"] === false)).toBe(true);
    h.answers.google_client_status = { clientId: "abc.apps.example.test" };
    const open = await show(() => <ConnectorsSection {...props} />);
    expect(open.one((n) => n.props.className === "w-disc").children.some((c) => typeof c !== "string" && c.tag === "button" && c.props["aria-expanded"] === true)).toBe(true);
  });
});

describe("a standing notice and an action's error are both shown", () => {
  it("Connected accounts keeps the blocked notice and adds the failed action's error", async () => {
    h.answers.calendar_accounts = { accessIssues: [], blocked: "Google sign-in is not set up in this build.", emails: ["a@example.test"], signInExpired: [], upcoming: 0 };
    h.answers.calendar_sync_now = () => {
      throw { kind: "network", message: "Sync could not reach Google." };
    };
    const tree = await show(() => <ConnectorsSection {...props} />);
    click(tree.button("Sync now"));
    await settleAll(tree);
    expect(tree.has("Google sign-in is not set up in this build.")).toBe(true);
    expect(tree.has("Sync could not reach Google.")).toBe(true);
  });
});

const TOKEN = "wmcp_one-time-token-0123456789";
/** The Connectors section as the frame draws it: the section, and beside it the token host that lives above the section switch. */
const WithHost = () => (
  <>
    <ConnectorsSection {...props} />
    <McpTokenHost />
  </>
);
describe("AI assistants (MCP)", () => {
  const make = async () => {
    h.answers.mcp_create_client = { id: 7, token: TOKEN };
    const tree = await show(() => <WithHost />);
    const name = tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Assistant name");
    (name.props.onChange as (e: unknown) => void)({ target: { value: " Claude Desktop " } });
    tree.render();
    const memory = tree.all((n) => n.tag === "input" && n.props.type === "checkbox")[0];
    (memory.props.onChange as (e: unknown) => void)({ target: { checked: true } });
    tree.render();
    return tree;
  };

  it("Create token is off with a reason until there is a name and a permission", async () => {
    h.answers.mcp_create_client = { id: 7, token: TOKEN };
    const tree = await show(() => <ConnectorsSection {...props} />);
    expect(tree.button("Create token").props.disabled).toBe(true);
    expect(tree.has("Give the assistant a name.")).toBe(true);
  });

  it("shows the token once, inside a dialog, with a Copy button, and never after it is closed", async () => {
    const tree = await make();
    expect(holds(tree, TOKEN)).toBe(false);
    click(tree.button("Create token"));
    await settleAll(tree);
    expect(sent("mcp_create_client")).toEqual([{ name: "Claude Desktop", scopes: ["memory"] }]);
    const box = dialog(tree);
    expect(box).toBeDefined();
    expect(text(box)).toContain(TOKEN);
    expect(text(box)).toContain("This is the only time this token is shown.");
    // Every place the token appears in the page is inside that dialog.
    expect(tree.text().split(TOKEN).length).toBe(text(box).split(TOKEN).length);
    click(tree.button("Copy"));
    await settleAll(tree);
    expect(h.clip).toEqual([TOKEN]);
    expect(tree.has("Copied")).toBe(true);
    click(tree.button("Done"));
    tree.render();
    await settleAll(tree);
    expect(dialog(tree)).toBeUndefined();
    expect(holds(tree, TOKEN)).toBe(false);
    expect(tree.text()).not.toContain("Claude Desktop ");
  });

  const escape = (tree: Tree) =>
    (dialog(tree).props.onKeyDown as (e: unknown) => void)({ key: "Escape", preventDefault() {}, stopPropagation() {}, target: { tagName: "DIV", getAttribute: () => null }, nativeEvent: {} });

  it("Escape before the token was copied asks first and keeps the token; only the second answer drops it", async () => {
    const tree = await make();
    click(tree.button("Create token"));
    await settleAll(tree);
    escape(tree);
    tree.render();
    expect(holds(tree, TOKEN)).toBe(true);
    expect(tree.has("Close without copying? This token is not shown again.")).toBe(true);
    // Done asks the same before a copy.
    click(tree.button("Keep open"));
    tree.render();
    expect(tree.has("Close without copying?")).toBe(false);
    click(tree.button("Done"));
    tree.render();
    expect(holds(tree, TOKEN)).toBe(true);
    click(tree.button("Close without copying"));
    tree.render();
    expect(holds(tree, TOKEN)).toBe(false);
    expect(dialog(tree)).toBeUndefined();
  });

  it("after Copy worked, Escape closes at once", async () => {
    const tree = await make();
    click(tree.button("Create token"));
    await settleAll(tree);
    click(tree.button("Copy"));
    await settleAll(tree);
    escape(tree);
    tree.render();
    expect(holds(tree, TOKEN)).toBe(false);
  });

  it("says Could not copy, never Copied, when the clipboard refuses, and still asks before closing", async () => {
    vi.stubGlobal("navigator", { clipboard: { writeText: () => Promise.reject(new Error("denied")) } });
    const tree = await make();
    click(tree.button("Create token"));
    await settleAll(tree);
    click(tree.button("Copy"));
    await settleAll(tree);
    expect(tree.has("Could not copy")).toBe(true);
    expect(tree.has("Copied")).toBe(false);
    escape(tree);
    tree.render();
    expect(holds(tree, TOKEN)).toBe(true);
    expect(tree.has("Close without copying?")).toBe(true);
  });

  it("the only-time sentence is the dialog's description", async () => {
    const tree = await make();
    click(tree.button("Create token"));
    await settleAll(tree);
    const box = dialog(tree);
    const desc = tree.one((n) => n.props.id === box.props["aria-describedby"], "description");
    expect(text(desc)).toContain("This is the only time this token is shown.");
  });

  it("revoking an assistant asks first, then sends the same command", async () => {
    h.answers.mcp_clients = [{ id: 3, name: "Claude Desktop", scopes: ["memory"], createdUnixMs: 1_700_000_000_000, revokedUnixMs: null }];
    h.answers.mcp_revoke_client = [{ id: 3, name: "Claude Desktop", scopes: ["memory"], createdUnixMs: 1_700_000_000_000, revokedUnixMs: 1_700_000_100_000 }];
    const tree = await show(() => <ConnectorsSection {...props} />);
    click(named(tree, "Revoke…, Claude Desktop"));
    tree.render();
    expect(commands()).not.toContain("mcp_revoke_client");
    expect(tree.has("Revoke Claude Desktop? It loses access at once")).toBe(true);
    click(tree.button("Revoke"));
    expect(sent("mcp_revoke_client")).toEqual([{ id: 3, deleteItems: false }]);
  });

  it("the switch sends the same command", async () => {
    h.answers.mcp_set_enabled = { enabled: false, status: "stopped", address: "", error: null };
    const tree = await show(() => <ConnectorsSection {...props} />);
    const sw = tree.one((n) => n.props.role === "switch");
    click(sw);
    expect(sent("mcp_set_enabled")).toEqual([{ enabled: false }]);
  });

  it("a server that failed is said in words, with the reason", async () => {
    h.answers.mcp_server_state = { enabled: true, status: "failed", address: "", error: "port in use" };
    const tree = await show(() => <ConnectorsSection {...props} />);
    expect(tree.has("Not running: port in use")).toBe(true);
  });
});

describe("redesigned wording", () => {
  const BANNED = [/memory database/i, /activity capture/i, /capture & privacy/i, /AI & routing/, /\bChats\b/, /never delete/i, /keep full quality forever/i];
  it("uses none of the retired names on any state of either section", async () => {
    h.answers.client_pending_devices = { pending_devices: 1 };
    h.answers.client_pending_device_ids = { devices: ["PUBKEY-LAP-000000"] };
    h.answers.google_client_status = { clientId: "abc.apps.example.test" };
    h.answers.mcp_clients = [{ id: 3, name: "Claude Desktop", scopes: ["memory", "meetings", "write"], createdUnixMs: 1_700_000_000_000, revokedUnixMs: null }];
    h.answers.gmail_accounts = { accounts: [{ email: "g@example.test", needsReconnect: true, sendOnly: true, signInExpired: false, readError: null }], blocked: null };
    const sections = [await show(() => <DevicesSection {...props} />), await show(() => <ConnectorsSection {...props} />)];
    for (const tree of sections) for (const re of BANNED) expect(tree.text()).not.toMatch(re);
  });

  it("uses no em or en dash in anything it writes itself", async () => {
    const tree = await show(() => <ConnectorsSection {...props} />);
    // The one sentence kept from the old Sync card ("don't approve it") has a dash in its existing key; it is checked by its own test.
    expect(tree.text()).not.toMatch(/[\u2013\u2014]/);
  });
});

describe("the new token outlives a change of section", () => {
  /** Draws a frame with a section switch: Connectors, or another section, with the token host above the switch. */
  const frame = async () => {
    h.answers.mcp_create_client = { id: 7, token: TOKEN };
    const state = { section: "connectors" };
    const Frame = () => (
      <>
        {state.section === "connectors" ? <ConnectorsSection {...props} /> : <p>Another section</p>}
        <McpTokenHost />
      </>
    );
    const tree = await show(() => <Frame />);
    (tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Assistant name").props.onChange as (e: unknown) => void)({ target: { value: "Claude Desktop" } });
    tree.render();
    (tree.all((n) => n.tag === "input" && n.props.type === "checkbox")[0].props.onChange as (e: unknown) => void)({ target: { checked: true } });
    tree.render();
    click(tree.button("Create token"));
    await settleAll(tree);
    return { tree, go: (section: string) => { state.section = section; tree.render(); } };
  };

  it("a search jump or a shortcut to another section keeps the dialog and its token, and coming back shows it once", async () => {
    const { tree, go } = await frame();
    expect(text(dialog(tree))).toContain(TOKEN);
    go("meetings");
    expect(tree.has("Another section")).toBe(true);
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(1);
    expect(text(dialog(tree))).toContain(TOKEN);
    click(tree.button("Copy"));
    await settleAll(tree);
    go("connectors");
    await settleAll(tree);
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(1);
    expect(tree.text().split(TOKEN).length).toBe(2);
    // The copy that worked is remembered across the switch: Done closes at once.
    expect(tree.has("Copied")).toBe(true);
    click(tree.button("Done"));
    tree.render();
    expect(dialog(tree)).toBeUndefined();
    expect(holds(tree, TOKEN)).toBe(false);
    expect(mcpToken.get()).toBeNull();
  });

  it("when the whole Settings tab goes away before the token was copied, the token is dropped and the way back says to revoke it", async () => {
    const { tree } = await frame();
    expect(mcpToken.get()?.token).toBe(TOKEN);
    mcpToken.abandon();
    expect(mcpToken.get()).toBeNull();
    // Nothing the store still holds is the token.
    expect(JSON.stringify(mcpToken.orphan())).not.toContain(TOKEN);
    const back = await show(() => <WithHost />);
    expect(holds(back, TOKEN)).toBe(false);
    expect(back.has("A token was created for Claude Desktop but was not copied. Revoke it and create a new one.")).toBe(true);
    expect(tree).toBeTruthy();
    h.answers.mcp_revoke_client = [];
    click(back.button("Revoke"));
    await settleAll(back);
    expect(sent("mcp_revoke_client")).toEqual([{ id: 7, deleteItems: false }]);
    expect(back.has("A token was created for")).toBe(false);
    expect(mcpToken.orphan()).toBeNull();
  });

  it("a token that was copied leaves nothing behind when the tab goes away", async () => {
    const { tree } = await frame();
    click(tree.button("Copy"));
    await settleAll(tree);
    mcpToken.abandon();
    expect(mcpToken.get()).toBeNull();
    expect(mcpToken.orphan()).toBeNull();
    const back = await show(() => <WithHost />);
    expect(back.has("A token was created for")).toBe(false);
  });

  it("is never written to disk: no localStorage, no sessionStorage", async () => {
    const writes: string[] = [];
    const spy = { setItem: (k: string, v: string) => void writes.push(`${k}=${v}`), getItem: () => null, removeItem() {}, clear() {} };
    vi.stubGlobal("localStorage", spy);
    vi.stubGlobal("sessionStorage", spy);
    const { tree } = await frame();
    click(tree.button("Copy"));
    await settleAll(tree);
    mcpToken.abandon();
    expect(writes).toEqual([]);
  });
});
