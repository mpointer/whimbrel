import { useEffect, useRef, useState } from "react";
import { hotkeyStatus, setHotkeyShortcut } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { HotkeyStatus } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WSettingsCard, WSettingsRow, WSettingsShortcutRow, WSettingsStatusRow } from "../../../../../system";
import { changeResultLine, DEFAULT_SHORTCUT, displayShortcut, hotkeyStatusLine } from "../../../../hotkeyModel";
import { recorderStep } from "./hotkeyRecorder";
import "./bottom.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

const ROW_ID = "rs-membottom-hotkey-row";

/** Hotkey panel: the shortcut, whether it is registered, and a recorder to change it. A shortcut that can't register is said here. */
export function HotkeyPanelCard() {
  const { t } = useL10n();
  const [status, setStatus] = useState<HotkeyStatus | null>(null);
  const [recording, setRecording] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // What the last successful change said, in a live region that is always there, so a screen reader hears it.
  const [done, setDone] = useState("");
  // Recording ended because focus moved on (Tab, or a click elsewhere): do not pull it back to the button.
  const movedOn = useRef(false);

  const refresh = () =>
    hotkeyStatus()
      .then(setStatus)
      .catch((e: unknown) => setNote(message(e)));
  useEffect(() => {
    void refresh();
  }, []);

  const apply = (accelerator: string) => {
    setBusy(true);
    setHotkeyShortcut(accelerator)
      .then(async (attempt) => {
        // A refused shortcut puts the old one back; the stored status is that one, so read it again.
        const current = attempt.registered ? attempt : await hotkeyStatus();
        setStatus(current);
        setNote(changeResultLine(attempt, current));
        setDone(attempt.registered ? t("Shortcut set to {shortcut}.", { shortcut: displayShortcut(attempt.shortcut) }) : "");
      })
      .catch((e: unknown) => {
        setNote(message(e));
        void refresh();
      })
      .finally(() => setBusy(false));
  };

  // While recording, every key press goes to the recorder, except Tab. Escape, Tab, focus moving to anything outside the
  // row, or leaving the window ends it. It never holds the keyboard.
  useEffect(() => {
    if (!recording) return;
    movedOn.current = false;
    const onKey = (e: KeyboardEvent) => {
      const step = recorderStep(e);
      if (step.kind === "leave") {
        movedOn.current = true;
        setRecording(false);
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      if (step.kind === "cancel") setRecording(false);
      else if (step.kind === "refused") setNote(step.reason);
      else if (step.kind === "apply") {
        setRecording(false);
        apply(step.accelerator);
      }
    };
    const onBlur = () => setRecording(false);
    const onFocusIn = (e: FocusEvent) => {
      const row = document.getElementById(ROW_ID);
      if (row && e.target instanceof Node && !row.contains(e.target)) {
        movedOn.current = true;
        setRecording(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onBlur);
    document.addEventListener("focusin", onFocusIn);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("focusin", onFocusIn);
      // Put focus back on the button that started it, unless the person moved on.
      if (!movedOn.current) setTimeout(() => document.getElementById(ROW_ID)?.querySelector<HTMLElement>(".w-short button")?.focus(), 0);
    };
    // apply only uses setters and stable commands
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  const atDefault = !!status && status.registered && status.shortcut === DEFAULT_SHORTCUT;
  return (
    <WSettingsCard
      id="capture.hotkey"
      title={t("Hotkey panel")}
      headingLevel={3}
      description={t("Press this shortcut anywhere to ask about the window you are in.")}
      loading={!status && !note}
      error={!status ? note : null}
    >
      {status && (
        <>
          <WSettingsStatusRow
            label={t("Status")}
            status={status.registered ? { tone: "good", children: hotkeyStatusLine(status) } : { tone: "warning", children: t("Not registered") }}
            description={status.registered ? undefined : hotkeyStatusLine(status)}
          />
          <WSettingsShortcutRow
            id={ROW_ID}
            label={t("Shortcut")}
            keys={displayShortcut(status.shortcut)}
            recording={recording}
            recordingText={t("Press a shortcut…")}
            disabled={busy}
            disabledReason={t("Saving…")}
            error={note}
            onRecord={() => {
              setNote(null);
              setDone("");
              setRecording(true);
            }}
            onCancel={() => setRecording(false)}
          />
          <div role="status" className="rs-membottom-announce">
            {done}
          </div>
          <WSettingsRow
            variant="button"
            label={t("Default shortcut")}
            actionLabel={t("Reset to default")}
            disabled={busy || atDefault}
            disabledReason={busy ? t("Saving…") : t("Already using the default shortcut.")}
            onPress={() => {
              setRecording(false);
              setNote(null);
              apply(DEFAULT_SHORTCUT);
            }}
          />
        </>
      )}
    </WSettingsCard>
  );
}
