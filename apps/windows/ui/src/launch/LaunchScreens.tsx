// The launch's own screens (brief 3.3.3, 3.3.5, 3.3.6): the silent start, the migration progress,
// the key-unavailable screen and the blocking screens drawn from their type. Every blocking screen
// has Quit; buttons are disabled from the press until the next stage.

import { useEffect, useState } from "react";
import type { LastActionView, LaunchAnswer, LaunchScreenView } from "../generated/types";
import { t } from "../l10n";
import { screenContent, type ScreenButton } from "./screenModel";

/** The page colour only, then after a second a small spinner with "Starting Whimbrel…". */
export function StartingScreen({ delayMs = 1000 }: { delayMs?: number }) {
  const [show, setShow] = useState(delayMs <= 0);
  useEffect(() => {
    if (delayMs <= 0) return;
    const timer = setTimeout(() => setShow(true), delayMs);
    return () => clearTimeout(timer);
  }, [delayMs]);
  return (
    <div className="launch-starting" aria-busy="true">
      {show && (
        <>
          <span className="launch-spinner" aria-hidden="true" />
          <span className="muted">{t("Starting Whimbrel…")}</span>
        </>
      )}
    </div>
  );
}

/** The migration at launch: a bar that never goes back, and no Quit (a crash is still safe). */
export function MigratingScreen({ overall }: { overall: number }) {
  return (
    <div className="launch-screen stack">
      <h1 className="display">{t("Encrypting the memory database")}</h1>
      <progress className="progress" max={1} value={overall} aria-label={t("Encrypting the memory database")} />
      <p>{t("This can take a few minutes on a large memory database. Keep Whimbrel open and your PC awake until it finishes.")}</p>
      <p className="muted small">{t("If it is interrupted, Whimbrel finishes or undoes it the next time it starts.")}</p>
    </div>
  );
}

/** Credential Manager did not answer for an encrypted database. Try again is the only action. */
export function KeyUnavailableScreen({
  stillUnavailable,
  busy,
  onAnswer,
}: {
  stillUnavailable: boolean;
  busy: boolean;
  onAnswer: (a: LaunchAnswer) => void;
}) {
  return (
    <div className="launch-screen stack">
      <div>
        <h1 className="display">{t("Whimbrel can't read its key right now")}</h1>
        <p>
          {t(
            "Windows did not let Whimbrel read the memory database key from Credential Manager. Your key and your data are most likely still there. This is not the same as a lost key, and you do not need your recovery key for it.",
          )}
        </p>
        <p className="muted small">{t("This can happen just after you sign in to Windows. Wait a moment, then try again.")}</p>
        {stillUnavailable && <p className="small setup-error">{t("The key still can't be read. If this keeps happening, sign out of Windows and back in, then try again.")}</p>}
      </div>
      <p className="muted small">{t("Whimbrel has not opened, changed or deleted anything.")}</p>
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy} onClick={() => onAnswer("retry")}>
          {busy ? t("Trying…") : t("Try again")}
        </button>
        <button className="mode" disabled={busy} onClick={() => onAnswer("quit")}>
          {t("Quit Whimbrel")}
        </button>
      </div>
    </div>
  );
}

/** A blocking screen from its type, with exactly the answers it offers. */
export function BlockingScreen({
  screen,
  offers,
  lastAction,
  busy,
  onAnswer,
}: {
  screen: LaunchScreenView;
  offers: readonly LaunchAnswer[];
  lastAction?: LastActionView | null;
  busy: boolean;
  onAnswer: (a: LaunchAnswer) => void;
}) {
  const content = screenContent(screen, offers, lastAction);
  const [asking, setAsking] = useState<ScreenButton | null>(null);
  const press = (b: ScreenButton) => (b.confirm ? setAsking(b) : onAnswer(b.answer));
  return (
    <div className="launch-screen stack">
      <div>
        <h1 className="display">{content.title}</h1>
        {content.paragraphs.map((p) => (
          <p key={p}>{p}</p>
        ))}
        {content.hints.map((h) => (
          <p key={h} className="muted small">
            {h}
          </p>
        ))}
        {content.lastAction && <p className="small setup-error">{content.lastAction}</p>}
      </div>
      <div className="toolbar">
        {content.buttons.map((b) => (
          <button key={b.answer} data-answer={b.answer} className={b.primary ? "mode mode--active" : "mode"} disabled={busy} onClick={() => press(b)}>
            {b.label}
          </button>
        ))}
      </div>
      {asking?.confirm && (
        <div role="alertdialog" aria-label={asking.confirm.title} className="card confirm stack-sm">
          <span className="row-title">{asking.confirm.title}</span>
          <span className="muted small">{asking.confirm.body}</span>
          <div className="toolbar">
            <button
              className={asking.confirm.destructive ? "mode setup-destructive" : "mode mode--active"}
              disabled={busy}
              onClick={() => {
                const answer = asking.answer;
                setAsking(null);
                onAnswer(answer);
              }}
            >
              {asking.confirm.confirm}
            </button>
            <button className="mode" onClick={() => setAsking(null)}>
              {t("Cancel")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
