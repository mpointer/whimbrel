import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { encryptionCard, encryptionNoteAction, setFastSearch, speedCheckCancel, speedCheckRun, speedCheckSave } from "../../../../../api";
import type { EncryptionCardView, NoteAction } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WButton, breakAtSeparators, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsProgressRow, WSettingsRow, WSettingsToggleRow } from "../../../../../system";
import { EncryptSection } from "../../../../EncryptionCard";
import { SpeedCheck } from "../../../../encryptionCardModel";
import { fastSearchHelp, libraryBadge, libraryBody, libraryEncryptWords, libraryNote, libraryNoteActionMessage, librarySpeedLines, unencryptedCopyNote } from "./encryptionWording";
import "./bottom.css";

/**
 * Library encryption, on the same commands and conditions as the old Database encryption card. The multi-step encrypt
 * flow is the existing one (EncryptSection), placed inside this card with this card's own words (encryptionWording.ts), so
 * the wizard and its messages say "your library" in every language and the old card's strings are untouched.
 */
export function LibraryEncryptionCard() {
  const { t } = useL10n();
  const [view, setView] = useState<EncryptionCardView | null>(null);
  const [failed, setFailed] = useState(false);
  const load = useCallback(
    () =>
      encryptionCard().then(
        (v) => {
          setView(v);
          setFailed(false);
        },
        () => setFailed(true),
      ),
    [],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const badge = view ? libraryBadge(view.badge) : undefined;
  return (
    <WSettingsCard
      id="capture.encryption"
      title={t("Library encryption")}
      headingLevel={3}
      status={badge ? { tone: badge.tone, children: badge.label } : undefined}
      loading={!view && !failed}
      error={failed && !view ? t("The encryption status could not be read.") : null}
    >
      {view && <LibraryEncryptionBody view={view} reload={load} />}
    </WSettingsCard>
  );
}

export function LibraryEncryptionBody({ view, reload }: { view: EncryptionCardView; reload: () => void }) {
  const paragraphs = libraryBody(view.badge);
  if (view.badge === "needsAttention") {
    return (
      <>
        <div className="rs-membottom-block">
          <WNoticeBox tone="warning">{paragraphs[0]}</WNoticeBox>
        </div>
        <LibraryNotes view={view} reload={reload} />
      </>
    );
  }
  return (
    <>
      <div className="rs-membottom-block">
        {paragraphs.map((p) => (
          <p key={p}>{p}</p>
        ))}
      </div>
      {view.plainBackupPresent && <WSettingsNote>{unencryptedCopyNote()}</WSettingsNote>}
      {view.badge === "notEncrypted" && (
        <div className="rs-membottom-block">
          <EncryptSection view={view} reload={reload} words={libraryEncryptWords()} />
        </div>
      )}
      <LibraryNotes view={view} reload={reload} />
      {view.showsStoreTools && view.fastSearch.available && <FastSearch enabled={view.fastSearch.enabled} reload={reload} />}
      {view.showsStoreTools && <LibrarySpeedCheck />}
    </>
  );
}

/** The notes from this launch, each with only its own button (off while the cleanup runs). */
function LibraryNotes({ view, reload }: { view: EncryptionCardView; reload: () => void }) {
  const [acting, setActing] = useState<NoteAction | null>(null);
  const [said, setSaid] = useState<{ text: string; error: boolean } | null>(null);
  const act = (action: NoteAction) => {
    setActing(action);
    setSaid(null);
    encryptionNoteAction(action)
      .then(
        (r) => setSaid(libraryNoteActionMessage(action, r)),
        () => setSaid(libraryNoteActionMessage(action, "failed")),
      )
      .finally(() => {
        setActing(null);
        reload();
      });
  };
  if (view.notes.length === 0 && !said) return null;
  return (
    <>
      {view.notes.map((row, i) => {
        const lines = libraryNote(row);
        if (lines.texts.length === 0 && !lines.action) return null;
        return (
          <div className="rs-membottom-block" key={i}>
            {lines.texts.map((text) => (
              <p key={text}>{text}</p>
            ))}
            {lines.action && (
              <div>
                <WButton disabled={view.cleanupRunning || acting !== null} onClick={() => act(lines.action!.action)}>
                  {lines.action.label}
                </WButton>
              </div>
            )}
          </div>
        );
      })}
      {said && (
        <div className="rs-membottom-block">
          <WNoticeBox tone={said.error ? "error" : "info"} live>
            {said.text}
          </WNoticeBox>
        </div>
      )}
    </>
  );
}

function FastSearch({ enabled, reload }: { enabled: boolean; reload: () => void }) {
  const { t } = useL10n();
  const [busy, setBusy] = useState(false);
  return (
    <WSettingsToggleRow
      label={t("Faster search (uses more memory)")}
      description={fastSearchHelp()}
      checked={enabled}
      disabled={busy}
      disabledReason={t("Saving…")}
      onToggle={(next) => {
        setBusy(true);
        setFastSearch(next)
          .catch(() => false)
          .finally(() => {
            setBusy(false);
            reload();
          });
      }}
    />
  );
}

function LibrarySpeedCheck() {
  const { t, locale } = useL10n();
  const check = useMemo(() => new SpeedCheck({ run: speedCheckRun, cancel: speedCheckCancel, save: speedCheckSave }), []);
  const s = useSyncExternalStore(check.subscribe, check.getState, check.getState);
  return (
    <>
      {s.kind === "running" ? (
        <WSettingsProgressRow label={t("Checking speed…")} progress={s.progress} onCancel={() => check.cancel()} />
      ) : (
        <WSettingsRow
          variant="button"
          label={t("Encryption speed")}
          actionLabel={t("Check speed")}
          disabled={!check.canStart}
          disabledReason={t("The last check is still stopping.")}
          error={s.kind === "idle" ? s.message : null}
          onPress={() => void check.start()}
        />
      )}
      {s.kind === "done" && (
        <>
          <div className="rs-membottom-block">
            <ul className="rs-membottom-list">
              {librarySpeedLines(s.report, locale).map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </div>
          <WSettingsControlRow label={t("Save report…")}>
            <WButton onClick={() => void check.save()}>{t("Save report…")}</WButton>
          </WSettingsControlRow>
          {s.savedTo && (
            <div role="status">
              <WSettingsNote>{t("Saved to {path}", { path: breakAtSeparators(s.savedTo) })}</WSettingsNote>
            </div>
          )}
        </>
      )}
      <WSettingsNote>
        {t("The report is saved only where you choose. It holds timings and the kind of PC only, no content, and you decide whether to send it.")}
      </WSettingsNote>
    </>
  );
}
