import type { SyncView, WhimbrelAiView } from "../generated/types";
import { whimbrelAllowanceLine } from "./aiModel";
import { planLine } from "./syncModel";

// Settings → Plan & billing and the Status page's Plan row (the Mac's PlanSectionState and PlanStatusLines).

/** What the Plan & billing section shows. */
export type PlanSectionState = "signedOut" | "loading" | "readFailed" | "finishSigningIn" | "plan";

/** Sync is far enough along for the plan, Buy and Manage: ready, or stopped at the free plan's device limit. */
export const planReachable = (view: Pick<SyncView, "state" | "deviceLimit">): boolean => view.state === "ready" || view.deviceLimit;

/**
 * Signed out (or sync not set up) asks to sign in. Ready or at the device limit shows the plan once
 * read, "Couldn't check" when the read failed, and "Checking" until then. Any other sign-in state
 * (approval, account key, an error) has no plan and no buttons: "Finish signing in".
 */
export function planSectionState(view: Pick<SyncView, "state" | "deviceLimit" | "plan" | "planUnknown">): PlanSectionState {
  if (view.state === "notSignedIn" || view.state === "notConfigured") return "signedOut";
  if (!planReachable(view)) return "finishSigningIn";
  if (view.plan) return "plan";
  return view.planUnknown ? "readFailed" : "loading";
}

/** The Plus allowance line for the plan page and Status: only while sync is ready, so a stale one never shows. */
export function planAllowanceLine(sync: Pick<SyncView, "state">, ai: WhimbrelAiView | null, locale: string): string | null {
  if (sync.state !== "ready" || !ai?.signedIn) return null;
  return whimbrelAllowanceLine(ai, locale);
}

/** The Status page's Plan row: empty (the row is hidden) unless sync is ready or at its device limit and the plan is known. */
export function planStatusLines(sync: SyncView | null, ai: WhimbrelAiView | null, locale: string): string[] {
  if (!sync || !planReachable(sync) || !sync.plan) return [];
  const lines = [planLine(sync.plan, locale)];
  const allowance = planAllowanceLine(sync, ai, locale);
  if (allowance) lines.push(allowance);
  return lines;
}

