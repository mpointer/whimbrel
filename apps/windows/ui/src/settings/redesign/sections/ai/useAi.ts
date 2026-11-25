// The AI section's state: one read of the AI settings, the routing save, the Whimbrel AI balance and the model lists.
// The same commands and the same refresh rules as the old AiCard.

import { useEffect, useState } from "react";
import { aiSettings, providerModels, setRoutingSettings, whimbrelAiRefresh, whimbrelAiStatus } from "../../../../api";
import { localModelDownload } from "../../../../chat/localModel";
import { describeError, isCommandError } from "../../../../errors";
import type { AiSettings, LocalModelStatus, RoutingSettings, WhimbrelAiView } from "../../../../generated/types";
import { ROUTING_CHANGED_EVENT } from "../../../aiModel";

export function aiMessage(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The Whimbrel AI balance: the shell's last read at once, a fresh balance, then the shell's view every 5 s and on focus. */
export function useWhimbrelAi(): WhimbrelAiView | null {
  const [view, setView] = useState<WhimbrelAiView | null>(null);
  useEffect(() => {
    let live = true;
    const show = (v: WhimbrelAiView) => live && setView(v);
    const read = () => void whimbrelAiStatus().then(show, () => undefined);
    const refresh = () => void whimbrelAiRefresh().then(show, () => undefined);
    read();
    refresh();
    const timer = setInterval(read, 5000);
    window.addEventListener("focus", refresh);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, []);
  return view;
}

export interface Ai {
  settings: AiSettings | null;
  setSettings: (next: AiSettings | ((s: AiSettings | null) => AiSettings | null)) => void;
  error: string | null;
  setError: (e: string | null) => void;
  saveRouting: (routing: RoutingSettings) => void;
  /** Null until the Ollama probe answers, so the on-ramp is not drawn and then taken away. */
  ollamaUp: boolean | null;
  local: LocalModelStatus | null;
  setLocalStatus: (s: LocalModelStatus) => void;
  whimbrel: WhimbrelAiView | null;
  whimbrelModels: string[] | null;
  loadWhimbrelModels: () => Promise<void>;
}

export function useAi(): Ai {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ollamaUp, setOllamaUp] = useState<boolean | null>(null);
  const { status: local } = localModelDownload.use();
  const whimbrel = useWhimbrelAi();
  const [whimbrelModels, setWhimbrelModels] = useState<string[] | null>(null);
  const loadWhimbrelModels = () => providerModels("whimbrel").then(setWhimbrelModels);

  useEffect(() => {
    aiSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(aiMessage(e)));
    // "Use Whimbrel AI" (the Plus question) saved a provider: read it, or a later save here would write the old one back.
    const reload = () => void aiSettings().then(setSettings, () => {});
    window.addEventListener(ROUTING_CHANGED_EVENT, reload);
    // Whether the local model answers decides if the on-ramp shows; a failed probe is "no".
    providerModels("ollama").then(
      (list) => setOllamaUp(list.length > 0),
      () => setOllamaUp(false),
    );
    return () => window.removeEventListener(ROUTING_CHANGED_EVENT, reload);
  }, []);

  const saveRouting = (routing: RoutingSettings) => {
    setSettings((s) => s && { ...s, routing });
    setRoutingSettings(routing)
      .then(() => setError(null))
      .catch((e: unknown) => setError(aiMessage(e)));
  };

  return {
    settings,
    setSettings,
    error,
    setError,
    saveRouting,
    ollamaUp,
    local,
    setLocalStatus: localModelDownload.setStatus,
    whimbrel,
    whimbrelModels,
    loadWhimbrelModels,
  };
}
