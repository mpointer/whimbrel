// The five Home element choices a skin makes in `today.options`: a TypeScript mirror of core/skins/src/elements.rs
// (docs/ui-redesign/skins.md). A key with no value takes it from the skin's old `archetype`; a key with a value this
// list does not know takes the default for that part; other keys are ignored.

export type NextMeetingVariant = "card" | "scoreboard" | "infobox";
export type TimelineVariant = "line" | "ruled";
export type LabelsVariant = "plain" | "bracketed" | "smallCaps";
export type AskVariant = "field" | "prompt";
export type ActionsVariant = "buttons" | "links";

export interface ElementChoices {
  nextMeeting: NextMeetingVariant;
  timeline: TimelineVariant;
  labels: LabelsVariant;
  ask: AskVariant;
  actions: ActionsVariant;
}

export const DEFAULT_ELEMENTS: ElementChoices = { nextMeeting: "card", timeline: "line", labels: "plain", ask: "field", actions: "buttons" };

/** Every key and its allowed values, in the spec's order (core `ELEMENT_KEYS`). */
export const ELEMENT_KEYS: readonly (readonly [keyof ElementChoices, readonly string[]])[] = [
  ["nextMeeting", ["card", "scoreboard", "infobox"]],
  ["timeline", ["line", "ruled"]],
  ["labels", ["plain", "bracketed", "smallCaps"]],
  ["ask", ["field", "prompt"]],
  ["actions", ["buttons", "links"]],
];

/** What an old skin with no element keys draws, by its `archetype` (the table in skins.md). An unknown archetype
 *  is Standard, as everywhere else. */
export function elementsFromArchetype(archetype: string | undefined): ElementChoices {
  const d = DEFAULT_ELEMENTS;
  switch (archetype) {
    case "field":
      return { ...d, nextMeeting: "scoreboard" };
    case "terminal":
      return { ...d, labels: "bracketed", ask: "prompt" };
    case "almanac":
      return { ...d, timeline: "ruled", labels: "smallCaps" };
    case "reference":
      return { ...d, nextMeeting: "infobox", timeline: "ruled", actions: "links" };
    default:
      return d;
  }
}

function pick<T extends string>(options: Record<string, string> | undefined, key: string, allowed: readonly T[], base: T, fallback: T): T {
  const value = options?.[key];
  if (value === undefined) return base;
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback;
}

/** Reads the five keys from `today.options`. */
export function elementsFromOptions(options: Record<string, string> | undefined, archetype: string | undefined): ElementChoices {
  const base = elementsFromArchetype(archetype);
  const d = DEFAULT_ELEMENTS;
  return {
    nextMeeting: pick(options, "nextMeeting", ["card", "scoreboard", "infobox"], base.nextMeeting, d.nextMeeting),
    timeline: pick(options, "timeline", ["line", "ruled"], base.timeline, d.timeline),
    labels: pick(options, "labels", ["plain", "bracketed", "smallCaps"], base.labels, d.labels),
    ask: pick(options, "ask", ["field", "prompt"], base.ask, d.ask),
    actions: pick(options, "actions", ["buttons", "links"], base.actions, d.actions),
  };
}
