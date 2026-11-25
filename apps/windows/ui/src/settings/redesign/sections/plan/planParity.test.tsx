import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SyncView, WhimbrelAiView } from "../../../../generated/types";
import { UpgradeButtons, Caption, captionText, purchaseOffers } from "../../../PlanControls";
import { PlanBody as OldBody } from "../../../PlanCard";
import { PlanBody } from "./PlanBody";

// The redesigned plan card draws the same actions, with the same words, as the old one in every state.
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

describe("redesigned Plan & billing draws the old card's actions", () => {
  for (const [name, view] of states) {
    it(`${name}: same buttons in the same order, same lines`, () => {
      const old = renderToStaticMarkup(<OldBody view={view} ai={ai} busy={false} act={noop} error="Billing said no." onSignIn={noop} onRetry={noop} />);
      const now = renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error="Billing said no." onRetry={noop} onSection={noop} />);
      // The old body has no provider row (the old card adds it); the redesigned card ends with it.
      expect(labels(now)).toEqual([...labels(old), "AI settings"]);
      for (const line of sentences(old).split(/\s{2,}/).map((s) => s.trim()).filter(Boolean)) expect(sentences(now)).toContain(line);
    });
  }
});

describe("prices come from the same words as the old buttons", () => {
  it("purchaseOffers and captionText match UpgradeButtons and Caption exactly", () => {
    for (const plus of [true, false]) {
      expect(purchaseOffers(plus).map((o) => o.label)).toEqual(labels(renderToStaticMarkup(<UpgradeButtons busy={false} act={noop} plus={plus} />)));
      expect(sentences(renderToStaticMarkup(<Caption plus={plus} />)).trim()).toBe(captionText(plus));
    }
  });

  it("never states a price the old buttons do not", () => {
    const prices = (s: string) => s.match(/\$\d+/g) ?? [];
    for (const [, view] of states) {
      const now = sentences(renderToStaticMarkup(<PlanBody view={view} ai={ai} busy={false} act={noop} error={null} onRetry={noop} onSection={noop} />));
      const old = sentences(renderToStaticMarkup(<OldBody view={view} ai={ai} busy={false} act={noop} error={null} onSignIn={noop} onRetry={noop} />));
      expect(prices(now)).toEqual(prices(old));
    }
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
