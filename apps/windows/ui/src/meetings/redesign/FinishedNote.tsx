// The finished meeting's note in the Thread look (docs/ui-redesign/screens/note.md), behind the redesign flag. Two views, the
// note and the transcript. In the note the main column reads top to bottom (date and title, notices, your notes, the summary,
// decisions, open questions, action items, marked moments, slides, alerts, ask) and every line Whimbrel wrote that has a source
// carries a numbered mark. Its card sits in the same grid row, so it is lined up with the line and comes right after it for the
// keyboard; a narrow window folds the cards away and shows the selected one under its line.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { meetingUpdateActionItem, recapDraft, recapSent, recapText, type RecapFormat } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { RecapDraft, RecapSentView, TopicRefView } from "../../generated/types";
import { useL10n } from "../../l10n";
import { DeviceChip } from "../../library/LibraryView";
import { spanLabel } from "../../library/rows";
import { setScreenTitle } from "../../shell/redesign/screenTitle";
import { WActionItemRow, WButton, WNoteBlock, WSourceCard } from "../../system";
import { TopicChip, useTopicLookup } from "../../topics/TopicChip";
import { ActionItems } from "../ActionItems";
import { LiveAlertsSection } from "../LiveAlertsSection";
import { MeetingAskBar } from "../MeetingAskBar";
import { disabledReason } from "../meetingAskModel";
import { RecapSheet } from "../RecapActions";
import { recapInput, recapSentLine } from "../recapModel";
import { meetingTitleLabel } from "../sentinels";
import { fallbackLine } from "../summaryFallback";
import { SlidesStrip } from "../SlidesStrip";
import { TrimSheet } from "../TrimSheet";
import { FallbackNotice } from "../Workspace";
import { displayOrder, transcriptionNoticeText, transcriptLanguageOptions } from "../workspaceModel";
import { NoteMenu, type NoteMenuItem } from "./NoteMenu";
import { buildNote, hasWhimbrelLines, holdMargin, marginShown, noteState, offerSources, offsetClock, sourceName, sourceSpeaker, type LineTarget, type NoteMode } from "./noteModel";
import { NoteTranscript, type TranscriptOpen } from "./NoteTranscript";
import { useNoteData } from "./useNoteData";
import "./finishedNote.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export interface FinishedNoteProps {
  meetingId: number;
  /** Open on the transcript at this line (its id as `meeting_transcript_lines` gives it): a search result or an Ask source. */
  initialLine?: number;
  /** Where this page has a way back (a meeting opened from Home, Library, Ask or Topics). The Meetings tab has its list beside it. */
  onBack?: () => void;
  /** The meeting changed or went (forgotten): the list behind reloads. */
  onChanged: () => void;
  onOpenSettings?: () => void;
  /** Told the meeting's phase each time the page reads it, so the pane that holds this page need not poll for it as well. */
  onPhase?: (phase: string | null, readOnly: boolean) => void;
}

/** The slides strip refreshes on its own clock (the old workspace's two seconds), so the page around it does not redraw with it. */
function SlidesRefreshed({ meetingId }: { meetingId: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 2_000);
    return () => clearInterval(timer);
  }, []);
  return <SlidesStrip meetingId={meetingId} refresh={now} />;
}

/** A confirm card that takes focus when it opens and gives it back to what had it (the menu's button) when it closes. */
function ConfirmCard({ label, children }: { label: string; children: ReactNode }) {
  const card = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    card.current?.focus();
    return () => {
      if (before && before !== document.body && document.contains(before)) before.focus();
    };
  }, []);
  return (
    <div ref={card} tabIndex={-1} className="card confirm th-fin-confirm" role="alertdialog" aria-label={label}>
      {children}
    </div>
  );
}

/** The page for one meeting. It is keyed by the meeting, so nothing of the last one (items, trim, an open sheet, an error) carries over. */
export function FinishedNote(props: FinishedNoteProps) {
  return <FinishedNoteBody key={props.meetingId} {...props} />;
}

