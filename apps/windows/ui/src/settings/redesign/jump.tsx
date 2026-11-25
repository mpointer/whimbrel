// "You jumped here from search": the frame holds the anchor of the row a search result opened, and a row passes
// `highlighted={useJump("capture.localOnly")}` to its part, which draws the mark and takes focus.
import { createContext, useContext } from "react";

export const JumpContext = createContext<string | null>(null);

/** True while the search result for this anchor is the one just opened. */
export function useJump(anchor: string): boolean {
  return useContext(JumpContext) === anchor;
}
