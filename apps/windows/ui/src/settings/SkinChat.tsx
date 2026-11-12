import { useEffect, useReducer, useRef, useState, useSyncExternalStore } from "react";
import {
  skinChatLoad,
  skinChatPost,
  skinImageAddFile,
  skinImageApprove,
  skinImageGenerate,
  skinImageReject,
  skinImageSearch,
  skinImageThumbnail,
  skinImageUnshown,
  skinSources,
  skinTurn,
  skinTurnStop,
  skinUndo,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ImageCandidateView, SkinLine, SkinSources } from "../generated/types";
import { useL10n } from "../l10n";
import { chooseSkin, currentSelection, installedSkins, reloadSkins, skinRevision, subscribeSkin } from "../skin";
import { chatReducer, composer, initialChat, keepsOffered, placeholder, providerLine, runTurn, type ChatEvent, type SkinChatApi } from "./skinChatModel";
import { pickCard } from "./skinGalleryModel";

const api: SkinChatApi = {
  turn: skinTurn,
  stop: skinTurnStop,
  chat: skinChatLoad,
  search: skinImageSearch,
  generate: skinImageGenerate,
  post: skinChatPost,
};

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** While the set-up state shows, how often to look again for a provider (a key may be added below). */
const SOURCES_POLL_MS = 5000;

/** Whether the chat's messages are hidden; remembered on this PC only. */
const CHAT_HIDDEN = "whimbrel.skinChatHidden";

function readHidden(): boolean {
  try {
    return localStorage.getItem(CHAT_HIDDEN) === "1";
  } catch {
    return false;
  }
}

function saveHidden(hidden: boolean) {
  try {
    localStorage.setItem(CHAT_HIDDEN, hidden ? "1" : "0");
  } catch {
    // The choice just won't be remembered.
  }
}

/** Settings → Appearance: make or change a skin by talking to Whimbrel, below the gallery as on the Mac. One
 *  chat per skin; the shell saves each turn's skin and keeps its chat. Replies are plain text. */