function FinishedNoteBody({ meetingId, initialLine, onBack, onChanged, onOpenSettings, onPhase }: FinishedNoteProps) {
  const { t, locale } = useL10n();
  const data = useNoteData(meetingId, onChanged);
  const { view } = data;
  const [mode, setMode] = useState<NoteMode>(initialLine != null ? "transcript" : "note");
  const [edit, setEdit] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  const [open, setOpen] = useState<TranscriptOpen | null>(initialLine != null ? { seq: 1, target: { kind: "line", id: initialLine }, focus: true } : null);
  const openSeq = useRef(initialLine != null ? 1 : 0);
  const [signedIn, setSignedIn] = useState<boolean | null>(null);
  const [forgetting, setForgetting] = useState(false);
  const [trimming, setTrimming] = useState(false);
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<ReturnType<typeof setTimeout>>();
  const [sent, setSent] = useState<RecapSentView | null>(null);
  const [sheet, setSheet] = useState<RecapDraft | null>(null);
  const [shareError, setShareError] = useState<string | null>(null);
  const [topic] = useTopicLookup([{ kind: "meeting", id: meetingId }]) as [TopicRefView | null];
  const titleRef = useRef<HTMLHeadingElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  // What to focus when the page comes back from the transcript: a selector for the control that opened it, else the title.
  const returnTo = useRef<string | null>(null);
  const returnFocus = useRef(false);
  // The margin's answer, taken when Edit began (see holdMargin).
  const marginHeld = useRef<boolean | null>(null);

  useEffect(() => {
    let alive = true;
    recapSent(meetingId).then((s) => alive && setSent(s), () => {});
    return () => {
      alive = false;
      clearTimeout(copiedTimer.current);
    };
  }, [meetingId]);
  const phase = view?.phase ?? null;
  const viewReadOnly = view?.readOnly === true;
  useEffect(() => {
    if (view) onPhase?.(phase, viewReadOnly);
    // `onPhase` is the pane's setter; only the phase matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view != null, phase, viewReadOnly]);

  const title = view ? meetingTitleLabel(view.row.title) : null;
  useEffect(() => {
    setScreenTitle(title);
    return () => setScreenTitle(null);
  }, [title]);

  // `focus`: move focus to the target (a jump from the page: a source, a mark, an alert, an Ask citation, Ctrl+T). The view switch's own
  // buttons keep focus where the person pressed.
  const showTranscript = useCallback((target: LineTarget | null, focus: boolean) => {
    if (focus) {
      const from = document.activeElement as HTMLElement | null;
      const markId = from?.getAttribute("data-mark-id");
      returnTo.current = markId != null ? `[data-mark-id="${markId}"]` : from?.closest(".th-fin-card") ? `.w-mark[aria-pressed="true"]` : null;
    } else returnTo.current = null;
    returnFocus.current = focus;
    setMode("transcript");
    setOpen({ seq: ++openSeq.current, target, focus });
  }, []);
  const showNote = useCallback((focus: boolean) => {
    returnFocus.current = focus;
    setMode("note");
    setOpen(null);
  }, []);
  // Back on the note by keyboard or a jump: focus lands on what opened the transcript, else on the title.
  useEffect(() => {
    if (mode !== "note" || !returnFocus.current) return;
    returnFocus.current = false;
    const opener = returnTo.current ? pageRef.current?.querySelector<HTMLElement>(returnTo.current) : null;
    (opener ?? titleRef.current)?.focus();
  }, [mode]);

  // Ctrl+T switches between the two views. Not while a sheet or a confirm is open, not on a held key or mid-composition.
  const dialogOpen = sheet != null || trimming || forgetting || data.conflict;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.metaKey || e.shiftKey || e.altKey || e.key.toLowerCase() !== "t") return;
      if (e.repeat || e.isComposing || e.defaultPrevented || dialogOpen) return;
      e.preventDefault();
      if (mode === "note") showTranscript(null, true);
      else showNote(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, dialogOpen, showTranscript, showNote]);

  if (!view) {
    return (
      <section className="th-fin" aria-label={t("Loading…")}>
        {data.error ? <p role="alert">{data.error}</p> : <p className="th-fin-quiet">{t("Loading…")}</p>}
      </section>
    );
  }

  const readOnly = view.readOnly;
  const summaryText = data.summary ?? view.summary ?? "";
  const items = displayOrder(data.items);
  const model = buildNote({
    decisions: view.decisions,
    decisionsAreEdited: view.decisionsAreEdited,
    openQuestions: view.openQuestions,
    openQuestionsAreEdited: view.openQuestionsAreEdited,
    items,
    sources: data.sources,
    editedItems: data.editedItems,
  });
  const state = noteState({ ...view, summary: summaryText });
  const hasLines = view.decisions.length > 0 || view.openQuestions.length > 0 || items.length > 0;
  const labels = Object.fromEntries(model.entries.map((e) => [e.number, sourceName(e.number, sourceSpeaker(e.source), e.source.startCs)]));
  const select = (n: number) => setSelected((cur) => (cur === n ? null : n));
  const processing = view.processingStage != null;
  const canRegenerate = !readOnly && view.hasTranscript;
  // Whoever can write here can edit, in every state: a summary can be typed where none was written, and the action items kept.
  const editing = edit && !readOnly;
  // Words typed into the summary are the person's from the first keystroke, saved or not.
  const summaryMine = view.summaryIsEdited || summaryText !== (view.summary ?? "");
  const aiWrote = hasWhimbrelLines(model, summaryText, summaryMine);
  // The margin column does not fold or unfold while the person edits: its answer is taken when Edit began and held until Edit ends.
  const margin = holdMargin(marginHeld.current, editing, marginShown(model, aiWrote));
  marginHeld.current = margin.held;
  // "Written by Whimbrel" is said once with a section's heading. A section that mixes authors (action items) says it on each of Whimbrel's rows.
  const allActionsAi = model.actions.length > 0 && model.actions.every((a) => a.ai);

  const copy = (format: RecapFormat) => {
    setShareError(null);
    recapText(recapInput(view, summaryText), format)
      .then((text) => navigator.clipboard.writeText(text))
      .then(
        () => {
          setCopied(true);
          clearTimeout(copiedTimer.current);
          copiedTimer.current = setTimeout(() => setCopied(false), 2000);
        },
        (e: unknown) => setShareError(message(e)),
      );
  };
  const openSheet = () => {
    setShareError(null);
    recapDraft(meetingId, recapInput(view, summaryText)).then(setSheet, (e: unknown) => setShareError(message(e)));
  };

  const moreItems: NoteMenuItem[] = [
    ...(canRegenerate ? [{ id: "regenerate", label: t("Regenerate summary"), disabled: data.busy || processing, onPress: () => data.askThenRegenerate() }] : []),
    ...(view.transcription && !readOnly ? [{ id: "again", label: t("Transcribe again"), disabled: data.busy || processing, onPress: data.transcribeAgain }] : []),
    ...(view.canChooseLanguage
      ? transcriptLanguageOptions(view.spokenLanguage ?? null, t("Automatic")).map((o, i) => ({
          id: `lang-${o.code}`,
          label: o.label,
          heading: i === 0 ? t("Transcript language") : undefined,
          grouped: i > 0,
          checked: (view.spokenLanguage ?? "") === o.code,
          disabled: data.busy || processing,
          onPress: () => data.setLanguage(o.code || null),
        }))
      : []),
    ...(data.trim?.offered && !readOnly ? [{ id: "trim", label: t("Trim recording…"), disabled: data.busy || processing, onPress: () => setTrimming(true) }] : []),
    { id: "forget", label: t("Forget meeting"), disabled: data.busy, onPress: () => setForgetting(true) },
  ];

  const card = (n: number | null) => {
    const entry = n == null ? null : model.entries.find((e) => e.number === n);
    if (!entry) return null;
    const { source } = entry;
    return (
      // A press on the card (not only on its mark) selects it.
      // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
      <div className="th-fin-card" data-selected={selected === n ? "true" : undefined} onClick={() => setSelected(n)}>
        <WSourceCard
          number={entry.number}
          appName={sourceSpeaker(source)}
          time={offsetClock(source.startCs)}
          quote={source.text}
          selected={selected === n}
          openLabel={t("Open in transcript at {time}", { time: offsetClock(source.startCs) })}
          onOpen={() => showTranscript({ kind: "line", id: source.segmentId }, true)}
        />
      </div>
    );
  };
  const row = (key: string, line: ReactNode, n: number | null, Tag: "div" | "li" = "div") => (
    <Tag className="th-fin-row" key={key} data-sourced={n != null ? "true" : undefined}>
      <div className="th-fin-line">{line}</div>
      {card(n)}
    </Tag>
  );
  const lineBlock = (l: { key: string; text: string; ai: boolean; number: number | null }) =>
    row(
      l.key,
      <WNoteBlock
        kind="paragraph"
        author={l.ai ? "whimbrel" : "you"}
        text={l.text}
        sources={l.number != null ? [l.number] : []}
        selected={l.number != null && selected === l.number}
        selectedSource={selected}
        onSelectSource={select}
        sourceLabels={labels}
        announceAi={false}
      />,
      l.number,
    );

  // The notices that exist today, never hidden: they sit above both views, so a run in progress or a failure shows in the transcript too.
  const notices = (
    <div className="th-fin-notices">
      {processing ? (
        <p role="status" className="th-fin-notice">
          {view.processingStage}
        </p>
      ) : null}
      {view.summaryFallback ? (
        <FallbackNotice
          line={fallbackLine(view.summaryFallback)}
          signedIn={signedIn}
          onSignedIn={setSignedIn}
          busy={data.busy}
          canResummarize={view.hasTranscript && !readOnly}
          error={data.resummarizeError}
          onResummarize={() => data.askThenRegenerate(true)}
          onOpenSettings={onOpenSettings}
        />
      ) : null}
      {view.transcription ? (
        <div className="th-fin-notice" role="status">
          <p>{transcriptionNoticeText(view.transcription)}</p>
          <WButton disabled={data.busy || processing} onClick={data.transcribeAgain}>
            {t("Transcribe again")}
          </WButton>
        </div>
      ) : null}
      {data.namesChanged && !readOnly ? (
        <div className="th-fin-notice" role="status">
          <p>{t("Names changed. Update the summary?")}</p>
          <WButton
            variant="primary"
            disabled={data.busy || processing}
            onClick={() => {
              data.setNamesChanged(false);
              data.askThenRegenerate();
            }}
          >
            {t("Update the summary")}
          </WButton>
        </div>
      ) : null}
      {view.summaryFailure != null ? (
        <p className="th-fin-notice" role="status">
          {t("The summary didn't finish. Regenerate summary tries again. What went wrong: {reason}", { reason: view.summaryFailure })}
        </p>
      ) : null}
      {sent ? <p className="th-fin-notice">{recapSentLine(sent.recipientCount)}</p> : null}
      {readOnly ? <p className="th-fin-notice">{t("Recorded on another device, so its audio and summary live there.")}</p> : null}
    </div>
  );

  const noteBody = (
    <div className="th-fin-page" ref={pageRef} data-has-margin={margin.shown ? "true" : "false"}>
      <div className="th-fin-row th-fin-row-head">
        <header className="th-fin-head">
          <p className="th-fin-when">
            {spanLabel(view.row.tsUnixMs, view.row.endUnixMs, { full: true, locale })} <DeviceChip device={view.row.device} />
          </p>
          <h2 className="th-fin-title" tabIndex={-1} ref={titleRef}>
            {title}
          </h2>
          {view.attendees.length > 0 ? <p className="th-fin-attendees">{view.attendees.join(", ")}</p> : null}
          {topic ? <TopicChip topic={topic} /> : null}
          {view.conferenceLink ? (
            <p className="th-fin-attendees">
              <a href={view.conferenceLink}>{view.conferenceLink}</a>
            </p>
          ) : null}
        </header>
        {aiWrote ? (
          <aside className="th-fin-intro" aria-label={t("About the numbers")}>
            <p>{t("Whimbrel wrote the marked lines. Each number points to the moment in the transcript it came from.")}</p>
            {offerSources(model, canRegenerate, hasLines) ? <p>{t("Regenerate the summary to add sources.")}</p> : null}
          </aside>
        ) : null}
      </div>

      {notices}

      {(data.notes ?? "").trim() !== "" || !readOnly ? (
        <div className="th-fin-row">
          <section className="th-fin-section" aria-label={t("Your notes")}>
            <WNoteBlock kind="heading" author="you" text={t("Your notes")} />
            <textarea
              className="th-fin-notes"
              aria-label={t("Your notes")}
              rows={Math.min(12, Math.max(3, (data.notes ?? "").split("\n").length))}
              value={data.notes ?? ""}
              readOnly={readOnly}
              onChange={(e) => data.setNotes(e.target.value)}
            />
          </section>
        </div>
      ) : null}

      {/* One summary section in every state, so typing the first words of a summary never swaps it for another tree and drops focus. */}
      <div className="th-fin-row">
        <section className="th-fin-section">
          <WNoteBlock kind="heading" author="whimbrel" text={t("Summary")} />
          {editing ? (
            <textarea className="th-fin-edit" aria-label={t("Summary")} rows={6} value={summaryText} onChange={(e) => data.setSummary(e.target.value)} />
          ) : state === "processing" || state === "noSummary" || state === "failed" ? (
            <>
              <p className="th-fin-quiet">
                {state === "processing" ? t("The summary appears here when it is ready.") : view.hasTranscript ? t("There is no summary yet.") : t("This meeting hasn't been transcribed.")}
              </p>
              {state !== "processing" && canRegenerate ? (
                <WButton disabled={data.busy} onClick={() => data.askThenRegenerate()}>
                  {t("Regenerate summary")}
                </WButton>
              ) : null}
            </>
          ) : summaryText.trim() ? (
            <WNoteBlock kind="paragraph" author={summaryMine ? "you" : "whimbrel"} text={summaryText} />
          ) : null}
        </section>
      </div>

      {model.decisions.length > 0 ? (
        <>
          <div className="th-fin-row">
            <WNoteBlock kind="heading" author="whimbrel" text={t("Decisions")} announceAi={model.decisions.some((l) => l.ai)} />
          </div>
          {model.decisions.map((l) => lineBlock(l))}
        </>
      ) : null}
      {model.questions.length > 0 ? (
        <>
          <div className="th-fin-row">
            <WNoteBlock kind="heading" author="whimbrel" text={t("Open questions")} announceAi={model.questions.some((l) => l.ai)} />
          </div>
          {model.questions.map((l) => lineBlock(l))}
        </>
      ) : null}

      {items.length > 0 || editing ? (
        <>
          <div className="th-fin-row">
            <WNoteBlock kind="heading" author="whimbrel" text={t("Action items")} announceAi={allActionsAi && !editing} />
          </div>
          {editing ? (
            <div className="th-fin-row">
              <div className="th-fin-edit-items">
                <ActionItems meetingId={meetingId} items={data.items} onItems={data.setItems} readOnly={readOnly} hideHeading />
              </div>
            </div>
          ) : (
            // One list for every row: each row is a list item in its own grid row, so its source card still sits beside it.
            <ul className="th-fin-actions">
              {model.actions.map((a) =>
                row(
                  `item-${a.item.id}`,
                  <WActionItemRow
                    as="div"
                    text={a.item.text}
                    owner={a.owner ?? undefined}
                    due={a.due ? t("Due {date}", { date: a.due }) : undefined}
                    done={a.item.completed}
                    disabled={readOnly}
                    sources={a.ai ? (a.number != null ? [a.number] : []) : undefined}
                    announceAi={!allActionsAi}
                    selectedSource={selected}
                    onSelectSource={select}
                    sourceLabels={labels}
                    onToggle={() => void meetingUpdateActionItem(meetingId, a.item.id, null, !a.item.completed).then(data.setItems, (e: unknown) => data.setError(message(e)))}
                  />,
                  a.number,
                  "li",
                ),
              )}
            </ul>
          )}
        </>
      ) : null}

      {data.marks.length > 0 ? (
        <div className="th-fin-row">
          <section className="th-fin-marks" aria-label={t("Moments you marked")}>
            <WNoteBlock kind="heading" author="you" text={t("Moments you marked")} />
            <ul>
              {data.marks.map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    className="th-fin-mark-open"
                    data-mark-id={m.id}
                    aria-label={t("Open the transcript at {time}", { time: offsetClock(m.atCs) })}
                    onClick={() => showTranscript({ kind: "time", cs: m.atCs }, true)}
                  >
                    {offsetClock(m.atCs)}
                  </button>
                  {editing ? (
                    <input
                      className="th-fin-mark-input"
                      aria-label={t("Mark label")}
                      defaultValue={m.label ?? ""}
                      placeholder={t("Marked moment")}
                      onBlur={(e) => e.target.value.trim() !== (m.label ?? "") && data.renameMark(m.id, e.target.value.trim() || null)}
                    />
                  ) : (
                    <span className="th-fin-mark-label">{m.label ?? t("Marked moment")}</span>
                  )}
                  {!readOnly ? (
                    <button type="button" className="w-icon-button" aria-label={t("Remove this mark")} onClick={() => data.removeMark(m.id)}>
                      <span aria-hidden="true">×</span>
                    </button>
                  ) : null}
                </li>
              ))}
            </ul>
          </section>
        </div>
      ) : null}

      <div className="th-fin-row th-fin-wide">
        <div className="th-fin-stack">
          <SlidesRefreshed meetingId={meetingId} />
          <LiveAlertsSection meetingId={meetingId} recording={false} endedUnixMs={view.row.endUnixMs ?? null} onJump={(cs) => showTranscript({ kind: "time", cs }, true)} />
          {summaryText.trim() || hasLines ? (
            <p className="th-fin-quiet">
              {summaryMine || view.decisionsAreEdited || view.openQuestionsAreEdited ? t("Edited by you") : view.summaryFallback ? null : t("Written by Whimbrel")}
            </p>
          ) : null}
          <MeetingAskBar
            key={`after-${meetingId}`}
            meetingId={meetingId}
            live={false}
            disabledReason={disabledReason({ recording: false, liveLines: 0, processing, hasTranscript: view.hasTranscript })}
            onCite={(id) => showTranscript({ kind: "line", id }, true)}
            continuable
          />
        </div>
      </div>
    </div>
  );

  return (
    <section className="th-fin" aria-label={t("{title} workspace", { title: title ?? "" })} data-mode={mode}>
      <div className="th-fin-bar">
        {onBack ? (
          <WButton variant="quiet" onClick={onBack}>
            {t("← Back")}
          </WButton>
        ) : null}
        <div className="th-fin-bar-end">
          {state === "ready" || summaryText.trim() || hasLines ? (
            <>
              <div className="th-fin-split">
                <WButton icon="copy" onClick={() => copy("plain")}>
                  {copied ? t("Copied") : t("Copy")}
                </WButton>
                <NoteMenu
                  iconOnly
                  label={t("More ways to copy")}
                  items={[
                    { id: "md", label: t("Copy as Markdown"), onPress: () => copy("markdown") },
                    { id: "slack", label: t("Copy as Slack"), onPress: () => copy("slack") },
                  ]}
                />
              </div>
              {!readOnly ? (
                <WButton icon="mail" onClick={openSheet}>
                  {t("Send recap")}
                </WButton>
              ) : null}
            </>
          ) : null}
          {mode === "note" && !readOnly ? (
            // Edit is a mode, Done leaves it: a filled button only while editing, so it reads apart from the view switch beside it.
            <WButton variant={edit ? "primary" : "secondary"} icon={edit ? "check" : undefined} aria-pressed={edit} onClick={() => setEdit(!edit)}>
              {edit ? t("Done") : t("Edit")}
            </WButton>
          ) : null}
          <div className="th-fin-switch" role="group" aria-label={t("View")}>
            <WButton aria-pressed={mode === "note"} onClick={() => showNote(false)}>
              {t("Note")}
            </WButton>
            <WButton icon="chat" hint="Ctrl+T" aria-pressed={mode === "transcript"} onClick={() => showTranscript(null, false)}>
              {t("Transcript")}
            </WButton>
          </div>
          <NoteMenu label={t("More")} items={moreItems} />
        </div>
      </div>
      {shareError ? <p role="alert">{shareError}</p> : null}

      {mode === "note" ? (
        noteBody
      ) : (
        <>
          {notices}
          <NoteTranscript
            meetingId={meetingId}
            hasTranscript={view.hasTranscript}
            marks={data.marks}
            open={open}
            reloadKey={data.transcriptRound}
            onNamesChanged={() => {
              data.setNamesChanged(true);
              data.refreshSources();
            }}
          />
        </>
      )}

      {sheet ? (
        <RecapSheet
          view={view}
          draft={sheet}
          onClose={() => setSheet(null)}
          onSent={(s) => {
            setSent(s);
            setSheet(null);
          }}
        />
      ) : null}

      {trimming && data.trim ? (
        <TrimSheet
          meetingId={meetingId}
          lengthMs={data.trim.lengthMs}
          onCancel={() => setTrimming(false)}
          onTrimmed={() => {
            setTrimming(false);
            // The lines changed: the transcript view reads them again, and the summary is redone from what is left, as Regenerate does.
            data.bumpTranscript();
            data.askThenRegenerate();
          }}
        />
      ) : null}

      {data.conflict ? (
        <ConfirmCard label={t("New summary")}>
          <p>{t("You've edited this meeting. What should happen to your edits?")}</p>
          <div className="toolbar">
            <button className="mode" onClick={() => data.regenerate(true)}>
              {t("Keep my edits")}
            </button>
            <button className="mode mode--active" onClick={() => data.regenerate(false)}>
              {t("Replace with the new summary")}
            </button>
            <button className="mode" onClick={() => data.setConflict(false)}>
              {t("Cancel")}
            </button>
          </div>
        </ConfirmCard>
      ) : null}

      {forgetting ? (
        <ConfirmCard label={t("Forget meeting")}>
          <p>
            <strong>{t("Forget this meeting?")}</strong>
          </p>
          <p>{t("This removes it from all your devices.")}</p>
          <div className="toolbar">
            <button className="mode mode--active" disabled={data.busy} onClick={data.forget}>
              {t("Forget it")}
            </button>
            <button className="mode" onClick={() => setForgetting(false)}>
              {t("Keep it")}
            </button>
          </div>
        </ConfirmCard>
      ) : null}

      {data.error ? <p role="alert">{data.error}</p> : null}
    </section>
  );
}
