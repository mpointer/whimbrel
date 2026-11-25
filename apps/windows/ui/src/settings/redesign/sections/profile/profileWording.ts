// The redesigned Profile's words where the old profileModel says a name the redesign has dropped. The section is
// "AI" now, not "AI & routing" (docs/ui-redesign/screens/settings.md, "Naming"). Everything else comes from profileModel.
import { t } from "../../../../l10n";

/** What the "Suggest additions" switch does, said plainly, because it sends meeting content to a model. */
export const suggestPrivacyRedesign = (): string =>
  t("Uses your meeting summaries (title, summary, decisions and action items) and the AI routing you chose in AI, so a hosted model sees them unless Local-only is on. Nothing is added unless you press Add.");

/** Phrases the redesigned copy must not show. */
export const BANNED_PROFILE_PHRASES = ["AI & routing", "memory database", "Activity capture", "Capture & Privacy"] as const;
