// What the Settings frame tells the person after a search hit: the row was found (said to a screen reader only, the mark
// on the row is the sighted cue) or it is not on screen in this state (said to everyone). Pure, so the words and the choice
// are tested without a window.
import { t } from "../../l10n";

export type JumpNotice = { kind: "shown"; announce: string } | { kind: "hidden"; hint: string };

/** `found` is whether the row's element appeared before the search gave up; `title` is the hit's name in the list. */
export function jumpNotice(found: boolean, title: string): JumpNotice {
  if (found) return { kind: "shown", announce: t("Showing {name}.", { name: title }) };
  return { kind: "hidden", hint: t("That setting isn't shown right now.") };
}
