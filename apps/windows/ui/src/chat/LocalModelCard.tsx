import { useEffect, useState } from "react";
import { aiSettings, providerModels } from "../api";
import type { AiSettings } from "../generated/types";
import { t } from "../l10n";
import { onRampOnceKnown } from "../settings/aiModel";
import { percent } from "../settings/searchModel";
import { cardState, localModelDownload } from "./localModel";

/** The empty Ask screen with nothing set up: one button for a model that runs here, and the way to
 *  a better one (the Mac's `LocalModelCard`, W-1.3). */
export function LocalModelCard({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const { status, job } = localModelDownload.use();
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [ollamaUp, setOllamaUp] = useState<boolean | null>(null);

  useEffect(() => {
    aiSettings().then(setSettings, () => {});
    providerModels("ollama").then(
      (list) => setOllamaUp(list.length > 0),
      () => setOllamaUp(false),
    );
  }, []);

  const state = cardState(!!settings && onRampOnceKnown(settings.routing, settings.keys, ollamaUp, status ? status.installed : null) === true, status, job);
  if (state === "hidden") return null;

  return (
    <section className="card notice" aria-label={t("Ask needs a model.")}>
      <h2 className="section-label">{t("Ask needs a model.")}</h2>
      <p className="muted small">
        {t("A small model runs on this PC with no account. Answers are slower and plainer than a hosted model's. It needs about 1.9 GB.")}
      </p>
      {state === "downloading" ? (
        <div role="status">
          <p>
            {t("Downloading the local model…")} {percent(job!.fraction)}
          </p>
          <progress className="progress" value={job!.fraction} max={1} />
        </div>
      ) : (
        <div className="toolbar">
          {state !== "setup-only" && (
            <button className="mode mode--active" onClick={() => void localModelDownload.start()}>
              {state === "failed" ? t("Try Again") : t("Download a Small Local Model")}
            </button>
          )}
          {onOpenSettings && (
            <button className="mode" onClick={onOpenSettings}>
              {t("Set up an AI provider")}
            </button>
          )}
        </div>
      )}
    </section>
  );
}
