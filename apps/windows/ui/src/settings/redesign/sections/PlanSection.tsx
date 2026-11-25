// Settings > Plan & billing, redrawn on the Settings parts. The reads, the states and the commands are the old
// PlanCard's (see plan/usePlan.ts and plan/PlanBody.tsx); prices come from the same strings the old buttons use.
import type { SectionProps } from "../types";
import { PlanBody } from "./plan/PlanBody";
import { usePlan } from "./plan/usePlan";

export function PlanSection({ onSection }: SectionProps) {
  const { view, ai, busy, billingError, act, retry } = usePlan();
  return <PlanBody view={view} ai={ai} busy={busy} error={billingError} act={act} onRetry={retry} onSection={onSection} />;
}
