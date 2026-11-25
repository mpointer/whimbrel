import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SyncView } from "../generated/types";
import { phoneCardState } from "./syncModel";

vi.mock("../assets/mobile-beta-qr.svg", () => ({ default: "qr.svg" }));
const { PhoneCard, PhoneConnected } = await import("./SyncCard");
const { LimitUpgrade } = await import("./PlanControls");
const { PlanBody } = await import("./PlanCard");
const Plan = (p: { view: SyncView; busy: boolean; act: () => void; error: string | null }) => (
  <PlanBody {...p} ai={null} onSignIn={() => {}} onRetry={() => {}} />
);

const view = (over: Partial<SyncView> = {}): SyncView => ({
  state: "ready",
  message: null,
  deviceId: "k",
  deviceLimit: false,
  planUnknown: false,
  plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false },
  ...over,
});
const noop = () => {};
const stripe = "Couldn't reach Stripe. Try again in a minute.";

describe("Upgrade and Manage errors sit under their buttons (Mac PR 673)", () => {
  it("in the Plan block, between the Upgrade buttons and the caption", () => {
    const html = renderToStaticMarkup(<Plan view={view()} busy={false} act={noop} error={stripe} />);
    const [buttons, error, caption] = ["Upgrade to Cloud: $50 a year", stripe.replace("'", "&#x27;"), "Plus tax"].map((s) => html.indexOf(s));
    expect(buttons).toBeGreaterThan(-1);
    expect(error).toBeGreaterThan(buttons);
    expect(caption).toBeGreaterThan(error);
  });

  it("under Manage subscription on Cloud", () => {
    const cloud = view({ plan: { plan: "cloud", untilUnixMs: Date.UTC(2027, 10, 9, 12), interval: "year", billing: true, plusAvailable: false } });
    const html = renderToStaticMarkup(<Plan view={cloud} busy={false} act={noop} error={stripe} />);
    expect(html.indexOf("&#x27;t reach Stripe")).toBeGreaterThan(html.indexOf("Manage subscription"));
  });

  it("shows only the Plus buttons when the server sells Plus, and only Cloud's otherwise", () => {
    const plusView = view({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: true } });
    const html = renderToStaticMarkup(<Plan view={plusView} busy={false} act={noop} error={null} />);
    expect(html).toContain("Get Plus: $15 a month");
    expect(html).toContain("Get Plus: $144 a year");
    expect(html).not.toContain("Upgrade to Cloud");
    const limit = renderToStaticMarkup(<LimitUpgrade view={{ ...plusView, deviceLimit: true }} busy={false} act={noop} error={null} />);
    expect(limit).toContain("Upgrade to Plus for unlimited devices.");
    expect(limit).not.toContain("Upgrade to Cloud");
    const cloudHtml = renderToStaticMarkup(<Plan view={view()} busy={false} act={noop} error={null} />);
    expect(cloudHtml).toContain("Upgrade to Cloud: $5 a month");
    expect(cloudHtml).not.toContain("Get Plus");
  });

  it("shows no purchase buttons on a paid plan, Cloud or Plus", () => {
    for (const planTier of ["cloud", "plus"]) {
      const paid = view({ plan: { plan: "cloud", untilUnixMs: null, interval: "month", billing: true, plusAvailable: true, planTier } });
      const html = renderToStaticMarkup(<Plan view={paid} busy={false} act={noop} error={null} />);
      expect(html).not.toContain("Get Plus");
      expect(html).not.toContain("Upgrade to Cloud");
    }
  });

  it("on the device-limit screen too", () => {
    const html = renderToStaticMarkup(<LimitUpgrade view={view({ deviceLimit: true })} busy={false} act={noop} error={stripe} />);
    expect(html.indexOf("&#x27;t reach Stripe")).toBeGreaterThan(html.indexOf("Upgrade to Cloud: $50 a year"));
  });
});

describe("the phone card", () => {
  it("has the Mac's text, the button and a labelled 84px QR code", () => {
    const html = renderToStaticMarkup(<PhoneCard />);
    for (const text of ["Whimbrel on your phone", "The apps are in beta.", "Join the beta", "Scan with your phone"]) expect(html).toContain(text);
    expect(html).toMatch(/<img src="qr.svg" width="84" height="84" alt="QR code for the phone beta page"/);
  });
});

describe("Plan & billing states", () => {
  const html = (over: Partial<SyncView>) => renderToStaticMarkup(<Plan view={view(over)} busy={false} act={noop} error={null} />);
  it("signed out asks to sign in, with no buying controls", () => {
    const h = html({ state: "notSignedIn", plan: null });
    expect(h).toContain("Sign in to sync to see your plan.");
    expect(h).toContain("Sign in to sync<");
    expect(h).not.toContain("Get Plus");
    expect(h).not.toContain("Upgrade to Cloud");
  });
  it("checking, couldn't check, and an unfinished sign-in", () => {
    expect(html({ plan: null })).toContain("Checking your plan…");
    expect(html({ plan: null, planUnknown: true })).toContain("Try again");
    const pending = html({ state: "waitingForApproval" });
    expect(pending).toContain("Finish signing in to manage your plan.");
    expect(pending).not.toContain("Upgrade to Cloud");
  });
  it("at the device limit shows the plan and its buttons", () => {
    expect(html({ state: "error", deviceLimit: true })).toContain("Upgrade to Cloud: $5 a month");
  });
});

describe("the phone card state", () => {
  it("is connected only for an approved phone", () => {
    expect(phoneCardState([{ status: "approved", platform: "ios" }])).toBe("connected");
    expect(phoneCardState([{ status: "approved", platform: "android" }])).toBe("connected");
    expect(phoneCardState([{ status: "pending", platform: "ios" }])).toBe("beta");
    expect(phoneCardState([{ status: "revoked", platform: "android" }])).toBe("beta");
    expect(phoneCardState([{ status: "approved", platform: "windows" }])).toBe("beta");
    expect(phoneCardState([])).toBe("beta");
    expect(renderToStaticMarkup(<PhoneConnected />)).toContain("Phone: connected");
  });
});
