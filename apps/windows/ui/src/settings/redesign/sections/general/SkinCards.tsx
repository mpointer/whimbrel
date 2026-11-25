// General → Appearance, Skin and Make a skin, drawn on the design system's parts. The skin commands are the old
// gallery's (skin_rename, skin_delete, skin_export, skin_import, and choosing a skin and palette); so is the model
// (skinGalleryModel.ts). "Make a skin" hosts the existing SkinChat unchanged.
import { useEffect, useRef, useState, useSyncExternalStore, type FormEvent } from "react";
import { skinDelete, skinExport, skinImport, skinRename } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import { t, useL10n } from "../../../../l10n";
import { chooseSkin, currentSelection, installedSkins, reloadSkins, resolveSelection, skinRevision, subscribeSkin } from "../../../../skin";
import { WButton, WConfirmDialog, WIconButton, WSettingsCard, WSettingsChoiceRow, WSettingsNote, WSkinCard, WSkinGrid } from "../../../../system";
import { APPEARANCES, resolveMode, type Appearance } from "../../../../theme";
import { SkinChat } from "../../../SkinChat";
import {
  actionsEnabled,
  afterDelete,
  afterImport,
  cardElementId,
  deletePrompt,
  galleryFootnote,
  galleryState,
  lockedCaption,
  pickCard,
  pickPalette,
  swatchColors,
  type Card,
} from "../../../skinGalleryModel";
import { actionsWaitNote, dotColor } from "./generalModel";
import "./general.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Brings a card into view once the reloaded gallery has drawn it. */
function scrollToCard(elementId: string) {
  requestAnimationFrame(() => document.getElementById(elementId)?.scrollIntoView({ behavior: "smooth", block: "nearest" }));
}

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

const appearanceLabels = (): Record<Appearance, string> => ({ system: t("System"), light: t("Light"), dark: t("Dark") });

/** Appearance: System, Light or Dark. A palette with one variant locks it and says so. */
export function AppearanceCard() {
  useSyncExternalStore(subscribeSkin, skinRevision, skinRevision);
  const skins = installedSkins();
  const selection = currentSelection();
  const caption = lockedCaption(resolveSelection(skins, selection).palette);
  const labels = appearanceLabels();
  return (
    <WSettingsCard id="general.appearance" title={t("Appearance")} headingLevel={3}>
      <WSettingsChoiceRow
        label={t("Appearance")}
        value={selection.appearance}
        options={APPEARANCES.map((a) => ({ value: a, label: labels[a] }))}
        onChange={(appearance) => chooseSkin({ ...selection, appearance })}
        disabled={caption !== null}
        disabledReason={caption ?? undefined}
      />
    </WSettingsCard>
  );
}

type Manage = { id: string; step: "menu" | "rename" | "delete" } | null;

