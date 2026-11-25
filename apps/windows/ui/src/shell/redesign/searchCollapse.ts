// The header's search field at a middle width, where it collapses to an icon button (see shell.css, the `shell` container band
// of 40 to 50 rem). This decides when the collapsed field is drawn open. Wider or narrower than the band the field is always
// drawn, whatever this says, because the rules that hide it live only in that band.

export interface SearchShownInput {
  /** The icon or Ctrl+K asked for it and it has not been closed since. */
  asked: boolean;
  /** What the field holds. */
  text: string;
  /** The Find page is showing: its results are for these words. */
  findShown: boolean;
}

export function searchShown({ asked, text, findShown }: SearchShownInput): boolean {
  return asked || findShown || text.trim() !== "";
}

/** Whether a key press puts the open search field away: Escape, pressed with focus in the field itself (not on the Ask button beside
 *  it, which sits in the same box), with nothing typed, and not on the Find page, where the field stays. */
export function escapeCloses(input: { key: string; inField: boolean; text: string; findShown: boolean }): boolean {
  return input.key === "Escape" && input.inField && input.text.trim() === "" && !input.findShown;
}
