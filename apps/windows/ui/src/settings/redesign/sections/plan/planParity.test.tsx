import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SyncView, WhimbrelAiView } from "../../../../generated/types";
import { captionText, purchaseOffers } from "../../../PlanControls";
import { PlanBody } from "./PlanBody";

// The plan card draws its actions with the words and prices of `purchaseOffers` in every state, and states no other price.
vi.mock("../../../../assets/mobile-beta-qr.svg", () => ({ default: "qr.svg" }));

const sync = (over: Partial<SyncView> = {}): SyncView => ({
  state: "ready",
  message: null,
  deviceId: "k",
  deviceLimit: false,
  planUnknown: false,
  plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false },
  ...over,
});
const paid = (planTier: string) => sync({ plan: { plan: "cloud", untilUnixMs: Date.UTC(2027, 10, 9, 12), interval: "year", billing: true, plusAvailable: true, planTier } });
const ai = { signedIn: true, hasBalance: true } as unknown as WhimbrelAiView;
const noop = () => {};

const labels = (html: string) => [...html.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map((m) => m[1].replace(/<[^>]*>/g, "").trim());
const sentences = (html: string) => html.replace(/<[^>]*>/g, " ").replace(/&#x27;/g, "'").replace(/&amp;/g, "&");

const states: [string, SyncView][] = [
  ["signed out", sync({ state: "notSignedIn", plan: null })],
  ["not configured", sync({ state: "notConfigured", plan: null })],
  ["checking", sync({ plan: null })],
  ["could not check", sync({ plan: null, planUnknown: true })],
  ["finish signing in", sync({ state: "waitingForGroupKey", plan: null })],
  ["free, Cloud on sale", sync()],
  ["free, Plus on sale", sync({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: true } })],
  ["free, billing closed", sync({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: false, plusAvailable: true } })],
  ["Cloud", paid("cloud")],
  ["Plus", paid("plus")],
  ["device limit, free", sync({ state: "error", deviceLimit: true })],
];

describe("Plan & billing draws the offers", () => {
  for (const [name, view] of states) {
    it(`${name}: ends with the AI settings button, and offers only what is on sale`, () => {
      const now = renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error="Billing said no." onRetry={noop} onSection={noop} />);
      expect(labels(now).at(-1)).toBe("AI settings");
      const offered = labels(now).filter((l) => /^(Get Plus|Upgrade to Cloud)/.test(l));
      const all = [...purchaseOffers(true), ...purchaseOffers(false)].map((o) => o.label);
      for (const label of offered) expect(all).toContain(label);
      if (name === "free, Cloud on sale") expect(offered).toEqual(purchaseOffers(false).map((o) => o.label));
      if (name === "free, Plus on sale") expect(offered).toEqual(purchaseOffers(true).map((o) => o.label));
      if (name === "Cloud" || name === "Plus" || name === "signed out" || name === "checking") expect(offered).toEqual([]);
    });
  }
});

describe("prices come from the offers", () => {
  it("states no price the offers do not carry, and shows the caption under them", () => {
    const prices = (s: string) => s.match(/\$\d+/g) ?? [];
    const offered = [...purchaseOffers(true), ...purchaseOffers(false)].flatMap((o) => prices(o.label));
    for (const [name, view] of states) {
      const now = sentences(renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error={null} onRetry={noop} onSection={noop} />));
      for (const price of prices(now)) expect(offered, `${name} ${price}`).toContain(price);
    }
    const sale = sentences(renderToStaticMarkup(<PlanBody view={states[5][1]} ai={ai} busy={false} act={noop} error={null} onRetry={noop} onSection={noop} />));
    expect(sale).toContain(captionText(false));
  });

  it("the signed-out plan lines use the same prices as the buy buttons", () => {
    const view = states.find(([n]) => n === "signed out")![1];
    const now = sentences(renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error={null} onRetry={noop} onSection={noop} />));
    const offered = [...purchaseOffers(true), ...purchaseOffers(false)].flatMap((o) => o.label.match(/\$\d+/g) ?? []);
    for (const price of now.match(/\$\d+/g) ?? []) expect(offered).toContain(price);
  });
});

describe("no dropped names", () => {
  it("shows none in any state", () => {
    for (const [, view] of states) {
      const now = sentences(renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error={null} onRetry={noop} onSection={noop} />));
      for (const phrase of ["AI & routing", "memory database", "Activity capture"]) expect(now).not.toContain(phrase);
    }
  });
});
