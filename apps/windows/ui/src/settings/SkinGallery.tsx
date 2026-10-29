import { useEffect, useState, useSyncExternalStore } from "react";
import { skinDelete, skinExport, skinImport, skinRename } from "../api";
import { describeError, isCommandError } from "../errors";
import { chooseSkin, currentSelection, installedSkins, reloadSkins, resolveSelection, skinRevision, subscribeSkin } from "../skin";
import { APPEARANCES, resolveMode, type Appearance } from "../theme";
import { SkinChat } from "./SkinChat";
import {
  actionsEnabled,
  afterDelete,
  afterImport,
  cardElementId,
  deletePrompt,
  galleryState,
  GALLERY_FOOTNOTE,
  lockedCaption,
  pickCard,
  pickPalette,
  swatchColors,
  type Card,
} from "./skinGalleryModel";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Brings a card into view once the reloaded gallery has drawn it. */
function scrollToCard(elementId: string) {
  requestAnimationFrame(() => document.getElementById(elementId)?.scrollIntoView({ behavior: "smooth", block: "nearest" }));
}

/** The "Yours" mark and the card's menu. Renaming and the delete confirmation happen right on the card. */
function YoursRow(props: { card: Card; enabled: boolean }) {
  const { card, enabled } = props;
  const [mode, setMode] = useState<"idle" | "menu" | "rename" | "delete">("idle");
  const [name, setName] = useState(card.name);
  const [failure, setFailure] = useState<string | null>(null);
  const fail = (e: unknown) => setFailure(message(e));
  const start = (next: typeof mode) => {
    setFailure(null);
    setName(card.name);
    setMode(next);
  };

  const rename = () => {
    if (!name.trim()) return;
    skinRename(card.id, name)
      .then(() => reloadSkins())
      .then(() => scrollToCard(cardElementId(card.id)), fail);
    setMode("idle");
  };
  const remove = () => {
    skinDelete(card.id)
      .then(() => {
        chooseSkin(afterDelete(currentSelection(), card.id));
        return reloadSkins();
      })
      .catch(fail);
    setMode("idle");
  };
  const exportSkin = () => {
    setMode("idle");
    skinExport(card.id).catch(fail);
  };

  if (mode === "rename") {
    return (
      <form
        className="field-row skin-yours-row"
        onSubmit={(e) => {
          e.preventDefault();
          rename();
        }}
      >
        <input className="input grow" aria-label={`New name for ${card.name}`} value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoFocus />
        <button type="submit" className="skin-action mode mode--active" disabled={!name.trim()}>
          Save
        </button>
        <button type="button" className="skin-action mode" onClick={() => setMode("idle")}>
          Cancel
        </button>
      </form>
    );
  }
  if (mode === "delete") {
    return (
      <div className="skin-yours-row stack-sm">
        <p className="small">{deletePrompt(card.name)}</p>
        <div className="field-row">
          <button type="button" className="skin-action mode mode--active" onClick={remove}>
            Delete
          </button>
          <button type="button" className="skin-action mode" onClick={() => setMode("idle")}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="skin-yours-row stack-sm">
      <div className="field-row">
        <span className="skin-yours">Yours</span>
        <button
          type="button"
          className="skin-options"
          aria-label={`${card.name} options`}
          aria-expanded={mode === "menu"}
          onClick={() => setMode(mode === "menu" ? "idle" : "menu")}
        >
          …
        </button>
      </div>
      {mode === "menu" && (
        <div className="field-row" role="group" aria-label={`${card.name} options`}>
          <button type="button" className="skin-action mode" disabled={!enabled} onClick={() => start("rename")}>
            Rename…
          </button>
          <button type="button" className="skin-action mode" disabled={!enabled} onClick={exportSkin}>
            Export…
          </button>
          <button type="button" className="skin-action mode" disabled={!enabled} onClick={() => start("delete")}>
            Delete…
          </button>
        </div>
      )}
      {failure && <p className="muted small">{failure}</p>}
    </div>
  );
}

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
  const [turnRunning, setTurnRunning] = useState(false);
  const [importFailure, setImportFailure] = useState<string | null>(null);
  const importSkin = () => {
    setImportFailure(null);
    skinImport()
      .then(async (id) => {
        if (!id) return;
        await reloadSkins();
        scrollToCard(afterImport(currentSelection(), id).scrollTo);
      })
      .catch((e: unknown) => setImportFailure(message(e)));
  };

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
          <div key={card.id} id={cardElementId(card.id)} className={card.selected ? "skin-card skin-card--selected" : "skin-card"}>
            <button
              className="skin-card-pick"
              aria-pressed={card.selected}
              onClick={() => pick(pickCard(skins, selection, card.id))}
            >
              <Swatches colors={swatchColors(card.previewPalette, mode)} />
              <span className="skin-card-name">{card.name}</span>
              <span className="muted small">{card.tagline}</span>
            </button>
            {card.yours && <YoursRow card={card} enabled={actionsEnabled(turnRunning)} />}
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
      <div className="field-row skin-gallery-footer">
        <p className="muted small grow">{GALLERY_FOOTNOTE}</p>
        <button type="button" className="skin-action mode" disabled={!actionsEnabled(turnRunning)} onClick={importSkin}>
          Import Skin…
        </button>
      </div>
      {importFailure && <p className="muted small">{importFailure}</p>}
      <SkinChat onRunning={setTurnRunning} />
    </section>
  );
}
