import { useCallback, useEffect, useState } from "react";
import {
  acceptTopicSuggestion,
  deleteTopicSeed,
  dismissTopicSuggestion,
  mergeTopicSeeds,
  saveTopicSeed,
  topicSeeds,
  topicNamingStatus,
  topicSuggestions,
} from "../api";
import type { TopicSeedView, TopicSuggestionView } from "../generated/types";
import { t } from "../l10n";
import { itemCount, kindLabel, parseAliases, seedDetail, TOPIC_KINDS, TOPICS_REFRESH_MS } from "./topicsModel";

/** What the inline editor is editing: a new topic, or an existing one. */
type Editing = { seed: TopicSeedView | null };

/** Settings → Topics: the topics the user named, with other names and a kind, and the ones
 *  Whimbrel suggests (the Mac's TopicsSettingsPane). */
export function TopicsCard() {
  const [seeds, setSeeds] = useState<TopicSeedView[]>([]);
  const [suggestions, setSuggestions] = useState<TopicSuggestionView[]>([]);
  // Suggested topics still showing a placeholder until a model names them (PR 720).
  const [placeholders, setPlaceholders] = useState(0);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [merging, setMerging] = useState<string | null>(null);
  const [mergeInto, setMergeInto] = useState("");
  const [names, setNames] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    topicSeeds().then(setSeeds, () => {});
    topicSuggestions().then(setSuggestions, () => {});
    topicNamingStatus().then((v) => setPlaceholders(v.placeholderCount), () => {});
  }, []);

  useEffect(() => {
    reload();
    const timer = setInterval(reload, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [reload]);

  /** Runs a write, then reloads; shows `failure` if it fails. */
  const run = (work: Promise<unknown>, failure: string) =>
    work.then(
      () => {
        setError(null);
        reload();
      },
      () => {
        setError(failure);
        reload();
      },
    );

  return (
    <section className="card about stack-sm" aria-labelledby="topics-heading">
      <h2 id="topics-heading" className="section-label">
        {t("Topics")}
      </h2>
      <p className="muted small">
        {t("Name the customers, products and projects you care about. Whimbrel groups your meetings, emails and notes under them.")}
      </p>
      {error && (
        <p role="alert" className="small">
          {error}
        </p>
      )}

      <div className="field-row">
        <h3 className="grow">{t("Your topics")}</h3>
        <button type="button" className="mode" onClick={() => setEditing({ seed: null })}>
          {t("Add topic")}
        </button>
      </div>
      {editing && !editing.seed && (
        <TopicEditor
          seed={null}
          onCancel={() => setEditing(null)}
          onSave={(input) => {
            setEditing(null);
            run(saveTopicSeed({ key: null, ...input }), t("Could not save that topic."));
          }}
        />
      )}
      {seeds.length === 0 && <p className="muted small">{t("No topics yet. Add one, or accept a suggestion below.")}</p>}
      <ul className="stack-sm">
        {seeds.map((seed) => (
          <li key={seed.key} className="stack-sm">
            <div className="field-row">
              <div className="grow">
                <div>{seed.name}</div>
                {seedDetail(seed) && <div className="muted small">{seedDetail(seed)}</div>}
              </div>
              <span className="muted small">{itemCount(seed.itemCount)}</span>
              <button type="button" className="mode" onClick={() => setEditing({ seed })}>
                {t("Edit")}
              </button>
              {seeds.length > 1 && (
                <button
                  type="button"
                  className="mode"
                  onClick={() => {
                    setMerging(seed.key);
                    setDeleting(null);
                    setMergeInto(seeds.find((s) => s.key !== seed.key)?.key ?? "");
                  }}
                >
                  {t("Merge into…")}
                </button>
              )}
              <button
                type="button"
                className="mode"
                onClick={() => {
                  setDeleting(seed.key);
                  setMerging(null);
                }}
              >
                {t("Delete")}
              </button>
            </div>
            {editing?.seed?.key === seed.key && (
              <TopicEditor
                seed={seed}
                onCancel={() => setEditing(null)}
                onSave={(input) => {
                  setEditing(null);
                  run(saveTopicSeed({ key: seed.key, ...input }), t("Could not save that topic."));
                }}
              />
            )}
            {deleting === seed.key && (
              <div className="field-row">
                <span role="alert" className="small grow">
                  {t("Delete {name}? Items stay in your timeline.", { name: seed.name })}
                </span>
                <button
                  type="button"
                  className="mode"
                  onClick={() => {
                    setDeleting(null);
                    run(deleteTopicSeed(seed.key), t("Could not delete that topic."));
                  }}
                >
                  {t("Delete")}
                </button>
                <button type="button" className="mode" onClick={() => setDeleting(null)}>
                  {t("Cancel")}
                </button>
              </div>
            )}
            {merging === seed.key && (
              <div className="field-row">
                <span className="muted small">{t("Merge {name} into", { name: seed.name })}</span>
                <select className="input" aria-label={t("Merge into")} value={mergeInto} onChange={(e) => setMergeInto(e.target.value)}>
                  {seeds
                    .filter((s) => s.key !== seed.key)
                    .map((s) => (
                      <option key={s.key} value={s.key}>
                        {s.name}
                      </option>
                    ))}
                </select>
                <button
                  type="button"
                  className="mode"
                  disabled={!mergeInto}
                  onClick={() => {
                    const keep = mergeInto;
                    setMerging(null);
                    run(mergeTopicSeeds(keep, seed.key), t("Could not merge those topics."));
                  }}
                >
                  {t("Merge")}
                </button>
                <button type="button" className="mode" onClick={() => setMerging(null)}>
                  {t("Cancel")}
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>

      <h3>{t("Suggested")}</h3>
      {placeholders > 0 && <p className="muted small">{t("Some suggested topics have placeholder names until an AI model names them.")}</p>}
      {suggestions.length === 0 && (
        <p className="muted small">
          {t("When a theme shows up across several of your meetings, emails and notes, Whimbrel suggests it here. Nothing is added until you press Add.")}
        </p>
      )}
      <ul className="stack-sm">
        {suggestions.map((s) => {
          const name = names[s.topicId] ?? s.name;
          return (
            <li key={s.topicId} className="field-row">
              <input
                className="input grow"
                aria-label={t("Name")}
                value={name}
                onChange={(e) => setNames({ ...names, [s.topicId]: e.target.value })}
              />
              <span className="muted small">{itemCount(s.itemCount)}</span>
              <button
                type="button"
                className="mode"
                disabled={!name.trim()}
                onClick={() => run(acceptTopicSuggestion(s.topicId, name.trim()), t("Could not add that topic."))}
              >
                {t("Add")}
              </button>
              <button type="button" className="mode" onClick={() => run(dismissTopicSuggestion(s.topicId), t("Could not dismiss that suggestion."))}>
                {t("Dismiss")}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** Add or edit a topic: a name, other names separated by commas, and a kind. */
function TopicEditor(props: {
  seed: TopicSeedView | null;
  onCancel: () => void;
  onSave: (input: { name: string; aliases: string[]; kind: string | null }) => void;
}) {
  const [name, setName] = useState(props.seed?.name ?? "");
  const [aliases, setAliases] = useState(props.seed?.aliases.join(", ") ?? "");
  const [kind, setKind] = useState(props.seed?.kind ?? "");
  return (
    <form
      className="stack-sm key-block"
      onSubmit={(e) => {
        e.preventDefault();
        if (name.trim()) props.onSave({ name: name.trim(), aliases: parseAliases(aliases), kind: kind || null });
      }}
    >
      <h3>{props.seed ? t("Edit topic") : t("Add topic")}</h3>
      <input className="input" aria-label={t("Name")} placeholder={t("Name")} value={name} onChange={(e) => setName(e.target.value)} />
      <input
        className="input"
        aria-label={t("Aliases, separated by commas")}
        placeholder={t("Aliases, separated by commas")}
        value={aliases}
        onChange={(e) => setAliases(e.target.value)}
      />
      <label className="field-row">
        <span>{t("Kind")}</span>
        <select className="input" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">{t("None")}</option>
          {TOPIC_KINDS.map((k) => (
            <option key={k} value={k}>
              {kindLabel(k)}
            </option>
          ))}
        </select>
      </label>
      <div className="field-row">
        <button type="submit" className="mode" disabled={!name.trim()}>
          {t("Save")}
        </button>
        <button type="button" className="mode" onClick={props.onCancel}>
          {t("Cancel")}
        </button>
      </div>
    </form>
  );
}
