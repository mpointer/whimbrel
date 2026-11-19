// The QR code of the 24 recovery words (brief 3.3.2): the words joined by single spaces, error
// correction M, as the Mac draws it. Built from qrcode's module matrix and drawn by React as one
// SVG path, so no markup string is ever injected. The QR IS the key: it is shown and printed,
// never copied, saved or logged.

import QRCode from "qrcode";

/** The modules around the code that must stay white (the QR standard's quiet zone). */
export const QUIET_ZONE = 4;

export interface QrShape {
  /** Modules per side, the quiet zone included. */
  size: number;
  /** One SVG path drawing every dark module as a 1x1 square. */
  path: string;
}

/** The QR of `words` (single spaces, error correction M). */
export function qrOfWords(words: readonly string[]): QrShape {
  const code = QRCode.create(words.join(" "), { errorCorrectionLevel: "M" });
  const n = code.modules.size;
  const parts: string[] = [];
  for (let row = 0; row < n; row++) {
    for (let col = 0; col < n; col++) {
      if (code.modules.get(row, col)) parts.push(`M${col + QUIET_ZONE} ${row + QUIET_ZONE}h1v1h-1z`);
    }
  }
  return { size: n + 2 * QUIET_ZONE, path: parts.join("") };
}
