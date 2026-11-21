import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SyncView } from "../generated/types";

vi.mock("../assets/mobile-beta-qr.svg", () => ({ default: "qr.svg" }));
const { LimitUpgrade, PhoneCard, Plan } = await import("./SyncCard");

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
