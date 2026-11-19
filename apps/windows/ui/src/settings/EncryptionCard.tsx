// Settings > Capture & Privacy > Database encryption (brief 3.3.7). The rules are in
// encryptionCardModel.ts; the recovery setup is the launch's own (launch/SetupView). Nothing here
// migrates: Encrypt only stores the pending key and sets "migrate at the next launch".

import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  encryptionCard,
  encryptionEncrypt,
  encryptionNoteAction,
  encryptionPrecheckForSetup,
  restartApp,
  setFastSearch,
  setupBegin,
  setupContinue,
  setupReusesKey,
  setupSavedTo,
  setupSaveFile,
  setupStartOver,
  setupVerify,
  speedCheckCancel,
  speedCheckRun,
  speedCheckSave,
} from "../api";
import type { EncryptionCardView, NoteAction } from "../generated/types";
import { useL10n } from "../l10n";
import { SetupModel } from "../launch/setupModel";
import { SetupView } from "../launch/SetupView";
import { badgeLabel, cardBody, EncryptFlow, noteActionMessage, noteLines, SpeedCheck, speedLines } from "./encryptionCardModel";

export function EncryptionCard() {
  const { t } = useL10n();
  const [view, setView] = useState<EncryptionCardView | null>(null);
  const load = useCallback(() => encryptionCard().then(setView, () => {}), []);
  useEffect(() => {
    void load();
  }, [load]);
  if (!view) return null;
  return (
    <section className="card stack-sm" aria-labelledby="encryption-heading">
      <div className="section-head">
        <h2 id="encryption-heading" className="section-label">
          {t("Database encryption")}
        </h2>
        <span className={`chip encryption-badge encryption-badge--${view.badge}`}>{badgeLabel(view.badge)}</span>
      </div>
      <EncryptionCardBody view={view} reload={load} />
    </section>
  );
}

/** Everything under the header, for a given card view. */
export function EncryptionCardBody({ view, reload }: { view: EncryptionCardView; reload: () => void }) {
  const { t } = useL10n();
  return (
    <>
      {cardBody(view.badge).map((p) => (
        <p key={p} className={view.badge === "needsAttention" ? undefined : "muted small"}>
          {p}
        </p>
      ))}
      {view.badge === "needsAttention" ? (
        <NotesList view={view} reload={reload} />
      ) : (
        <>
          {view.plainBackupPresent && (
            <p className="muted small">
              {t(
                "Whimbrel keeps an unencrypted copy from before encryption until it has started normally three times, then deletes it. Deleting a file does not guarantee its contents are gone from the disk.",
              )}
            </p>
          )}
          {view.badge === "notEncrypted" && <EncryptSection view={view} reload={reload} />}
          <NotesList view={view} reload={reload} />
          {view.showsStoreTools && view.fastSearch.available && <FastSearchToggle enabled={view.fastSearch.enabled} reload={reload} />}
          {view.showsStoreTools && <SpeedCheckSection />}
        </>
      )}
    </>
  );
}

/** The notes from this launch, each with only its own button (off while the cleanup runs). */
export function NotesList({ view, reload }: { view: EncryptionCardView; reload: () => void }) {
  const [acting, setActing] = useState<NoteAction | null>(null);
  const [said, setSaid] = useState<{ text: string; error: boolean } | null>(null);
  const act = (action: NoteAction) => {
    setActing(action);
    setSaid(null);
    encryptionNoteAction(action)
      .then((r) => setSaid(noteActionMessage(action, r)), () => setSaid(noteActionMessage(action, "failed")))
      .finally(() => {
        setActing(null);
        reload();
      });
  };
  if (view.notes.length === 0 && !said) return null;
  return (
    <div className="stack-sm encryption-notes">
      {view.notes.map((row, i) => {
        const lines = noteLines(row);
        return (
          <div key={i} className="encryption-note">
            {lines.texts.map((text) => (
              <p key={text} className="small">
                {text}
              </p>
            ))}
            {lines.action && (
              <button className="mode" disabled={view.cleanupRunning || acting !== null} onClick={() => act(lines.action!.action)}>
                {lines.action.label}
              </button>
            )}
          </div>
        );
      })}
      {said && <p className={said.error ? "small setup-error" : "small"}>{said.text}</p>}
    </div>
  );
}

