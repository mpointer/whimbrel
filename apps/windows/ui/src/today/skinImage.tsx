// A skin's header and badge image on Home: read once from the skin's folder, drawn decoratively.

import { useEffect, useState } from "react";
import { skinImage } from "../api";
import { isDrawn, type Imagery, type ImagePlacement } from "./layoutModel";
import "./skinImage.css";

/** An approved image of the skin as an object URL, or null while it loads or when it cannot be read. */
export function useSkinImage(skinId: string, asset: string | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!asset) {
      setUrl(null);
      return;
    }
    let live = true;
    let made: string | null = null;
    skinImage(skinId, asset)
      .then((bytes) => {
        if (!live) return;
        made = URL.createObjectURL(new Blob([bytes]));
        setUrl(made);
      })
      .catch(() => live && setUrl(null));
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [skinId, asset]);
  return url;
}

/** The skin's image for one placement: a header band 120 px tall under a scrim (none when the skin marks it text-free) or a
 *  small badge beside the text. Always decorative: empty alt text and hidden from Narrator. Draws nothing, and takes no space,
 *  when the skin has none or it cannot be read (a bad asset name is refused by the backend and lands here as null). */
export function SkinImage({ imagery, skinId, placement }: { imagery: Imagery[]; skinId: string; placement: ImagePlacement }) {
  const entry = imagery.find((i) => i.placement === placement);
  const url = useSkinImage(skinId, entry?.asset ?? null);
  return <SkinImageView entry={entry} url={url} placement={placement} />;
}

/** What `SkinImage` draws once its file is read: nothing without an entry or a readable file; the picture with empty alt
 *  text inside a hidden wrapper; and the page-colour scrim over it where the entry has one. */
export function SkinImageView({ entry, url, placement }: { entry: Imagery | undefined; url: string | null; placement: ImagePlacement }) {
  if (!entry || !url || !isDrawn(entry)) return null;
  return (
    <div className={`skin-image skin-image--${placement}`} aria-hidden="true">
      <img src={url} alt="" />
      {entry.scrim > 0 && <div className="skin-image__scrim" style={{ opacity: entry.scrim }} />}
    </div>
  );
}
