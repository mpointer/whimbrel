// A small menu button for the finished note's header (redesign flag on only): Copy options and More. It opens on press, moves
// with the arrow keys (Home and End jump to the ends), and closes on Escape or an item press (focus goes back to the button), on
// a press outside it and when focus leaves it. Items are buttons with the menu roles, so a screen reader names each one and, for
// a choice, says which is on. A run of choices under a heading is a labelled group.

import { useEffect, useId, useRef, useState } from "react";
import { WButton, WIcon, type WIconName } from "../../system";

export interface NoteMenuItem {
  id: string;
  label: string;
  onPress: () => void;
  /** A choice among others (the transcript language): says whether it is the one in use. */
  checked?: boolean;
  disabled?: boolean;
  /** A heading drawn above this item: it starts a labelled group. */
  heading?: string;
  /** Belongs to the group the last heading started (the choices under it); without it the item stands outside the group. */
  grouped?: boolean;
}

/** The items cut into runs: a heading starts a labelled group that takes the `grouped` items after it; the rest stand outside any group. */
export function menuGroups(items: NoteMenuItem[]): { heading?: string; items: NoteMenuItem[] }[] {
  const groups: { heading?: string; items: NoteMenuItem[] }[] = [];
  for (const item of items) {
    const last = groups[groups.length - 1];
    if (item.heading) groups.push({ heading: item.heading, items: [item] });
    else if (last && (last.heading ? item.grouped : true)) last.items.push(item);
    else groups.push({ items: [item] });
  }
  return groups;
}

export function NoteMenu({ label, icon, hint, items, iconOnly }: { label: string; icon?: WIconName; hint?: string; items: NoteMenuItem[]; iconOnly?: boolean }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menuId = useId();
  // The first button in the root is the trigger.
  const trigger = () => root.current?.querySelector<HTMLButtonElement>("button");
  const list = useRef<HTMLDivElement>(null);
  const enabled = () => [...(list.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => root.current && !root.current.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", away);
    // The first enabled item takes focus, so the arrows work at once.
    enabled()[0]?.focus();
    return () => window.removeEventListener("mousedown", away);
  }, [open]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      if (!open) return;
      e.stopPropagation();
      setOpen(false);
      trigger()?.focus();
      return;
    }
    if (!open || !["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const all = enabled();
    if (all.length === 0) return;
    e.preventDefault();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
    all[next].focus();
  };

  // Focus leaving the whole menu (Tab, or a click elsewhere that takes focus) closes it, without taking focus back.
  const onBlur = (e: React.FocusEvent) => {
    if (open && !root.current?.contains(e.relatedTarget as Node | null)) setOpen(false);
  };

  const popup = { "aria-haspopup": "menu" as const, "aria-expanded": open, "aria-controls": open ? menuId : undefined };
  return (
    <div className="th-fin-menu-root" ref={root} onKeyDown={onKey} onBlur={onBlur}>
      {iconOnly ? (
        <button type="button" className="w-icon-button" aria-label={label} {...popup} onClick={() => setOpen(!open)}>
          <WIcon name="chevron" />
        </button>
      ) : (
        <WButton icon={icon} hint={hint} {...popup} onClick={() => setOpen(!open)}>
          {label}
        </WButton>
      )}
      {open ? (
        <div className="th-fin-menu" id={menuId} role="menu" aria-label={label} ref={list} tabIndex={-1}>
          {menuGroups(items).map((group, g) => {
            const buttons = group.items.map((item) => (
              <button
                key={item.id}
                type="button"
                role={item.checked == null ? "menuitem" : "menuitemradio"}
                aria-checked={item.checked == null ? undefined : item.checked}
                disabled={item.disabled}
                className="w-menu-item"
                onClick={() => {
                  setOpen(false);
                  trigger()?.focus();
                  item.onPress();
                }}
              >
                {item.label}
              </button>
            ));
            // The visible heading is drawn for the eye; the group's own label is what a screen reader says.
            return group.heading ? (
              <div key={g} role="group" aria-label={group.heading}>
                <p className="th-fin-menu-heading" aria-hidden="true">
                  {group.heading}
                </p>
                {buttons}
              </div>
            ) : (
              <div key={g} role="presentation">
                {buttons}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
