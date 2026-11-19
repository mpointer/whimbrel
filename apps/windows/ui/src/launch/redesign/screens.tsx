// The launch's own screens in the Thread look (docs/ui-redesign/screens/onboarding.md): the silent start, the
// migration, the key-unavailable screen and the blocking screens. The same words, the same answers and the same
// calls as ../LaunchScreens.tsx; only the layout and the components change. Colors, fonts and shape come from
// the selected skin.

import { useEffect, useState } from "react";
import type { LastActionView, LaunchAnswer, LaunchScreenView } from "../../generated/types";
import { t } from "../../l10n";
import { WButton, WDialog, WFlowPage, WFlowWait, WMoreMenu, WNotice, WProgressLine } from "../../system";
import { screenContent, type ScreenButton } from "../screenModel";

/** The page color only, then after a second a quiet bar with "Starting Whimbrel…". */
export function StartingScreen({ delayMs = 1000 }: { delayMs?: number }) {
  const [show, setShow] = useState(delayMs <= 0);
  useEffect(() => {
    if (delayMs <= 0) return;
    const timer = setTimeout(() => setShow(true), delayMs);
    return () => clearTimeout(timer);
  }, [delayMs]);
  return show ? (
    <WFlowWait label={t("Starting Whimbrel…")} />
  ) : (
    <div className="w-sys w-flow-root" aria-busy="true" />
  );
}

/** The migration at launch: a bar that never goes back, and no Quit (a crash is still safe). */
export function MigratingScreen({ overall }: { overall: number }) {
  return (
    <WFlowPage
      title={t("Encrypting the memory database")}
      intro={<p>{t("This can take a few minutes on a large memory database. Keep Whimbrel open and your PC awake until it finishes.")}</p>}
      finePrint={<p>{t("If it is interrupted, Whimbrel finishes or undoes it the next time it starts.")}</p>}
    >
      <WProgressLine label={t("Encrypting the memory database")} value={overall} labelHidden />
    </WFlowPage>
  );
}

/** Credential Manager did not answer for an encrypted database. Try again is the only action. */
export function KeyUnavailableScreen({ stillUnavailable, busy, onAnswer }: { stillUnavailable: boolean; busy: boolean; onAnswer: (a: LaunchAnswer) => void }) {
  return (
    <WFlowPage
      title={t("Whimbrel can't read its key right now")}
      intro={
        <>
          <p>
            {t(
              "Windows did not let Whimbrel read the memory database key from Credential Manager. Your key and your data are most likely still there. This is not the same as a lost key, and you do not need your recovery key for it.",
            )}
          </p>
          <p className="w-flow-secondary">{t("This can happen just after you sign in to Windows. Wait a moment, then try again.")}</p>
        </>
      }
      actions={
        <>
          <WButton data-answer="quit" disabled={busy} onClick={() => onAnswer("quit")}>
            {t("Quit Whimbrel")}
          </WButton>
          <WButton variant="primary" data-answer="retry" disabled={busy} onClick={() => onAnswer("retry")}>
            {busy ? t("Trying…") : t("Try again")}
          </WButton>
        </>
      }
      finePrint={<p>{t("Whimbrel has not opened, changed or deleted anything.")}</p>}
      busyLabel={busy ? t("Trying…") : undefined}
      onPrimary={busy ? undefined : () => onAnswer("retry")}
    >
      {stillUnavailable ? <WNotice tone="error">{t("The key still can't be read. If this keeps happening, sign out of Windows and back in, then try again.")}</WNotice> : null}
    </WFlowPage>
  );
}

/** The one primary: the first answer the screen marks as its main one; Quit when it is all there is. */
export function pickPrimary(buttons: readonly ScreenButton[]): ScreenButton | null {
  const main = buttons.find((b) => b.primary);
  if (main) return main;
  return buttons.length === 1 && buttons[0].answer === "quit" ? buttons[0] : null;
}

/** The buttons left to right, the primary last: the others, then Quit, then the primary. */
export function rowOrder(buttons: readonly ScreenButton[]): ScreenButton[] {
  const primary = pickPrimary(buttons);
  const quit = buttons.find((b) => b.answer === "quit" && b !== primary);
  const others = buttons.filter((b) => b !== primary && b !== quit);
  return [...others, ...(quit ? [quit] : []), ...(primary ? [primary] : [])];
}

/** A row never shows more than three controls: past three, the first ones go under More options. */
export function splitRow(row: readonly ScreenButton[]): { menu: ScreenButton[]; shown: ScreenButton[] } {
  return row.length <= 3 ? { menu: [], shown: [...row] } : { menu: row.slice(0, row.length - 2), shown: row.slice(-2) };
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
  const primary = pickPrimary(content.buttons);
  const { menu, shown } = splitRow(rowOrder(content.buttons));
  const draw = (b: ScreenButton) => (
    <WButton key={b.answer} variant={b === primary ? "primary" : "secondary"} data-answer={b.answer} disabled={busy} onClick={() => press(b)}>
      {b.label}
    </WButton>
  );
  return (
    <WFlowPage
      title={content.title}
      intro={
        <>
          {content.paragraphs.map((p) => (
            <p key={p}>{p}</p>
          ))}
          {content.hints.map((h) => (
            <p key={h} className="w-flow-secondary">
              {h}
            </p>
          ))}
        </>
      }
      actions={
        <>
          {menu.length > 0 ? <WMoreMenu disabled={busy} items={menu.map((b) => ({ key: b.answer, label: b.label, disabled: busy, onPress: () => press(b) }))} /> : null}
          {shown.map(draw)}
        </>
      }
      onPrimary={primary && !busy && !asking ? () => press(primary) : undefined}
    >
      {content.lastAction ? <WNotice tone="error">{content.lastAction}</WNotice> : null}
      {asking?.confirm ? (
        <WDialog title={asking.confirm.title} body={asking.confirm.body} onCancel={() => setAsking(null)}>
          <WButton data-initial="" onClick={() => setAsking(null)}>
            {t("Cancel")}
          </WButton>
          <WButton
            variant={asking.confirm.destructive ? "destructive" : "primary"}
            disabled={busy}
            onClick={() => {
              const answer = asking.answer;
              setAsking(null);
              onAnswer(answer);
            }}
          >
            {asking.confirm.confirm}
          </WButton>
        </WDialog>
      ) : null}
    </WFlowPage>
  );
}
