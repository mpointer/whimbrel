import { t } from "../l10n";
import { bootstrapModels, useModelFooter } from "./modelBootstrap";

/** Sidebar footer: what the background model download is doing (the Mac's `ModelStatusLine`).
 *  Nothing once it's done. */
export function ModelStatusLine() {
  const footer = useModelFooter();
  if (footer.kind === "hidden") return null;
  return (
    <div className="sidebar-footer muted small" role="status">
      {footer.kind === "downloading" ? (
        <>
          <progress className="progress" value={footer.fraction} max={1} />
          <p>{t("Downloading models. Keyword search works now; search by meaning starts when this finishes.")}</p>
        </>
      ) : (
        <p>
          {t("Couldn't download models.")}{" "}
          <button className="link" onClick={bootstrapModels}>
            {t("Try again")}
          </button>
        </p>
      )}
    </div>
  );
}
