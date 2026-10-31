import { useEffect, useState } from "react";
import {
  profileAcceptSuggestion,
  profileAddFact,
  profileDismissSuggestion,
  profileGet,
  profileRemoveFact,
  profileSetFields,
  profileSetSwitches,
  profileSuggestions,
  profileUpdateFact,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { Fact, ProfileView, SuggestionView } from "../generated/types";
import { useL10n } from "../l10n";
import {
  addFactBlocker,
  canAddFact,
  counterLine,
  factsCaption,
  fieldsDirty,
  isOverCap,
  previewText,
  sourceLabel,
  suggestPrivacy,
  suggestStatus,
  suggestionsEmpty,
  toDraft,
  type FieldsDraft,
} from "./profileModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Profile: who you are, as background for summaries and answers. */
export function ProfileCard() {
  const { t, locale } = useL10n();
  const [view, setView] = useState<ProfileView | null>(null);
  const [draft, setDraft] = useState<FieldsDraft | null>(null);
  const [suggestions, setSuggestions] = useState<SuggestionView[]>([]);
  const [newFact, setNewFact] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    profileGet()
      .then((v) => {
        setView(v);
        setDraft(toDraft(v));
      })
      .catch((e: unknown) => setError(message(e)));
    profileSuggestions()
      .then(setSuggestions)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  if (!view || !draft) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;

  /** Runs a change that returns the new view; true when it was saved. A refusal is shown and nothing changes. */
  const run = (change: Promise<ProfileView>): Promise<boolean> =>
    change.then(
      (v) => {
        setView(v);
        setError(null);
        return true;
      },
      (e: unknown) => {
        setError(message(e));
        return false;
      },
    );

  const saveFields = () => {
    if (fieldsDirty(view, draft)) void run(profileSetFields(draft));
  };

  const setField = (key: keyof FieldsDraft, value: string) => setDraft({ ...draft, [key]: value });

  const accept = (s: SuggestionView) =>
    run(profileAcceptSuggestion(s.id)).then((saved) => {
      if (saved) setSuggestions((all) => all.filter((x) => x.id !== s.id));
    });

  const dismiss = (s: SuggestionView) =>
    profileDismissSuggestion(s.id).then(
      () => {
        setSuggestions((all) => all.filter((x) => x.id !== s.id));
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );

  const addBlocker = addFactBlocker(view, newFact);

  return (
    <section className="card about" aria-labelledby="profile-heading">
      <h2 id="profile-heading" className="section-label">
        {t("Profile")}
      </h2>
      <p className="muted small">
        {t(
          "Background about you, added to summaries, answers and digests so they fit you. It is stored on this PC and reaches a model only through the AI routing you chose (a hosted model sees it unless Local-only is on). It is context, not instructions.",
        )}
      </p>
      <label className="toggle">
        <input
          type="checkbox"
          checked={view.enabled}
          onChange={(e) => void run(profileSetSwitches({ enabled: e.target.checked, suggestEnabled: view.suggestEnabled }))}
        />
        {t("Use my profile in summaries and answers")}
      </label>

      <div className="stack-sm">
        <label className="field">
          <span>{t("Name")}</span>
          <input className="input" value={draft.name} onChange={(e) => setField("name", e.target.value)} onBlur={saveFields} />
        </label>
        <label className="field">
          <span>{t("Role")}</span>
          <input className="input" value={draft.role} onChange={(e) => setField("role", e.target.value)} onBlur={saveFields} />
        </label>
        <label className="field">
          <span>{t("Organization")}</span>
          <input
            className="input"
            value={draft.organization}
            onChange={(e) => setField("organization", e.target.value)}
            onBlur={saveFields}
          />
        </label>
        <label className="field">
          <span>{t("About me and my work")}</span>
          <textarea
            className="input"
            rows={3}
            value={draft.about}
            onChange={(e) => setField("about", e.target.value)}
            onBlur={saveFields}
          />
        </label>
        <p className={isOverCap(view) ? "small" : "muted small"} role={isOverCap(view) ? "alert" : undefined}>
          {counterLine(view, locale)}
        </p>
      </div>

      <h3 className="section-label">{t("Learned facts")}</h3>
      <p className="muted small">{factsCaption(view)}</p>
      {view.facts.length > 0 && (
        <ul className="fact-list">
          {view.facts.map((f) => (
            <FactRow
              key={f.id}
              fact={f}
              onSave={(text) => run(profileUpdateFact(f.id, text))}
              onRemove={() => void run(profileRemoveFact(f.id))}
            />
          ))}
        </ul>
      )}
      <form
        className="field-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (canAddFact(view, newFact)) void run(profileAddFact(newFact)).then((saved) => saved && setNewFact(""));
        }}
      >
        <input
          className="input fact-input"
          aria-label={t("Add a fact")}
          placeholder={t("e.g. Works in US Eastern time")}
          value={newFact}
          onChange={(e) => setNewFact(e.target.value)}
        />
        <button type="submit" className="mode" disabled={!canAddFact(view, newFact)}>
          {t("Add fact")}
        </button>
      </form>
      {addBlocker && <p className="muted small">{addBlocker}</p>}

      <h3 className="section-label">{t("Suggested")}</h3>
      <label className="toggle">
        <input
          type="checkbox"
          checked={view.suggestEnabled}
          onChange={(e) => void run(profileSetSwitches({ enabled: view.enabled, suggestEnabled: e.target.checked }))}
        />
        {t("Suggest additions from my meetings")}
      </label>
      <p className="muted small">{suggestPrivacy()}</p>
      <p className="small">{suggestStatus(view)}</p>
      {suggestions.length === 0 ? (
        <p className="muted small">{suggestionsEmpty()}</p>
      ) : (
        <ul className="fact-list">
          {suggestions.map((s) => (
            <li key={s.id} className="fact-row">
              <div className="fact-text">
                <span>{s.text}</span>
                {sourceLabel(s) && <span className="muted small">{sourceLabel(s)}</span>}
              </div>
              <button className="mode" onClick={() => void accept(s)}>
                {t("Add")}
              </button>
              <button className="mode" onClick={() => void dismiss(s)}>
                {t("Dismiss")}
              </button>
            </li>
          ))}
        </ul>
      )}

      <h3 className="section-label">{t("What Whimbrel sees")}</h3>
      <p className="muted small">{t("Exactly what is added to a prompt, after the {limit}-character limit.", { limit: view.maxChars.toLocaleString(locale) })}</p>
      <pre className="profile-preview" aria-label={t("What Whimbrel sees")}>
        {previewText(view)}
      </pre>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

function FactRow(props: { fact: Fact; onSave: (text: string) => Promise<boolean>; onRemove: () => void }) {
  const { t } = useL10n();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(props.fact.text);

  if (!editing) {
    return (
      <li className="fact-row">
        <span className="fact-text">{props.fact.text}</span>
        <button
          className="mode"
          onClick={() => {
            setText(props.fact.text);
            setEditing(true);
          }}
        >
          {t("Edit")}
        </button>
        <button className="mode" onClick={props.onRemove}>
          {t("Remove")}
        </button>
      </li>
    );
  }
  return (
    <li className="fact-row">
      <input className="input fact-input" aria-label={t("Edit fact")} value={text} onChange={(e) => setText(e.target.value)} autoFocus />
      <button className="mode" onClick={() => void props.onSave(text).then((saved) => saved && setEditing(false))}>
        {t("Save")}
      </button>
      <button className="mode" onClick={() => setEditing(false)}>
        {t("Cancel")}
      </button>
    </li>
  );
}
