// A short main window: the CSS height of the viewport, not the text size. At a large Windows text size WebView2 shrinks the CSS
// viewport (the root font stays 16 px), so the main window at 200% has about half the CSS space it has at 100%: 550x360 for a window
// that opens at 1100x720. The top navigation, the title, the search field, the Memory state and the filter chips then take most of
// that height, so below this height the header rows scroll away with the content and only one compact row stays pinned.

import { useEffect, useState } from "react";

/** The CSS height below which the window counts as short. A shell with its top navigation and every header row needs about 440 px
 *  before the first content row has room; below 480 the content would start at the very bottom. */
export const SHORT_VIEWPORT_PX = 480;

/** Whether a viewport height (CSS px) is short. Pure, so the threshold is testable. */
export function isShortViewport(heightPx: number): boolean {
  return heightPx < SHORT_VIEWPORT_PX;
}

/** Whether the window is short now, watched so a resize (or a change of text size, which resizes the CSS viewport) counts. */
export function useShortViewport(): boolean {
  const read = () => typeof window !== "undefined" && isShortViewport(window.innerHeight);
  const [short, setShort] = useState(read);
  useEffect(() => {
    const update = () => setShort(read());
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  return short;
}
