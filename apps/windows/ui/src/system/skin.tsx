// A card for choosing a skin: a preview, a name, a selected ring with a check, and a row of palette dots.

import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { t } from "../l10n";
import { navFocusIndex } from "./settingsLogic";
import { WIcon } from "./WIcon";
import "./system.css";
import "./settings.css";

export interface WSkinPalette {
  id: string;
  /** "Dusk": the dot's name for a screen reader. */
  label: string;
  /** A CSS color for the dot, taken from the palette itself at run time. */
  color: string;
}

export interface WSkinCardProps {
  name: string;
  /** One line under the name, such as the skin's tagline. */
  caption?: string;
  /** The drawing of the skin, at any size: the card gives it a fixed frame. */
  preview: ReactNode;
  selected: boolean;
  onSelect: () => void;
  /** A skin the person made. */
  yours?: boolean;
  yoursLabel?: string;
  /** Rename, Export, Delete: any menu control, drawn at the card's top corner. */
  menu?: ReactNode;
  palettes?: readonly WSkinPalette[];
  paletteId?: string;
  onPalette?: (id: string) => void;
  paletteLabel?: string;
}

/** The card is a toggle button (`aria-pressed`) for the skin, with its palette dots as a radio group beside it, so
 *  no control sits inside another. The dot row is always reserved, so the card does not grow when it is selected. */
export function WSkinCard({ name, caption, preview, selected, onSelect, yours, yoursLabel, menu, palettes = [], paletteId, onPalette, paletteLabel }: WSkinCardProps) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const current = palettes.some((p) => p.id === paletteId) ? paletteId : palettes[0]?.id;
  const move = (e: KeyboardEvent<HTMLButtonElement>, i: number) => {
    const next = navFocusIndex(e.key === "ArrowRight" ? "ArrowDown" : e.key === "ArrowLeft" ? "ArrowUp" : e.key, i, palettes.length);
    if (next === null) return;
    e.preventDefault();
    onPalette?.(palettes[next].id);
    refs.current[next]?.focus();
  };
  return (
    <div className={selected ? "w-skin w-skin-selected" : "w-skin"}>
      <button type="button" className="w-skin-main" aria-pressed={selected} onClick={onSelect}>
        <span className="w-skin-preview">{preview}</span>
        <span className="w-skin-name">
          <span>{name}</span>
          {yours ? <span className="w-skin-yours">{yoursLabel ?? t("Yours")}</span> : null}
        </span>
        {caption ? <span className="w-skin-caption">{caption}</span> : null}
      </button>
      {selected ? (
        <span className="w-skin-check" aria-hidden="true">
          <WIcon name="check" size={16} />
        </span>
      ) : null}
      {menu ? <div className="w-skin-menu">{menu}</div> : null}
      <div className="w-skin-palettes" role={palettes.length > 1 ? "radiogroup" : undefined} aria-label={palettes.length > 1 ? (paletteLabel ?? t("Palette")) : undefined}>
        {palettes.length > 1
          ? palettes.map((p, i) => (
              <button
                key={p.id}
                ref={(el) => {
                  refs.current[i] = el;
                }}
                type="button"
                role="radio"
                aria-checked={selected && p.id === current}
                aria-label={p.label}
                tabIndex={p.id === current ? 0 : -1}
                className={selected && p.id === current ? "w-dot w-dot-on" : "w-dot"}
                onClick={() => onPalette?.(p.id)}
                onKeyDown={(e) => move(e, i)}
              >
                <span className="w-dot-fill" style={{ background: p.color }} />
              </button>
            ))
          : null}
      </div>
    </div>
  );
}

/** The grid the cards sit in: as many columns as fit, never narrower than 11rem. */
export function WSkinGrid({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="w-skin-grid" role="group" aria-label={label}>
      {children}
    </div>
  );
}
