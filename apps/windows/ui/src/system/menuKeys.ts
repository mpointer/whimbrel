// Arrow-key movement in a menu (role="menu" with menuitem and menuitemradio children): Down and Up move to the next and previous item and wrap,
// Home and End go to the first and last. One helper for every menu in the app, so they move the same way. Escape and focus return are the
// menu owner's (it knows its trigger).

import type { KeyboardEvent } from "react";

/** The item a key moves to, or null when the key is not one of the four. `current` is -1 when focus is not on an item. Pure. */
export function nextMenuIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowDown":
      return current < 0 ? 0 : (current + 1) % count;
    case "ArrowUp":
      return current < 0 ? count - 1 : (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

const ITEMS = '[role="menuitem"]:not(:disabled), [role="menuitemradio"]:not(:disabled), [role="menuitemcheckbox"]:not(:disabled)';

/** Put on the menu element's onKeyDown: moves focus among its enabled items. */
export function menuKeyDown(e: KeyboardEvent<HTMLElement>): void {
  if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
  const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(ITEMS));
  const next = nextMenuIndex(e.key, items.indexOf(document.activeElement as HTMLElement), items.length);
  if (next === null) return;
  e.preventDefault();
  items[next]?.focus();
}