export function SkinChat(props: { onRunning?: (running: boolean) => void } = {}) {
  const { t } = useL10n();
  useSyncExternalStore(subscribeSkin, skinRevision, skinRevision);
  const selected = currentSelection().skinId;
  const [state, dispatch] = useReducer(chatReducer, selected, (id: string) => initialChat(id));
  const latest = useRef(state);
  latest.current = state;
  const runId = useRef<string | null>(null);
  /** `undefined` until the shell answers. */
  const [sources, setSources] = useState<SkinSources | null | undefined>(undefined);
  const [draft, setDraft] = useState("");
  const [hidden, setHiddenState] = useState(readHidden);
  const setHidden = (next: boolean) => {
    setHiddenState(next);
    saveHidden(next);
  };
  const turnsBox = useRef<HTMLOListElement | null>(null);
  const { onRunning } = props;
  useEffect(() => onRunning?.(state.running), [state.running, onRunning]);
  // The newest message stays in view inside the chat's capped height (only the box scrolls, not the page).
  useEffect(() => {
    const box = turnsBox.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [state.turns.length, hidden]);
  // For the log: found images leaving the strip, however they go (a failed thumbnail, another skin, a new turn).
  const shown = useRef(0);
  useEffect(() => {
    const now = state.candidates.length;
    if (now < shown.current) void skinImageUnshown(`strip went from ${shown.current} to ${now} images`).catch(() => {});
    shown.current = now;
  }, [state.candidates.length]);

  const refreshSources = () =>
    skinSources().then(
      (s) => (setSources(s), s),
      () => (setSources(null), null),
    );

  useEffect(() => {
    void refreshSources();
    const onFocus = () => void refreshSources();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    if (sources?.chat) return;
    const timer = window.setInterval(() => void refreshSources(), SOURCES_POLL_MS);
    return () => window.clearInterval(timer);
  }, [sources?.chat]);

  // The chat follows the selected skin. Picking another skin mid-turn stops the turn (the reducer drops its
  // late reply; this tells the shell not to save it).
  useEffect(() => {
    let live = true;
    if (selected !== latest.current.skinId && latest.current.running && runId.current) void skinTurnStop(runId.current);
    skinChatLoad(selected).then(
      (turns) => live && dispatch({ type: "show", skinId: selected, turns: turns ?? [] }),
      () => live && dispatch({ type: "show", skinId: selected, turns: [] }),
    );
    return () => {
      live = false;
    };
  }, [selected]);

  /** The shell saved a skin: show it, keeping the colour when the same skin changed. */
  const showSaved = async (origin: string, saved: string) => {
    await reloadSkins();
    const now = currentSelection();
    if (now.skinId !== origin) return;
    chooseSkin(saved === origin ? now : pickCard(installedSkins(), now, saved));
  };

  const posted = (skinId: string, line: SkinLine | null) => {
    if (!line) return;
    dispatch({ type: "line", skinId, text: line.text });
    void reloadSkins();
  };

  const failed = (skinId: string) => (e: unknown) => dispatch({ type: "line", skinId, text: message(e) });

  const send = async () => {
    const text = draft.trim();
    if (!text || latest.current.running) return;
    const fresh = await refreshSources();
    // A second submit during the wait finds the first one running.
    if (!fresh?.chat || latest.current.running) return;
    setDraft("");
    // Asking something shows the chat, so the answer is seen.
    if (hidden) setHidden(false);
    const origin = latest.current.skinId;
    dispatch({ type: "send", text, providerLine: providerLine(fresh) });
    const run = latest.current.run + 1;
    const id = `${Date.now().toString(36)}-${run}`;
    runId.current = id;
    const forward = (event: ChatEvent) => {
      dispatch(event);
      if (event.type === "replied" && event.result.savedSkinId) void showSaved(origin, event.result.savedSkinId);
    };
    await runTurn(api, { run, runId: id, skinId: origin, request: text }, forward, () => latest.current.run === run);
  };

  function stop() {
    dispatch({ type: "stop" });
    if (runId.current) void skinTurnStop(runId.current);
  }

  const approve = (candidate: ImageCandidateView) => {
    const skinId = latest.current.skinId;
    skinImageApprove(skinId, candidate.url).then((line) => {
      // Refused in local-only mode, the image stays offered, as on the Mac.
      if (!keepsOffered(line)) dispatch({ type: "forget", url: candidate.url });
      posted(skinId, line);
    }, failed(skinId));
  };

  const reject = (candidate: ImageCandidateView) => {
    dispatch({ type: "forget", url: candidate.url });
    void skinImageReject(candidate.url);
  };

  const addImage = () => skinImageAddFile(selected).then((line) => posted(selected, line), failed(selected));
  const undo = () => skinUndo(selected).then((line) => posted(selected, line), failed(selected));
  const setUp = () => document.getElementById("ai-heading")?.scrollIntoView({ behavior: "smooth", block: "start" });

  return (
    <div className="skin-chat">
      <div className="skin-chat__header">
        <h3 className="gallery-heading muted small">{t("Make a skin")}</h3>
        {state.turns.length > 0 && (
          <button type="button" className="link-button small" aria-expanded={!hidden} aria-controls="skin-chat-turns" onClick={() => setHidden(!hidden)}>
            {hidden ? `${t("Show chat")} (${state.turns.length})` : t("Hide chat")}
          </button>
        )}
      </div>
      {state.turns.length > 0 && !hidden && (
        <ol id="skin-chat-turns" ref={turnsBox} className="skin-chat__turns" tabIndex={0} aria-label={t("Chat about this skin")}>
          {state.turns.map((turn, i) => (
            <li key={i} className={turn.role === "user" ? "skin-chat__bubble skin-chat__bubble--user" : "skin-chat__bubble"}>
              {turn.text}
            </li>
          ))}
        </ol>
      )}
      {state.status.length > 0 && (
        <div className="skin-chat__status" role="status" aria-label={t("Progress")}>
          {state.status.map((line, i) => (
            <p key={i} className="muted small">
              {state.running && i === state.status.length - 1 && <span className="skin-chat__spinner" aria-hidden="true" />}
              {line}
            </p>
          ))}
          {state.running && (
            <button type="button" className="mode" onClick={stop}>
              {t("Stop")}
            </button>
          )}
        </div>
      )}
      {state.candidates.length > 0 && (
        <div className="skin-chat__found">
          <p className="small skin-chat__found-heading">{t("Found these")}</p>
          <div className="skin-chat__strip">
            {state.candidates.map((c) => (
              <Candidate key={c.url} candidate={c} onApprove={() => approve(c)} onReject={() => reject(c)} onBroken={() => dispatch({ type: "forget", url: c.url })} />
            ))}
          </div>
        </div>
      )}
      {composer(sources) !== "set-up" ? (
        <form
          className="field-row"
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <input className="input grow" aria-label={t("Message to Whimbrel about this skin")} placeholder={placeholder(selected)} value={draft} onChange={(e) => setDraft(e.target.value)} />
          <button type="submit" className="skin-chat__send mode mode--active" disabled={state.running || !draft.trim() || composer(sources) === "loading"}>
            {t("Send")}
          </button>
        </form>
      ) : (
        <button type="button" className="skin-chat__set-up" onClick={setUp}>
          {t("Set up an AI provider")} <span aria-hidden="true">↗</span>
        </button>
      )}
      <div className="field-row">
        <button type="button" className="mode" onClick={() => void addImage()}>
          {t("Add your own image…")}
        </button>
        <button type="button" className="mode" disabled={state.running} onClick={() => void undo()}>
          {t("Undo")}
        </button>
      </div>
    </div>
  );
}

/** A found or made image with Approve and Reject. Its thumbnail comes from the shell's guarded fetch; one that
 *  won't load leaves the strip. */
function Candidate(props: { candidate: ImageCandidateView; onApprove: () => void; onReject: () => void; onBroken: () => void }) {
  const { t } = useL10n();
  const { candidate, onBroken } = props;
  const [src, setSrc] = useState<string | null>(null);
  /** What arrived, for the log line when the image won't show. */
  const got = useRef("");
  useEffect(() => {
    let url: string | null = null;
    let live = true;
    skinImageThumbnail(candidate.thumbnailUrl).then(
      (bytes) => {
        if (!live) return;
        got.current = `${bytes.byteLength} bytes`;
        url = URL.createObjectURL(new Blob([bytes]));
        setSrc(url);
      },
      (e: unknown) => {
        // The shell logs its own fetch failures; anything else (the call itself failing) is logged here.
        if (!isCommandError(e)) void skinImageUnshown(`thumbnail call failed: ${String(e)}`).catch(() => {});
        if (live) onBroken();
      },
    );
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [candidate.thumbnailUrl]);
  return (
    <figure className="skin-candidate">
      {src ? (
        <img
          src={src}
          alt={candidate.title}
          onError={() => {
            void skinImageUnshown(`the image didn't decode (${got.current})`).catch(() => {});
            onBroken();
          }}
        />
      ) : <div className="skin-candidate__blank" aria-label={candidate.title} />}
      <figcaption className="muted small">{candidate.title}</figcaption>
      <div className="field-row">
        <button type="button" className="mode mode--active" onClick={props.onApprove}>
          {t("Approve")}
        </button>
        <button type="button" className="mode" onClick={props.onReject}>
          {t("Reject")}
        </button>
      </div>
    </figure>
  );
}
