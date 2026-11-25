// Settings > Profile, redrawn on the Settings parts. Same commands, same conditions and the same blockers as the old
// ProfileCard (the model is ../profileModel; the wording layer for the dropped name "AI & routing" is
// profile/profileWording.ts). One card, with the switch, the four fields, the facts, the suggestions and the preview.
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
} from "../../../api";
import { describeError, isCommandError } from "../../../errors";
import type { ProfileView, SuggestionView } from "../../../generated/types";
import { t, useL10n } from "../../../l10n";
import { WButton, WIcon, WSettingsCard, WSettingsControlRow, WSettingsList, WSettingsListRow, WSettingsNote, WSettingsTextRow, WSettingsToggleRow } from "../../../system";
import {
  addFactBlocker,
  counterLine,
  factsCaption,
  factsLine,
  fieldsDirty,
  isOverCap,
  previewText,
  sourceLabel,
  suggestStatus,
  suggestionsEmpty,
  toDraft,
  type FieldsDraft,
} from "../../profileModel";
import { PROFILE_SAVED_EVENT } from "../../ProfileCard";
import type { SectionProps } from "../types";
import { suggestPrivacyRedesign } from "./profile/profileWording";
import "./profile/profile.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export function ProfileSection(_props: SectionProps) {
  const { locale } = useL10n();
  const [view, setView] = useState<ProfileView | null>(null);
  const [draft, setDraft] = useState<FieldsDraft | null>(null);
  const [suggestions, setSuggestions] = useState<SuggestionView[]>([]);
  /** The fact being edited, by id. */
  const [editing, setEditing] = useState<string | null>(null);
  /** Suggestions whose Add or Dismiss is on its way, so the row shows the click registered. */
  const [busy, setBusy] = useState<ReadonlySet<number>>(new Set());
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

  if (!view || !draft) {
    return <WSettingsCard id="profile.profile" title={t("Profile")} headingLevel={3} loading={!error} error={error} />;
  }

  /** Runs a change that returns the new view. Null when it was saved; otherwise the refusal, shown, and nothing changes. */
  const run = (change: Promise<ProfileView>): Promise<string | null> =>
    change.then(
      (v) => {
        setView(v);
        setError(null);
        // The Meetings card's question alerts need a name; it rechecks when the profile changes.
        window.dispatchEvent(new Event(PROFILE_SAVED_EVENT));
        return null;
      },
      (e: unknown) => {
        const m = message(e);
        setError(m);
        return m;
      },
    );

  /** One field was committed: save all four (the saved values with this one changed) when they differ from what is saved.
   *  Resolves false when the save failed, so the row keeps the typed text and tries again on its next blur: what is
   *  compared is the text against the saved view, never against the last thing typed. */
  const saved = toDraft(view);
  const commitField = (key: keyof FieldsDraft, value: string): Promise<boolean> => {
    const next = { ...saved, [key]: value };
    if (!fieldsDirty(view, next)) return Promise.resolve(true);
    return run(profileSetFields(next)).then((failed) => failed === null);
  };

  /** Marks the row busy for the length of `work`; a second click while it's busy does nothing. */
  const whileBusy = (id: number, work: () => Promise<void>): Promise<void> => {
    if (busy.has(id)) return Promise.resolve();
    setBusy((all) => new Set(all).add(id));
    return work().finally(() =>
      setBusy((all) => {
        const next = new Set(all);
        next.delete(id);
        return next;
      }),
    );
  };

  const accept = (s: SuggestionView) =>
    whileBusy(s.id, () =>
      run(profileAcceptSuggestion(s.id)).then((failed) => {
        if (failed === null) setSuggestions((all) => all.filter((x) => x.id !== s.id));
      }),
    );

  const dismiss = (s: SuggestionView) =>
    whileBusy(s.id, () =>
      profileDismissSuggestion(s.id).then(
        () => {
          setSuggestions((all) => all.filter((x) => x.id !== s.id));
          setError(null);
        },
        (e: unknown) => setError(message(e)),
      ),
    );

  const over = isOverCap(view);

  return (
    <WSettingsCard
      id="profile.profile"
      title={t("Profile")}
      headingLevel={3}
      description={t(
        "Background about you, added to summaries, answers and digests so they fit you. It is stored on this PC and reaches a model only through the AI routing you chose (a hosted model sees it unless Local-only is on). It is context, not instructions.",
      )}
      error={error}
    >
      <WSettingsToggleRow
        label={t("Use my profile in summaries and answers")}
        checked={view.enabled}
        onToggle={(next) => void run(profileSetSwitches({ enabled: next, suggestEnabled: view.suggestEnabled }))}
      />
      <WSettingsTextRow label={t("Name")} value={saved.name} onCommit={(v) => commitField("name", v)} />
      <WSettingsTextRow label={t("Role")} value={saved.role} onCommit={(v) => commitField("role", v)} />
      <WSettingsTextRow label={t("Organization")} value={saved.organization} onCommit={(v) => commitField("organization", v)} />
      <WSettingsTextRow label={t("About me and my work")} rows={3} value={saved.about} onCommit={(v) => commitField("about", v)} />
      {over ? (
        <p className="rs-profileplan-over" role="alert">
          <WIcon name="warning" size={16} />
          <span>{counterLine(view, locale)}</span>
        </p>
      ) : (
        <WSettingsNote>{counterLine(view, locale)}</WSettingsNote>
      )}

      <WSettingsList
        heading={4}
        label={t("Learned facts")}
        description={factsCaption(view)}
        isEmpty={view.facts.length === 0}
        empty={factsLine(view)}
        add={{
          label: t("Add a fact"),
          placeholder: t("e.g. Works in US Eastern time"),
          addLabel: t("Add fact"),
          onAdd: (text) => addFactBlocker(view, text) ?? run(profileAddFact(text)),
        }}
      >
        {view.facts.map((f) => (
          <WSettingsListRow
            key={f.id}
            title={f.text}
            edit={
              editing === f.id
                ? {
                    label: t("Edit fact"),
                    value: f.text,
                    onSave: (text) => void run(profileUpdateFact(f.id, text)).then((failed) => failed === null && setEditing(null)),
                    onCancel: () => setEditing(null),
                  }
                : undefined
            }
            actions={
              <>
                <WButton aria-label={`${t("Edit")}: ${f.text}`} onClick={() => setEditing(f.id)}>
                  {t("Edit")}
                </WButton>
                <WButton aria-label={`${t("Remove")}: ${f.text}`} onClick={() => void run(profileRemoveFact(f.id))}>
                  {t("Remove")}
                </WButton>
              </>
            }
          />
        ))}
      </WSettingsList>

      <WSettingsToggleRow
        label={t("Suggest additions from my meetings")}
        description={suggestPrivacyRedesign()}
        checked={view.suggestEnabled}
        onToggle={(next) => void run(profileSetSwitches({ enabled: view.enabled, suggestEnabled: next }))}
      />
      <WSettingsNote>{suggestStatus(view)}</WSettingsNote>
      <WSettingsList heading={4} label={t("Suggested")} isEmpty={suggestions.length === 0} empty={suggestionsEmpty()}>
        {suggestions.map((s) => (
          <WSettingsListRow
            key={s.id}
            title={s.text}
            caption={sourceLabel(s) ?? undefined}
            actions={
              <>
                <WButton aria-label={`${t("Add")}: ${s.text}`} disabled={busy.has(s.id)} onClick={() => void accept(s)}>
                  {t("Add")}
                </WButton>
                <WButton aria-label={`${t("Dismiss")}: ${s.text}`} disabled={busy.has(s.id)} onClick={() => void dismiss(s)}>
                  {t("Dismiss")}
                </WButton>
              </>
            }
          />
        ))}
      </WSettingsList>

      <WSettingsControlRow
        stacked
        heading={4}
        label={t("What Whimbrel sees")}
        description={t("Exactly what is added to a prompt, after the {limit}-character limit.", { limit: view.maxChars.toLocaleString(locale) })}
      >
        <pre className="rs-profileplan-preview" tabIndex={0} aria-label={t("What Whimbrel sees")}>
          {previewText(view)}
        </pre>
      </WSettingsControlRow>
    </WSettingsCard>
  );
}