/** Skin: a card per installed skin; the selected one offers its palettes. Yours get Rename, Export and Delete. */
export function SkinGalleryCard(props: { turnRunning: boolean }) {
  const { lang } = useL10n();
  useSyncExternalStore(subscribeSkin, skinRevision, skinRevision);
  const systemDark = useSystemDark();
  const skins = installedSkins();
  const selection = currentSelection();
  const cards = galleryState(skins, selection, lang);
  const mode = resolveMode(selection.appearance, systemDark);
  const enabled = actionsEnabled(props.turnRunning);
  const [manage, setManage] = useState<Manage>(null);
  const [name, setName] = useState("");
  const [failure, setFailure] = useState<string | null>(null);
  const fail = (e: unknown) => setFailure(message(e));
  const managed = manage ? cards.find((c) => c.id === manage.id) : undefined;
  const panel = useRef<HTMLDivElement>(null);
  // The options panel takes focus when it opens (its Rename box takes it itself when that step opens).
  useEffect(() => {
    if (manage?.step === "menu") panel.current?.focus({ preventScroll: false });
  }, [manage?.id, manage?.step]);
  /** Closes the panel and gives focus back to the card's options button, which is still there. */
  const closePanel = (id: string) => {
    setManage(null);
    setTimeout(() => document.getElementById(`rs-skin-opts-${id}`)?.focus(), 0);
  };

  const open = (card: Card, step: "menu" | "rename" | "delete") => {
    setFailure(null);
    setName(card.name);
    setManage({ id: card.id, step });
  };
  const rename = (card: Card) => {
    if (!name.trim()) return;
    skinRename(card.id, name)
      .then(() => reloadSkins())
      .then(() => scrollToCard(cardElementId(card.id)), fail);
    closePanel(card.id);
  };
  const remove = (card: Card) => {
    skinDelete(card.id)
      .then(() => {
        chooseSkin(afterDelete(currentSelection(), card.id));
        return reloadSkins();
      })
      .then(() => setTimeout(() => document.querySelector<HTMLElement>("#general\\.skin button")?.focus(), 0))
      .catch(fail);
    setManage(null);
  };
  const exportSkin = (card: Card) => {
    setManage(null);
    skinExport(card.id).catch(fail);
  };
  const importSkin = () => {
    setFailure(null);
    skinImport()
      .then(async (id) => {
        if (!id) return;
        await reloadSkins();
        scrollToCard(afterImport(currentSelection(), id).scrollTo);
      })
      .catch(fail);
  };

  return (
    <WSettingsCard
      id="general.skin"
      title={t("Skin")}
      headingLevel={3}
      description={galleryFootnote()}
      error={failure}
      actions={
        <WButton onClick={importSkin} disabled={!enabled}>
          {t("Import Skin…")}
        </WButton>
      }
    >
      <div className="rs-general-skins">
        <WSkinGrid label={t("Skin")}>
          {cards.map((card) => {
            const dto = skins.find((s) => s.skin.id === card.id);
            const current = card.palettes.find((p) => p.current)?.id;
            return (
              <div key={card.id} id={cardElementId(card.id)} className="rs-general-skin-item">
                <WSkinCard
                  name={card.name}
                  caption={card.tagline}
                  preview={
                    <span className="rs-general-swatches" aria-hidden="true">
                      {swatchColors(card.previewPalette, mode).map((color, i) => (
                        <span key={i} className="rs-general-swatch" style={{ background: color }} />
                      ))}
                    </span>
                  }
                  selected={card.selected}
                  onSelect={() => chooseSkin(pickCard(skins, selection, card.id))}
                  yours={card.yours}
                  yoursLabel={t("Yours")}
                  menu={
                    card.yours ? (
                      <WIconButton
                        id={`rs-skin-opts-${card.id}`}
                        icon="more"
                        label={t("{name} options", { name: card.name })}
                        aria-expanded={manage?.id === card.id}
                        onClick={() => (manage?.id === card.id ? closePanel(card.id) : open(card, "menu"))}
                      />
                    ) : undefined
                  }
                  palettes={card.palettes.map((p) => {
                    const choice = dto?.palettes.find((x) => x.id === p.id);
                    return { id: p.id, label: p.name, color: choice ? dotColor(choice, mode) : "transparent" };
                  })}
                  paletteId={current}
                  paletteLabel={t("{name} palettes", { name: card.name })}
                  onPalette={(id) => chooseSkin(pickPalette(selection, card.id, id))}
                />
              </div>
            );
          })}
        </WSkinGrid>
        {managed && manage && manage.step !== "delete" && (
          <div className="rs-general-panel" role="group" tabIndex={-1} ref={panel} aria-label={t("{name} options", { name: managed.name })}>
            {manage.step === "menu" ? (
              <div className="rs-general-panel-actions">
                <WButton onClick={() => open(managed, "rename")} disabled={!enabled}>
                  {t("Rename…")}
                </WButton>
                <WButton onClick={() => exportSkin(managed)} disabled={!enabled}>
                  {t("Export…")}
                </WButton>
                <WButton onClick={() => open(managed, "delete")} disabled={!enabled}>
                  {t("Delete…")}
                </WButton>
                <WButton variant="quiet" onClick={() => closePanel(managed.id)}>
                  {t("Cancel")}
                </WButton>
              </div>
            ) : (
              <form
                className="rs-general-rename"
                onSubmit={(e: FormEvent) => {
                  e.preventDefault();
                  rename(managed);
                }}
              >
                <input className="w-input" aria-label={t("New name for {name}", { name: managed.name })} value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoFocus />
                <WButton type="submit" variant="primary" disabled={!name.trim()}>
                  {t("Save")}
                </WButton>
                <WButton variant="quiet" onClick={() => closePanel(managed.id)}>
                  {t("Cancel")}
                </WButton>
              </form>
            )}
          </div>
        )}
        {!enabled && <WSettingsNote>{actionsWaitNote()}</WSettingsNote>}
      </div>
      {managed && manage?.step === "delete" && (
        <WConfirmDialog title={t("Delete skin")} body={deletePrompt(managed.name)} confirmLabel={t("Delete")} onConfirm={() => remove(managed)} onCancel={() => closePanel(managed.id)} />
      )}
    </WSettingsCard>
  );
}

/** Make a skin: the existing chat inside a card (the card is the heading; the chat draws none of its own). "Set up an AI provider" opens the AI section. */
export function MakeSkinCard(props: { onRunning: (running: boolean) => void; onSetUp: () => void }) {
  return (
    <WSettingsCard id="general.makeSkin" title={t("Make a skin")} headingLevel={3}>
      <div className="rs-general-chat">
        <SkinChat onRunning={props.onRunning} onSetUp={props.onSetUp} redesigned />
      </div>
    </WSettingsCard>
  );
}
