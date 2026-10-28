import { useCallback, useEffect, useState } from "react";
import { aiSettings } from "../api";
import { askAvailable, askModelLabel, composerPrompt } from "./todayModel";

/**
 * Today's Ask launcher: one field that hands its text to Library's Ask as a new conversation. It never
 * answers inline. With no usable AI provider it is one button into Settings.
 */
export function TodayComposer({
  onAsk,
  onOpenSettings,
  placeholder = "Ask Whimbrel about your meetings",
}: {
  onAsk: (prompt: string) => void;
  onOpenSettings: () => void;
  /** The skin's askPlaceholder. */
  placeholder?: string;
}) {
  // null until Settings has been read: nothing is drawn rather than flashing the wrong state.
  const [state, setState] = useState<{ available: boolean; model: string } | null>(null);
  const [text, setText] = useState("");

  const load = useCallback(() => {
    aiSettings()
      .then((s) => setState({ available: askAvailable(s.routing, s.keys), model: askModelLabel(s.routing) }))
      .catch(() => setState(null));
  }, []);

  useEffect(() => {
    load();
    // A key added in Settings is picked up when the window is next focused.
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [load]);

  if (!state) return null;

  if (!state.available) {
    return (
      <button type="button" className="card today-setup" onClick={onOpenSettings} title="Add an API key or choose a local model in Settings">
        <span className="muted">Set up an AI provider</span>
        <span aria-hidden="true">&rarr;</span>
      </button>
    );
  }

  const send = () => {
    const prompt = composerPrompt(text);
    if (!prompt) return;
    setText("");
    onAsk(prompt);
  };

  return (
    <form
      className="card today-composer"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <input
        className="today-composer-field"
        type="text"
        value={text}
        placeholder={placeholder}
        aria-label="Ask Whimbrel"
        onChange={(e) => setText(e.target.value)}
      />
      <span className="muted small today-composer-model" title="The default model for Ask">
        {state.model}
      </span>
      <button type="submit" className="mode mode--active" disabled={!composerPrompt(text)}>
        Ask
      </button>
    </form>
  );
}
