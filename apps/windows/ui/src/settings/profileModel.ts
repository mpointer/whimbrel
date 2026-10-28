import type { ProfileFields, ProfileView, SuggestionView } from "../generated/types";

// What Settings → Profile says and decides, apart from the component: the counters, the privacy
// wording beside the suggestion switch, the preview, and when an edit is worth saving.

/** Fixed locale: the same text on every PC, and in tests. */
const count = (n: number) => n.toLocaleString("en-US");

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
export function counterLine(view: Pick<ProfileView, "composedChars" | "maxChars">): string {
  const over = view.composedChars - view.maxChars;
  if (over > 0) {
    return `${count(view.composedChars)} of ${count(view.maxChars)} characters. The last ${count(over)} are not sent. Shorten it to send all of it.`;
  }
  return `${count(view.composedChars)} of ${count(view.maxChars)} characters`;
}

export const isOverCap = (view: Pick<ProfileView, "composedChars" | "maxChars">): boolean => view.composedChars > view.maxChars;

export const factsLine = (view: Pick<ProfileView, "facts" | "maxFacts">): string => `${view.facts.length} of ${view.maxFacts} facts`;

/** Why a new fact cannot be added yet, or null when it can. Empty text is silent: the button is just off. */
export function addFactBlocker(view: Pick<ProfileView, "facts" | "maxFacts" | "maxFactChars">, text: string): string | null {
  // Characters as the shell counts them (code points, not UTF-16 units), after whitespace collapses to one line.
  const length = Array.from(text.split(/\s+/).filter(Boolean).join(" ")).length;
  if (length === 0) return null;
  if (view.facts.length >= view.maxFacts) return `You can keep at most ${view.maxFacts} facts. Remove one first.`;
  if (length > view.maxFactChars) return `A fact can be at most ${view.maxFactChars} characters (this one is ${length}).`;
  return null;
}

export const canAddFact = (view: Pick<ProfileView, "facts" | "maxFacts" | "maxFactChars">, text: string): boolean =>
  text.trim() !== "" && addFactBlocker(view, text) === null;

/** What the "Suggest additions" switch does, said plainly, because it sends meeting content to a model. */
export const SUGGEST_PRIVACY =
  "Uses your meeting summaries (title, summary, decisions and action items) and the AI routing you chose in AI & routing, so a hosted model sees them unless Local-only is on. Nothing is added unless you press Add.";

/** Whether the daily job is on, armed or waiting, so the switch is never a silent promise. */
export function suggestStatus(view: Pick<ProfileView, "suggestEnabled" | "suggestionsActive">): string {
  if (!view.suggestEnabled) return "Off. Whimbrel will not look at your meetings.";
  if (!view.suggestionsActive) return "On, but waiting: nothing happens until your Profile has a field or a fact.";
  return "On. Once a day Whimbrel reads that day's meeting summaries and proposes facts for you to add or dismiss.";
}

/** "What Whimbrel sees": the exact block, or why there is none. */
export function previewText(view: Pick<ProfileView, "enabled" | "preview">): string {
  if (!view.enabled) return "Off. Nothing from your Profile is sent with a prompt.";
  if (view.preview === "") return "Nothing written yet. Nothing is sent.";
  return view.preview;
}

/** Where a suggestion came from, or null when it was not drawn from one meeting. */
export function sourceLabel(s: Pick<SuggestionView, "sourceMeetingId" | "sourceMeetingTitle">): string | null {
  if (s.sourceMeetingId === null) return null;
  return s.sourceMeetingTitle ? `From “${s.sourceMeetingTitle}”` : "From a meeting that is no longer here";
}

export const SUGGESTIONS_EMPTY =
  "No suggestions yet. Once a day, if the switch above is on and your Profile has something in it, Whimbrel looks at your meeting summaries and proposes facts here. Nothing is added unless you press Add.";
