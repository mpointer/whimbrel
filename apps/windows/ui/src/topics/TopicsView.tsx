import { useCallback, useEffect, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { topicPage, topicRoleSet, topicsPageList, topicStepAction, topicSuggestions } from "../api";
import { describeError, isCommandError } from "../errors";
import type { LibraryTarget, StepActionInput, TopicListItemView, TopicPageView, TopicStepView } from "../generated/types";
import { useL10n } from "../l10n";
import { Detail, type Selection } from "../library/Detail";
import {
  attendeeLine,
  chooseTopic,
  dayTime,
  dueText,
  groupLabel,
  groupOf,
  listHint,
  monthDay,
  RECENT_FIRST,
  summaryLine,
  TOPIC_GROUPS,
  TOPICS_REFRESH_MS,
} from "./topicPageModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

const SUMMARY_COLLAPSED = "whimbrel.topicSummaryCollapsed";

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(SUMMARY_COLLAPSED) === "1";
  } catch {
    return false;
  }
}

/**
 * The Topics tab (topics piece 2, the Mac's `TopicsView`): named topics grouped by kind, and the
 * chosen topic's page. Reloads on open, when the topic changes and every 30 s while shown.
 */
export function TopicsView({ request }: { request?: { seq: number; topicId: number } | null }) {
  const { t, locale } = useL10n();
  const [list, setList] = useState<TopicListItemView[] | null>(null);
  const [selected, setSelected] = useState<number | null>(request?.topicId ?? null);
  const [page, setPage] = useState<TopicPageView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<Selection | null>(null);
  const wanted = useRef<number | null>(selected);
  // Suggestions waiting in Settings → Topics, for the empty state; asked once there are no named topics.
  const [suggested, setSuggested] = useState(0);
  const empty = list !== null && list.length === 0;
  useEffect(() => {
    if (!empty) return;
    topicSuggestions().then((s) => setSuggested(Array.isArray(s) ? s.length : 0), () => {});
  }, [empty]);

  // A way in (Today, a meeting, search, the tray) asked for a topic.
  useEffect(() => {
    if (!request) return;
    wanted.current = request.topicId;
    setSelected(request.topicId);
    setOpen(null);
  }, [request]);

  const reload = useCallback(() => {
    topicsPageList()
      .then((l) => {
        setList(l);
        const id = chooseTopic(l, wanted.current);
        wanted.current = id;
        setSelected(id);
        if (id == null) {
          setPage(null);
          return;
        }
        return topicPage(id).then((p) => {
          // A reply for a topic no longer chosen is dropped; the next reload covers it.
          if (wanted.current === id) setPage(p);
        });
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  useEffect(() => {
    reload();
    const timer = setInterval(reload, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [reload, selected]);

  const choose = (id: number) => {
    wanted.current = id;
    setSelected(id);
    setPage(null);
    setError(null);
  };

  if (open) return <Detail selection={open} onBack={() => setOpen(null)} />;

  // No named topics yet: the Mac's empty state, centred on its own, not pushed into the page column beside
  // an empty list. It counts the suggestions waiting in Settings → Topics.
  if (list && list.length === 0) {
    return (
      <section className="card empty-state topics-empty">
        {error && <p role="alert">{error}</p>}
        <h2 className="section-label">{t("No topics yet")}</h2>
        <p className="muted">
          {suggested > 0
            ? t("Suggested topics waiting: {n}. Add the ones you care about in Settings.", { n: String(suggested) })
            : t("Name the customers, products and projects you care about in Settings.")}
        </p>
        <button type="button" className="mode mode--active" onClick={() => void emit("open-settings", "topics")}>
          {t("Open Topics settings")}
        </button>
      </section>
    );
  }

  return (
    <div className="topics">
      <nav className="topics-list" aria-label={t("Topics")}>
        {TOPIC_GROUPS.map((group) => {
          const entries = (list ?? []).filter((e) => groupOf(e.kind) === group);
          if (entries.length === 0) return null;
          return (
            <section key={group}>
              <h2 className="section-label">{groupLabel(group)}</h2>
              <ul>
                {entries.map((e) => {
                  const hint = listHint(e, locale);
                  return (
                    <li key={e.topic.topicId}>
                      <button
                        className={e.topic.topicId === selected ? "topic-row topic-row--active" : "topic-row"}
                        aria-current={e.topic.topicId === selected ? "true" : undefined}
                        onClick={() => choose(e.topic.topicId)}
                      >
                        <span className="row-title">{e.topic.name}</span>
                        {hint && <span className="muted small">{hint}</span>}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
      </nav>
      <section className="topics-page">
        {error && <p role="alert">{error}</p>}
        {page ? (
          <TopicPage
            key={page.topic.topicId}
            page={page}
            onOpen={setOpen}
            act={(work) => {
              setError(null);
              work()
                .then(reload)
                .catch((e: unknown) => setError(message(e)));
            }}
          />
        ) : (
          list && (
            <p className="muted">
              {list.length === 0
                ? t("No topics yet. Name the customers, products and projects you care about in Settings > Topics.")
                : t("Choose a topic.")}
            </p>
          )
        )}
      </section>
    </div>
  );
}

type Act = (work: () => Promise<unknown>) => void;

function TopicPage({ page, onOpen, act }: { page: TopicPageView; onOpen: (s: Selection) => void; act: Act }) {
  const { t, locale } = useL10n();
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [role, setRole] = useState(page.role);
  const [allRecent, setAllRecent] = useState(false);
  const editingRole = useRef(false);

  // A reload brings the role back; it replaces the field unless the user is typing in it.
  useEffect(() => {
    if (!editingRole.current) setRole(page.role);
  }, [page.role]);

  const toggleSummary = () => {
    const next = !collapsed;
    setCollapsed(next);
    try {
      localStorage.setItem(SUMMARY_COLLAPSED, next ? "1" : "0");
    } catch {
      // The choice just won't be remembered.
    }
  };

  const line = summaryLine(page, locale);
  const recent = allRecent ? page.recent : page.recent.slice(0, RECENT_FIRST);
  const hidden = page.recentTotal - page.recent.length;

  return (
    <article className="topic-page">
      <h2 className="display">{page.topic.name}</h2>
      {line && (
        <div className="field">
          <button className="link-button small muted" aria-expanded={!collapsed} onClick={toggleSummary}>
            {t("Summary")}
          </button>
          {!collapsed && <p>{line}</p>}
        </div>
      )}

      <Section title={t("Your role")}>
        <input
          className="input"
          value={role}
          placeholder={t("Your role on this topic")}
          aria-label={t("Your role")}
          title={page.roleEdited ? t("Your wording. Enter saves; empty it to let Whimbrel suggest one.") : t("Whimbrel's guess. Edit it and press Enter.")}
          onFocus={() => (editingRole.current = true)}
          onBlur={() => (editingRole.current = false)}
          onChange={(e) => setRole(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            editingRole.current = false;
            act(() => topicRoleSet(page.topic.key, role));
          }}
        />
      </Section>

      {page.nextMeeting && (
        <Section title={t("Next meeting")}>
          <p>
            {dayTime(page.nextMeeting.startUnixMs, locale)} {page.nextMeeting.title}
          </p>
          {attendeeLine(page.nextMeeting) && <p className="muted small">{attendeeLine(page.nextMeeting)}</p>}
        </Section>
      )}

      <Section title={t("You owe")}>
        <Steps steps={page.youOwe} empty={t("Nothing you owe right now.")} onOpen={onOpen} act={act} />
      </Section>
      <Section title={t("Owed to you")}>
        <Steps steps={page.owedToYou} empty={t("Nothing owed to you right now.")} onOpen={onOpen} act={act} />
      </Section>
      {page.done.length > 0 && (
        <details className="field">
          <summary className="section-label">{t("Done")}</summary>
          <Steps steps={page.done} empty="" onOpen={onOpen} act={act} />
        </details>
      )}
      {page.older.length > 0 && (
        <details className="field">
          <summary className="section-label">{t("Older")}</summary>
          <Steps steps={page.older} empty="" onOpen={onOpen} act={act} />
        </details>
      )}

      <Section title={t("Recent")}>
        {page.recent.length === 0 ? (
          <p className="muted small">{t("Nothing yet.")}</p>
        ) : (
          <>
            <ul className="topic-recent">
              {recent.map((item, i) => (
                <li key={i}>
                  <span className="muted small">{monthDay(new Date(item.tsUnixMs), locale)}</span> <SourceLink item={item} onOpen={onOpen} />
                </li>
              ))}
            </ul>
            {!allRecent && page.recent.length > RECENT_FIRST ? (
              <button className="mode" onClick={() => setAllRecent(true)}>
                {t("More")}
              </button>
            ) : (
              hidden > 0 && <p className="muted small">{t("{n} older items not shown", { n: String(hidden) })}</p>
            )}
          </>
        )}
      </Section>
    </article>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="field">
      <h3 className="section-label">{title}</h3>
      {children}
    </section>
  );
}

function kindLabel(kind: string): string {
  return kind ? kind[0].toUpperCase() + kind.slice(1) : kind;
}

/** The source's title, as a link when it can be opened here (an email thread can't, on Windows). */
function SourceLink({ item, onOpen }: { item: { kind: string; title: string; open?: LibraryTarget | null }; onOpen: (s: Selection) => void }) {
  const label = item.title || kindLabel(item.kind);
  const target = item.open;
  return target ? (
    <button className="link-button" onClick={() => onOpen(target)}>
      {label}
    </button>
  ) : (
    <span>{label}</span>
  );
}

type Editing = { kind: "note" } | { kind: "edit" } | null;

function Steps({ steps, empty, onOpen, act }: { steps: TopicStepView[]; empty: string; onOpen: (s: Selection) => void; act: Act }) {
  if (steps.length === 0) return empty ? <p className="muted small">{empty}</p> : null;
  return (
    <ul className="topic-steps">
      {steps.map((s) => (
        <StepRow key={s.key} step={s} onOpen={onOpen} act={act} />
      ))}
    </ul>
  );
}

function StepRow({ step, onOpen, act }: { step: TopicStepView; onOpen: (s: Selection) => void; act: Act }) {
  const { t, locale } = useL10n();
  const [editing, setEditing] = useState<Editing>(null);
  const [note, setNote] = useState(step.note ?? "");
  const [text, setText] = useState(step.text);
  const [due, setDue] = useState(step.dueDate ?? "");
  const [dismissing, setDismissing] = useState(false);
  const run = (action: StepActionInput) => act(() => topicStepAction(step.key, action));
  const done = step.status === "done";
  const due_ = step.dueDate ? dueText(step.dueDate, locale) : null;

  return (
    <li className="topic-step">
      <p className={done ? "topic-step-text topic-step-text--done" : "topic-step-text"}>
        {step.confirmed && (
          <span className="topic-step-confirmed" title={t("Confirmed")} aria-label={t("Confirmed")}>
            ✓{" "}
          </span>
        )}
        {step.text}
      </p>
      <p className="muted small">
        <SourceLink item={{ kind: step.sourceKind, title: step.sourceTitle, open: step.open }} onOpen={onOpen} />
        {step.fromScreen && <> · {t("from screen")}</>}
        {" · "}
        {t("asked {day}", { day: monthDay(new Date(step.askedAtUnixMs), locale) })}
        {due_ && <> · <span className={due_ === t("overdue") ? "topic-overdue" : undefined}>{due_}</span></>}
      </p>
      {step.note && <p className="muted small">{t("Note: {note}", { note: step.note })}</p>}
      {done && step.closedBy && <p className="muted small">{t("Closed by {source}", { source: step.closedBy })}</p>}

      {editing?.kind === "note" ? (
        <form
          className="field-row"
          onSubmit={(e) => {
            e.preventDefault();
            setEditing(null);
            run({ action: "note", text: note });
          }}
        >
          <input className="input grow" value={note} aria-label={t("Note")} autoFocus onChange={(e) => setNote(e.target.value)} />
          <button type="submit" className="mode mode--active">
            {t("Save")}
          </button>
          <button type="button" className="mode" onClick={() => setEditing(null)}>
            {t("Cancel")}
          </button>
        </form>
      ) : editing?.kind === "edit" ? (
        <form
          className="field-row"
          onSubmit={(e) => {
            e.preventDefault();
            if (!text.trim()) return;
            setEditing(null);
            run({ action: "edit", text, dueDate: due || null });
          }}
        >
          <input className="input grow" value={text} aria-label={t("Step")} autoFocus onChange={(e) => setText(e.target.value)} />
          <input className="input" type="date" value={due} aria-label={t("Due date")} onChange={(e) => setDue(e.target.value)} />
          <button type="submit" className="mode mode--active" disabled={!text.trim()}>
            {t("Save")}
          </button>
          <button type="button" className="mode" onClick={() => setEditing(null)}>
            {t("Cancel")}
          </button>
        </form>
      ) : dismissing ? (
        <div className="toolbar">
          <span className="small">{t("Dismiss")}:</span>
          <button className="mode" onClick={() => run({ action: "dismiss", reason: "not_mine" })}>
            {t("Not mine")}
          </button>
          <button className="mode" onClick={() => run({ action: "dismiss", reason: "not_task" })}>
            {t("Not a task")}
          </button>
          <button className="mode" onClick={() => setDismissing(false)}>
            {t("Cancel")}
          </button>
        </div>
      ) : (
        <div className="toolbar">
          {done ? (
            <button className="mode" onClick={() => run({ action: "reopen" })}>
              {t("Reopen")}
            </button>
          ) : (
            <>
              <button className="mode" onClick={() => run({ action: "done" })}>
                {t("Check off")}
              </button>
              <button className="mode" onClick={() => setEditing({ kind: "note" })}>
                {t("Note")}
              </button>
              <button className="mode" onClick={() => setDismissing(true)}>
                {t("Dismiss")}
              </button>
              {!step.confirmed && (
                <button className="mode" onClick={() => run({ action: "confirm", confirmed: true })}>
                  {t("Confirm")}
                </button>
              )}
              <button className="mode" onClick={() => setEditing({ kind: "edit" })}>
                {t("Edit")}
              </button>
            </>
          )}
        </div>
      )}
    </li>
  );
}
