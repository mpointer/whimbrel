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
  plan: { plan: "free", untilUnixMs: null, interval: null, billing: true },
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
    const cloud = view({ plan: { plan: "cloud", untilUnixMs: Date.UTC(2027, 10, 9, 12), interval: "year", billing: true } });
    const html = renderToStaticMarkup(<Plan view={cloud} busy={false} act={noop} error={stripe} />);
    expect(html.indexOf("&#x27;t reach Stripe")).toBeGreaterThan(html.indexOf("Manage subscription"));
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
