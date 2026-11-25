// The 24 words and their QR (brief 3.3.1 step 4, 3.3.2). They ARE the key: shown and printed;
// selecting, copying and dragging them is refused. No clipboard API, no logging.

import { createPortal } from "react-dom";
import type { SyntheticEvent } from "react";
import { t } from "../l10n";
import { qrOfWords } from "./qr";

const refuse = (e: SyntheticEvent) => e.preventDefault();

/** The words numbered 1. to 24., in 3 columns of 8 (down each column), monospace. */
export function WordsGrid({ words, className = "recovery-words" }: { words: readonly string[]; className?: string }) {
  return (
    <ol className={`${className} mono`} onCopy={refuse} onCut={refuse} onContextMenu={refuse} onDragStart={refuse}>
      {words.map((w, i) => (
        <li key={i}>
          <span className="recovery-word-number">{`${i + 1}.`}</span> {w}
        </li>
      ))}
    </ol>
  );
}

/** The QR of the words, black on a white quiet zone on every theme. */
export function WordsQr({ words, px }: { words: readonly string[]; px: number }) {
  const qr = qrOfWords(words);
  return (
    <svg
      className="recovery-qr"
      role="img"
      aria-label={t("QR code of the 24 words")}
      width={px}
      height={px}
      viewBox={`0 0 ${qr.size} ${qr.size}`}
      shapeRendering="crispEdges"
      onContextMenu={refuse}
      onDragStart={refuse}
    >
      <rect width={qr.size} height={qr.size} className="recovery-qr-quiet" />
      <path d={qr.path} className="recovery-qr-modules" />
    </svg>
  );
}

/** The two sentences of the printed page that name what the words open. Left out, the launch's own ("the memory
 *  database"); the redesigned Settings card passes "your library" ones. */
export interface PrintCopy {
  notice: string;
  footer: string;
}

/** The printed page (port of the Mac's EncryptionPrintPage): title, the words-are-the-key notice,
 *  the words, the QR at 200 px and the footer. Black on white; only visible in print media. */
export function PrintPage({ words, copy }: { words: readonly string[]; copy?: PrintCopy }) {
  return (
    <div className="print-page" aria-hidden="true">
      <h1>{t("Whimbrel recovery words")}</h1>
      <p className="print-page-notice">
        {copy?.notice ??
          t(
            "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.",
          )}
      </p>
      <WordsGrid words={words} className="print-page-words" />
      <WordsQr words={words} px={200} />
      <p className="print-page-footer">
        {copy?.footer ?? t("Keep this page somewhere safe and offline. Anyone who has it can read the memory database of the PC it was made on.")}
      </p>
    </div>
  );
}

/** The print page, beside the app's root so `@media print` can hide everything else. Rendered only
 *  while the words exist, and gone when they are cleared. */
export function PrintPortal({ words, copy }: { words: readonly string[]; copy?: PrintCopy }) {
  if (typeof document === "undefined") return null;
  return createPortal(<PrintPage words={words} copy={copy} />, document.body);
}
