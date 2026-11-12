import { useEffect, useRef, useState } from "react";
import { hotkeyStatus, setHotkeyShortcut } from "../api";
import { describeError, isCommandError } from "../errors";
import type { HotkeyStatus } from "../generated/types";
import { t } from "../l10n";
import { changeResultLine, DEFAULT_SHORTCUT, displayShortcut, hotkeyStatusLine, recordKey } from "./hotkeyModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Capture: the hotkey panel's shortcut, whether it is registered, and a recorder to change it
 *  (the Mac's hotkey row). A shortcut that can't register is said here, not only logged. */
export function HotkeyCard() {
  const [status, setStatus] = useState<HotkeyStatus | null>(null);
  const [recording, setRecording] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const recorder = useRef<HTMLButtonElement | null>(null);

  const refresh = () =>
    hotkeyStatus()
      .then(setStatus)
      .catch((e: unknown) => setNote(message(e)));
  useEffect(() => {
    refresh();
  }, []);

  const apply = (accelerator: string) => {
    setBusy(true);
    setHotkeyShortcut(accelerator)
      .then(async (attempt) => {
        // A refused shortcut puts the old one back; the stored status is that one, so read it again.
        const current = attempt.registered ? attempt : await hotkeyStatus();
        setStatus(current);
        setNote(changeResultLine(attempt, current));
      })
      .catch((e: unknown) => {
        setNote(message(e));
        void refresh();
      })
      .finally(() => setBusy(false));
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setRecording(false);
      return;
    }
    const recorded = recordKey(e);
    if (recorded.kind === "waiting") return;
    if (recorded.kind === "refused") {
      // Keep recording, so the next press can try another.
      setNote(recorded.reason);
      return;
    }
    setRecording(false);
    apply(recorded.accelerator);
  };

  return (
    <section className="card about" aria-labelledby="hotkey-heading">
      <h2 id="hotkey-heading" className="section-label">
        {t("Hotkey panel")}
      </h2>
      <p className="muted small">{t("Press this shortcut anywhere to ask about the window you are in.")}</p>
      {status ? (
        <>
          <p role="status" className={status.registered ? "muted" : undefined}>
            {hotkeyStatusLine(status)}
          </p>
          <div className="field-row">
            <button
              ref={recorder}
              className="mode"
              disabled={busy}
              aria-pressed={recording}
              onClick={() => {
                setNote(null);
                setRecording((r) => !r);
              }}
              onKeyDown={onKeyDown}
              onBlur={() => setRecording(false)}
            >
              {recording ? t("Press a shortcut…") : displayShortcut(status.shortcut)}
            </button>
            <button
              className="mode"
              disabled={busy || (status.registered && status.shortcut === DEFAULT_SHORTCUT)}
              onClick={() => {
                setRecording(false);
                setNote(null);
                apply(DEFAULT_SHORTCUT);
              }}
            >
              {t("Reset to default")}
            </button>
          </div>
        </>
      ) : (
        <p>{t("Loading…")}</p>
      )}
      {note && <p role="alert" className="small">{note}</p>}
    </section>
  );
}
