import type { BriefRecap } from "../generated/types";
import { briefRecapNote } from "./todayModel";

/** Today's Daily Brief shown as the recap: the routine's own summary and where it came from. */
export function BriefRecapBody({ brief }: { brief: BriefRecap }) {
  return (
    <>
      <p className="recap">{brief.summary}</p>
      <p className="muted small">{briefRecapNote(brief)}</p>
    </>
  );
}
