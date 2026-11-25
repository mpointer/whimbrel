// The top-level tabs — docs/12 §6: same names and order as the Mac's WhimbrelShellView
// (Topics after Library, topics piece 2). Settings is a tab here, not a separate window (Windows convention).

import { t } from "./l10n";

export type TabId = "today" | "meetings" | "library" | "topics" | "conversations" | "automations" | "settings";

export interface Tab {
  id: TabId;
  label: string;
  /** Set while the tab's content belongs to a later milestone. */
  comingIn?: string;
}

/** The tabs with their labels in the current language; called at render, so a language change shows. With the redesign switch on there is a
 *  seventh section, Conversations, after Topics (it is Ctrl+7: the other sections keep their keys, see shell/redesign/shortcuts.ts); with it off, exactly the six. */
export function tabs(redesign = false): readonly Tab[] {
  const six: Tab[] = [
    { id: "today", label: t("Today") },
    { id: "meetings", label: t("Meetings") },
    { id: "library", label: t("Library") },
    { id: "topics", label: t("Topics") },
    { id: "automations", label: t("Automations") },
    { id: "settings", label: t("Settings") },
  ];
  if (!redesign) return six;
  six.splice(4, 0, { id: "conversations", label: t("Conversations") });
  return six;
}

export const DEFAULT_TAB: TabId = "today";

/** The tab to draw for `active`: the default tab when `active` is not in the list (Conversations was open, then the redesign switch went off at
 *  runtime). The Mac's `WhimbrelTab.shown`. */
export function shownTab(list: readonly Tab[], active: TabId): Tab {
  return list.find((each) => each.id === active) ?? list.find((each) => each.id === DEFAULT_TAB) ?? list[0];
}
