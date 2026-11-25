import type { BriefRecap } from "../generated/types";
import { useL10n } from "../l10n";
import { RecapMarkdown } from "./redesign/RecapMarkdown";
import { briefRecapNote } from "./todayModel";

/** Today's Daily Brief shown as the recap: the routine's own summary and where it came from. */
export function BriefRecapBody({ brief, formatted = false }: { brief: BriefRecap; formatted?: boolean }) {
  const { locale } = useL10n();
  return (
    <>
      {/* `formatted` is the redesigned Today only: the summary's Markdown is drawn as text. Off, the line is as it always was. */}
      {formatted ? <RecapMarkdown text={brief.summary} /> : <p className="recap">{brief.summary}</p>}
      <p className="muted small">{briefRecapNote(brief, locale)}</p>
    </>
  );
}