function EncryptSection({ view, reload }: { view: EncryptionCardView; reload: () => void }) {
  const { t } = useL10n();
  const flow = useMemo(
    () =>
      new EncryptFlow({
        precheck: encryptionPrecheckForSetup,
        continueSetup: () => setupContinue("settings"),
        encrypt: encryptionEncrypt,
        reusesKey: () => setupReusesKey("settings"),
        discard: () => setupStartOver("settings"),
      }),
    [],
  );
  const phase = useSyncExternalStore(flow.subscribe, flow.getState, flow.getState);
  // Leaving Settings mid-setup is a Cancel: the shell forgets the words it made. Cancel does
  // nothing once encrypting started or before the setup opened.
  useEffect(() => () => flow.cancel(), [flow]);
  // The Settings setup reuses the key the migration will use (db-key, else the pending key); the
  // shell says which after the precheck.
  const reusesKey = flow.reusesKey;
  const setup = useMemo(
    () =>
      new SetupModel({
        begin: (p) => setupBegin("settings", p),
        saveFile: () => setupSaveFile("settings"),
        verify: (p) => setupVerify("settings", p),
        startOver: () => setupStartOver("settings"),
        savedTo: () => setupSavedTo("settings"),
        print: () => window.print(),
        reusesKey,
      }),
    [reusesKey],
  );
  useEffect(() => {
    if (phase.kind === "atNextLaunch") reload();
  }, [phase.kind, reload]);

  if (view.pendingNextLaunch || phase.kind === "atNextLaunch") {
    return (
      <div className="stack-sm">
        <p className="small">{t("Whimbrel will finish encrypting the memory database the next time it starts.")}</p>
        <div className="toolbar">
          <button className="mode mode--active" onClick={() => void restartApp().catch(() => {})}>
            {t("Restart Whimbrel")}
          </button>
        </div>
      </div>
    );
  }
  switch (phase.kind) {
    case "checking":
      return <p className="small muted">{t("Checking that encryption can start…")}</p>;
    case "settingUp":
    case "finishing":
      return (
        <SetupView
          model={setup}
          continueTitle={t("Encrypt the memory database")}
          continuing={phase.kind === "finishing"}
          onContinue={() => void flow.finish().then(() => setup.clear())}
          onCancel={() => {
            setup.clear();
            flow.cancel();
          }}
        />
      );
    case "failed":
      return (
        <div className="stack-sm">
          <p className="small setup-error">{phase.message}</p>
          <div className="toolbar">
            <button className="mode" onClick={() => flow.dismiss()}>
              {t("OK")}
            </button>
          </div>
        </div>
      );
    default:
      return (
        <div className="stack-sm">
          {phase.kind === "idle" && phase.message && <p className="small setup-error">{phase.message}</p>}
          <div className="toolbar">
            <button className="mode mode--active" onClick={() => void flow.start()}>
              {t("Encrypt the database…")}
            </button>
          </div>
        </div>
      );
  }
}

function FastSearchToggle({ enabled, reload }: { enabled: boolean; reload: () => void }) {
  const { t } = useL10n();
  const [busy, setBusy] = useState(false);
  return (
    <div className="stack-sm">
      <label className="toggle">
        <input
          type="checkbox"
          checked={enabled}
          disabled={busy}
          onChange={(e) => {
            setBusy(true);
            setFastSearch(e.target.checked)
              .catch(() => false)
              .finally(() => {
                setBusy(false);
                reload();
              });
          }}
        />
        {t("Faster search (uses more memory)")}
      </label>
      <p className="muted small">
        {t("Gives search up to 1 GB of extra memory. Takes effect after you restart Whimbrel, and only applies once the memory database is encrypted.")}
      </p>
    </div>
  );
}

function SpeedCheckSection() {
  const { t, locale } = useL10n();
  const check = useMemo(() => new SpeedCheck({ run: speedCheckRun, cancel: speedCheckCancel, save: speedCheckSave }), []);
  const s = useSyncExternalStore(check.subscribe, check.getState, check.getState);
  return (
    <div className="stack-sm">
      {s.kind === "running" ? (
        <>
          <p className="small">{t("Checking speed…")}</p>
          <progress className="progress" max={1} value={s.progress} aria-label={t("Checking speed…")} />
          <div className="toolbar">
            <button className="mode" onClick={() => check.cancel()}>
              {t("Cancel")}
            </button>
          </div>
        </>
      ) : (
        <div className="toolbar">
          <button className="mode" disabled={!check.canStart} onClick={() => void check.start()}>
            {t("Check encryption speed")}
          </button>
        </div>
      )}
      {s.kind === "idle" && s.message && <p className="small setup-error">{s.message}</p>}
      {s.kind === "done" && (
        <>
          <ul className="encryption-speed small">
            {speedLines(s.report, locale).map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <div className="toolbar">
            <button className="mode" onClick={() => void check.save()}>
              {t("Save report…")}
            </button>
          </div>
          {s.savedTo && <p className="small">{t("Saved to {path}", { path: s.savedTo })}</p>}
        </>
      )}
      <p className="muted small">
        {t("The report is saved only where you choose. It holds timings and the kind of PC only, no content, and you decide whether to send it.")}
      </p>
    </div>
  );
}
