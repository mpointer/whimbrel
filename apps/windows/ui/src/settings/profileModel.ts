import type { ProfileFields, ProfileView, SuggestionView } from "../generated/types";
import { t } from "../l10n";

// What Settings → Profile says and decides, apart from the component: the counters, the privacy
// wording beside the suggestion switch, the preview, and when an edit is worth saving.

/** A count in the UI language's digits and grouping. */
const count = (n: number, locale: string) => n.toLocaleString(locale);

/** The four text fields as typed, before they are saved. */
export type FieldsDraft = ProfileFields;

export const toDraft = (view: ProfileView): FieldsDraft => ({
  name: view.name,
  role: view.role,
  organization: view.organization,
  about: view.about,
});

/** Whether the draft differs from what is saved: saving on blur only when it does. */
export const fieldsDirty = (view: ProfileView, draft: FieldsDraft): boolean =>
  view.name !== draft.name || view.role !== draft.role || view.organization !== draft.organization || view.about !== draft.about;

/** The length of everything that is sent, against the cap. Over it, the rest is not sent. */
export function counterLine(view: Pick<ProfileView, "composedChars" | "maxChars">, locale: string): string {
  const used = count(view.composedChars, locale);
  const max = count(view.maxChars, locale);
  const over = view.composedChars - view.maxChars;
  if (over > 0) {
    return t("{used} of {max} characters. The last {over} are not sent. Shorten it to send all of it.", { used, max, over: count(over, locale) });
  }
  return t("{used} of {max} characters", { used, max });
}

export const isOverCap = (view: Pick<ProfileView, "composedChars" | "maxChars">): boolean => view.composedChars > view.maxChars;

export const factsLine = (view: Pick<ProfileView, "facts" | "maxFacts">): string => t("{used} of {max} facts", { used: view.facts.length, max: view.maxFacts });

/** The line under "Learned facts": the count, then how the list works, as one sentence to translate. */
export const factsCaption = (view: Pick<ProfileView, "facts" | "maxFacts">): string =>
  t("{used} of {max} facts. One line each. Edit or remove any of them.", { used: view.facts.length, max: view.maxFacts });

/** Why a new fact cannot be added yet, or null when it can. Empty text is silent: the button is just off. */
export function addFactBlocker(view: Pick<ProfileView, "facts" | "maxFacts" | "maxFactChars">, text: string): string | null {
  // Characters as the shell counts them (code points, not UTF-16 units), after whitespace collapses to one line.
  const length = Array.from(text.split(/\s+/).filter(Boolean).join(" ")).length;
  if (length === 0) return null;
  if (view.facts.length >= view.maxFacts) return t("You can keep at most {count} facts. Remove one first.", { count: view.maxFacts });
  if (length > view.maxFactChars) return t("A fact can be at most {count} characters (this one is {length}).", { count: view.maxFactChars, length });
  return null;
}

export const canAddFact = (view: Pick<ProfileView, "facts" | "maxFacts" | "maxFactChars">, text: string): boolean =>
  text.trim() !== "" && addFactBlocker(view, text) === null;

/** What the "Suggest additions" switch does, said plainly, because it sends meeting content to a model. */
export const suggestPrivacy = (): string =>
  t("Uses your meeting summaries (title, summary, decisions and action items) and the AI routing you chose in AI & routing, so a hosted model sees them unless Local-only is on. Nothing is added unless you press Add.");

/** Whether the daily job is on, armed or waiting, so the switch is never a silent promise. */
export function suggestStatus(view: Pick<ProfileView, "suggestEnabled" | "suggestionsActive">): string {
  if (!view.suggestEnabled) return t("Off. Whimbrel will not look at your meetings.");
  if (!view.suggestionsActive) return t("On, but waiting: nothing happens until your Profile has a field or a fact.");
  return t("On. Once a day Whimbrel reads that day's meeting summaries and proposes facts for you to add or dismiss.");
}

/** "What Whimbrel sees": the exact block, or why there is none. */
export function previewText(view: Pick<ProfileView, "enabled" | "preview">): string {
  if (!view.enabled) return t("Off. Nothing from your Profile is sent with a prompt.");
  if (view.preview === "") return t("Nothing written yet. Nothing is sent.");
  return view.preview;
}

/** Where a suggestion came from, or null when it was not drawn from one meeting. */
export function sourceLabel(s: Pick<SuggestionView, "sourceMeetingId" | "sourceMeetingTitle">): string | null {
  if (s.sourceMeetingId === null) return null;
  return s.sourceMeetingTitle ? t("From “{title}”", { title: s.sourceMeetingTitle }) : t("From a meeting that is no longer here");
}

export const suggestionsEmpty = (): string =>
  t("No suggestions yet. Once a day, if the switch above is on and your Profile has something in it, Whimbrel looks at your meeting summaries and proposes facts here. Nothing is added unless you press Add.");
