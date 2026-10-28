// The five top-level tabs — docs/12 §6: same names and order as the Mac's
// WhimbrelShellView. Settings is a tab here, not a separate window (Windows convention).

export type TabId = "today" | "meetings" | "library" | "automations" | "settings";

export interface Tab {
  id: TabId;
  label: string;
  /** Set while the tab's content belongs to a later milestone. */
  comingIn?: string;
}

export const TABS: readonly Tab[] = [
  { id: "today", label: "Today" },
  { id: "meetings", label: "Meetings" },
  { id: "library", label: "Library" },
  { id: "automations", label: "Automations" },
  { id: "settings", label: "Settings" },
];

export const DEFAULT_TAB: TabId = "today";
