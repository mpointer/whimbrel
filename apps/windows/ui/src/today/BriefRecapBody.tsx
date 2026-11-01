import type { BriefRecap } from "../generated/types";
import { useL10n } from "../l10n";
import { briefRecapNote } from "./todayModel";

/** Today's Daily Brief shown as the recap: the routine's own summary and where it came from. */
export function BriefRecapBody({ brief }: { brief: BriefRecap }) {
  const { locale } = useL10n();
  return (
    <>
      <p className="recap">{brief.summary}</p>
      <p className="muted small">{briefRecapNote(brief, locale)}</p>
    </>
  );
}
