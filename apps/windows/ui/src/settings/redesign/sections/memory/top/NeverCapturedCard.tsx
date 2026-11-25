// The Never captured card: excluded apps (the chooser and the list) and the text-recognition (OCR) languages.
// Nothing here is collapsed or small. Commands: captureSettings, setExcludedApps.

import { useEffect, useState } from "react";
import { captureSettings, setExcludedApps } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { CaptureSettings } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WButton, WSettingsCard, WSettingsControlRow, WSettingsList, WSettingsListRow, WSettingsNote, WSettingsSelectRow } from "../../../../../system";
import { excludeCandidates, ocrLine } from "./memoryTopModel";
import "./memtop.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export function NeverCapturedCard() {
  const { t } = useL10n();
  const [settings, setSettings] = useState<CaptureSettings | null>(null);
  const [pick, setPick] = useState("");
  const [error, setError] = useState<string | null>(null);
  // A save is in flight: a second press would build its list from the old one and drop an app. The buttons say so
  // (aria-disabled) and ignore presses; they are not `disabled`, which would drop keyboard focus out of the card.
  const [saving, setSaving] = useState(false);
  // What the last change did, in a live region that is always there.
  const [said, setSaid] = useState("");

  const load = () =>
    captureSettings().then(
      (s) => {
        setSettings(s);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  useEffect(() => {
    void load();
  }, []);

  const save = (excluded: string[]) => {
    if (saving) return Promise.resolve(false);
    setSaving(true);
    return setExcludedApps(excluded)
      .then(
        (list) => {
          setSettings((s) => (s ? { ...s, excluded: list } : s));
          setError(null);
          return true;
        },
        (e: unknown) => {
          setError(message(e));
          return false;
        },
      )
      .finally(() => setSaving(false));
  };
  /** After a removal the row that had focus is gone: focus goes to the next Remove button, or to the chooser. */
  const focusAfterRemoval = () =>
    setTimeout(() => {
      const row = document.getElementById("rs-memtop-exclude-row");
      const next = document.getElementById("rs-memtop-excluded")?.querySelector<HTMLElement>("button");
      (next ?? row?.querySelector<HTMLElement>("select"))?.focus();
    }, 0);

  const ocr = settings ? ocrLine(settings.ocr) : null;
  return (
    <WSettingsCard headingLevel={3} title={t("Never captured")} description={t("Memory skips the apps listed here.")} loading={settings === null && error === null} error={error}>
      {settings ? (
        <>
          <div role="status" className="rs-memtop-announce">
            {said}
          </div>
          <WSettingsList id="rs-memtop-excluded" label={t("Excluded apps")} isEmpty={settings.excluded.length === 0} empty={t("No apps excluded.")}>
            {settings.excluded.map((id) => (
              <WSettingsListRow
                key={id}
                title={id}
                actions={
                  <WButton
                    variant="quiet"
                    aria-label={`${t("Remove")} ${id}`}
                    aria-disabled={saving ? true : undefined}
                    onClick={() =>
                      void save(settings.excluded.filter((e) => e !== id)).then((ok) => {
                        if (!ok) return;
                        setSaid(t("{app} removed from the list.", { app: id }));
                        focusAfterRemoval();
                      })
                    }
                  >
                    {t("Remove")}
                  </WButton>
                }
              />
            ))}
          </WSettingsList>
          <WSettingsSelectRow
            id="rs-memtop-exclude-row"
            label={t("App to exclude")}
            value={pick}
            options={[{ value: "", label: t("Choose an open app…") }, ...excludeCandidates(settings)]}
            onChange={setPick}
            trailing={
              <span className="rs-memtop-buttons">
                <WButton
                  disabled={!pick}
                  aria-disabled={saving ? true : undefined}
                  aria-describedby={!pick ? "rs-memtop-exclude-why" : undefined}
                  onClick={() => {
                    if (!pick) return;
                    const chosen = pick;
                    void save([...settings.excluded, chosen]).then((ok) => {
                      if (!ok) return;
                      setPick("");
                      setSaid(t("{app} added to the list.", { app: chosen }));
                    });
                  }}
                >
                  {t("Exclude")}
                </WButton>
                <WButton onClick={() => void load()}>{t("Refresh")}</WButton>
              </span>
            }
          />
          {!pick ? <WSettingsNote id="rs-memtop-exclude-why">{t("Choose an open app first, then press Exclude.")}</WSettingsNote> : null}
          {ocr ? (
            <WSettingsControlRow label={t("Text recognition (OCR)")} stacked>
              {/* Same as the old card: a missing language pack is an alert, the covered line is plain text. */}
              <p className="w-setting-desc" role={ocr.covered ? undefined : "alert"}>
                {ocr.text}
              </p>
            </WSettingsControlRow>
          ) : null}
        </>
      ) : null}
    </WSettingsCard>
  );
}
