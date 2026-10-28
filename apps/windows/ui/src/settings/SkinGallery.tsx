import { useEffect, useState, useSyncExternalStore } from "react";
import { chooseSkin, currentSelection, installedSkins, resolveSelection, skinRevision, subscribeSkin } from "../skin";
import { APPEARANCES, resolveMode, type Appearance } from "../theme";
import { galleryState, GALLERY_FOOTNOTE, lockedCaption, pickCard, pickPalette, swatchColors, type Card } from "./skinGalleryModel";

const APPEARANCE_LABELS: Record<Appearance, string> = { system: "System", light: "Light", dark: "Dark" };

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** Whether the OS is dark right now, so System previews the right half of each palette. */
function useSystemDark(): boolean {
  const [dark, setDark] = useState(() => window.matchMedia(DARK_QUERY).matches);
  useEffect(() => {
    const query = window.matchMedia(DARK_QUERY);
    const onChange = () => setDark(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return dark;
}

function Swatches({ colors }: { colors: string[] }) {
  return (
    <span className="swatches" aria-hidden="true">
      {colors.map((color, i) => (
        <span key={i} className="swatch" style={{ background: color }} />
      ))}
    </span>
  );
}

/** Settings → Appearance: System / Light / Dark, and a card per installed skin; the selected one offers its
 *  palettes. Kept on this PC only. */
export function SkinGallery() {
  useSyncExternalStore(subscribeSkin, skinRevision, skinRevision);
  const systemDark = useSystemDark();
  const skins = installedSkins();
  const selection = currentSelection();
  const cards = galleryState(skins, selection);
  const { palette } = resolveSelection(skins, selection);
  const caption = lockedCaption(palette);
  const mode = resolveMode(selection.appearance, systemDark);
  const pick = (next: typeof selection) => chooseSkin(next);

  return (
    <section className="card" aria-labelledby="appearance-heading">
      <h2 id="appearance-heading" className="section-label">
        Appearance
      </h2>
      <div className="mode-picker" role="group" aria-label="Appearance">
        {APPEARANCES.map((appearance) => (
          <button
            key={appearance}
            className={appearance === selection.appearance ? "mode mode--active" : "mode"}
            aria-pressed={appearance === selection.appearance}
            disabled={caption !== null}
            onClick={() => pick({ ...selection, appearance })}
          >
            {APPEARANCE_LABELS[appearance]}
          </button>
        ))}
      </div>
      {caption && <p className="muted small">{caption}</p>}
      <h3 className="gallery-heading muted small">Skin</h3>
      <div className="skin-gallery">
        {cards.map((card: Card) => (
          <div key={card.id} className={card.selected ? "skin-card skin-card--selected" : "skin-card"}>
            <button
              className="skin-card-pick"
              aria-pressed={card.selected}
              onClick={() => pick(pickCard(skins, selection, card.id))}
            >
              <Swatches colors={swatchColors(card.previewPalette, mode)} />
              <span className="skin-card-name">{card.name}</span>
              <span className="muted small">{card.tagline}</span>
            </button>
            {card.yours && <span className="skin-yours">Yours</span>}
            {card.selected && card.palettes.length > 1 && (
              <div className="skin-palettes" role="group" aria-label={`${card.name} palettes`}>
                {card.palettes.map((p) => {
                  const choice = skins.find((s) => s.skin.id === card.id)?.palettes.find((x) => x.id === p.id);
                  return (
                    <button
                      key={p.id}
                      className={p.current ? "skin-palette skin-palette--current" : "skin-palette"}
                      aria-pressed={p.current}
                      title={p.name}
                      aria-label={p.name}
                      onClick={() => pick(pickPalette(selection, card.id, p.id))}
                    >
                      {choice && <Swatches colors={swatchColors(choice, mode)} />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        ))}
      </div>
      <p className="muted small">{GALLERY_FOOTNOTE}</p>
    </section>
  );
}
