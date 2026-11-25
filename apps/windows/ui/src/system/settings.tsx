// Parts for a Settings screen: a section list, a card, and rows that share one layout (label, description, control;
// the control drops under the label at a narrow width or large text). Plain props, no app imports. The rules are in
// docs/ui-redesign/design-system-settings-windows.md.

import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { t } from "../l10n";
import { WButton, WIconButton } from "./controls";
import { WSegmented } from "./flow";
import { breakAtSeparators, checkAddDraft, commitNumberDraft, navFocusIndex, secretToSave, secretView, type NumberLimits, type NumberProblem } from "./settingsLogic";
import { WIcon, type WIconName } from "./WIcon";
import { WInlineConfirm } from "./confirm";
import "./system.css";
import "./settings.css";

/* ---- Status chip and notice box ---- */

export type WStatusTone = "neutral" | "good" | "warning" | "problem";
const CHIP_ICON: Record<WStatusTone, WIconName> = { neutral: "info", good: "check", warning: "warning", problem: "error" };

export interface WStatusChipProps {
  tone?: WStatusTone;
  /** The state in words. The glyph and the border repeat it; the words carry it. */
  children: ReactNode;
}

/** A short state ("Encrypted", "Not running"): a glyph that differs by tone, the words, and a border. Never color alone. */
export function WStatusChip({ tone = "neutral", children }: WStatusChipProps) {
  return (
    <span className={`w-status w-status-${tone}`}>
      <WIcon name={CHIP_ICON[tone]} size={14} />
      <span>{children}</span>
    </span>
  );
}

const BOX_ICON: Record<"info" | "ok" | "warning" | "error", WIconName> = { info: "info", ok: "check", warning: "warning", error: "error" };

export interface WNoticeBoxProps {
  tone?: "info" | "ok" | "warning" | "error";
  title?: string;
  children: ReactNode;
  /** Buttons under the words, the main one last. */
  actions?: ReactNode;
  /** Read out when it appears. An error always is. Leave off for a note that is simply part of the page. */
  live?: boolean;
}

/** A bordered note with a glyph, words, and optional buttons. Unlike WNotice it is not announced unless asked, so a
 *  note that is part of the page does not interrupt a screen reader. */
export function WNoticeBox({ tone = "info", title, children, actions, live }: WNoticeBoxProps) {
  const role = tone === "error" ? "alert" : live ? "status" : undefined;
  return (
    <div className={`w-nbox w-nbox-${tone}`} role={role}>
      <WIcon name={BOX_ICON[tone]} />
      <div className="w-nbox-body">
        {title ? <p className="w-nbox-title">{title}</p> : null}
        <div className="w-nbox-text">{children}</div>
        {actions ? <div className="w-nbox-actions">{actions}</div> : null}
      </div>
    </div>
  );
}

/** A muted line under a control: a limit, a reason, "Available in release builds only." */
export function WSettingsNote({ children, id }: { children: ReactNode; id?: string }) {
  return (
    <p className="w-setting-note" id={id}>
      {children}
    </p>
  );
}

/* ---- Section list ---- */

export interface WSettingsNavItem {
  id: string;
  label: string;
  icon?: WIconName;
  /** A count or word on the item, such as pending approvals. Needs `badgeLabel`, so it has a name. */
  badge?: number | string;
  badgeLabel?: string;
}

export interface WSettingsNavProps {
  /** What the list is, for a screen reader: "Settings sections". */
  label: string;
  items: readonly WSettingsNavItem[];
  selected: string;
  onSelect: (id: string) => void;
  /** The name of the select shown in the compact layout. Defaults to `label`. */
  selectLabel?: string;
  /** True: always the select. False: always the list. Left out: the list, switching to the select when the
   *  WSettingsLayout around it is narrower than 40rem. */
  compact?: boolean;
}

/** A vertical list of sections with ONE Tab stop. Up, Down, Home and End move focus; Return or Space opens the
 *  section. The open one is marked `aria-current="page"`, and drawn with a fill and a leading bar. */
export function WSettingsNav({ label, items, selected, onSelect, selectLabel, compact }: WSettingsNavProps) {
  const [focused, setFocused] = useState<string | null>(null);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const stop = focused !== null && items.some((i) => i.id === focused) ? focused : selected;
  const cls = compact === true ? "w-snav w-snav-compact" : compact === false ? "w-snav w-snav-list" : "w-snav";
  return (
    <nav
      className={cls}
      aria-label={label}
      onBlur={(e) => {
        // Focus left the list: the next Tab in comes back to the open section.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(null);
      }}
    >
      <ul className="w-snav-items" role="list">
        {items.map((item, i) => (
          <li key={item.id}>
            <button
              type="button"
              ref={(el) => {
                refs.current[i] = el;
              }}
              id={`${item.id}-nav`}
              className="w-snav-item"
              tabIndex={item.id === stop ? 0 : -1}
              aria-current={item.id === selected ? "page" : undefined}
              onFocus={() => setFocused(item.id)}
              onClick={() => onSelect(item.id)}
              onKeyDown={(e: KeyboardEvent<HTMLButtonElement>) => {
                const next = navFocusIndex(e.key, i, items.length);
                if (next === null) return;
                e.preventDefault();
                setFocused(items[next].id);
                refs.current[next]?.focus();
              }}
            >
              {item.icon ? <WIcon name={item.icon} /> : null}
              <span className="w-snav-label">{item.label}</span>
              {item.badge !== undefined ? (
                <span className="w-badge" role="img" aria-label={item.badgeLabel ?? String(item.badge)}>
                  {item.badge}
                </span>
              ) : null}
            </button>
          </li>
        ))}
      </ul>
      <select className="w-input w-select w-snav-select" aria-label={selectLabel ?? label} value={selected} onChange={(e) => onSelect(e.target.value)}>
        {items.map((item) => (
          <option key={item.id} value={item.id}>
            {item.badge !== undefined ? `${item.label} (${item.badgeLabel ?? item.badge})` : item.label}
          </option>
        ))}
      </select>
    </nav>
  );
}

/** The two columns of a Settings screen: the section list, and the content. It is the container the list measures,
 *  so when it is narrower than 40rem (a small window, or large text) the list becomes a select above the content and
 *  the content keeps the full width. */
export function WSettingsLayout({ nav, children }: { nav: ReactNode; children: ReactNode }) {
  return (
    <div className="w-slayout">
      <div className="w-slayout-grid">
        <div className="w-slayout-nav">{nav}</div>
        <div className="w-slayout-body">{children}</div>
      </div>
    </div>
  );
}

export interface WSettingsSearchResult {
  id: string;
  title: string;
  /** The section it lives in, as a caption: "Capture and privacy". */
  section: string;
}

export interface WSettingsSearchResultsProps {
  results: readonly WSettingsSearchResult[];
  onOpen: (id: string) => void;
  /** Read out when the list changes, such as "3 settings found". */
  summary?: string;
  label?: string;
  emptyLabel?: string;
}

/** What a settings search finds: each row names the setting and the section it is in. */
export function WSettingsSearchResults({ results, onOpen, summary, label, emptyLabel }: WSettingsSearchResultsProps) {
  return (
    <div className="w-sres">
      <p className="w-visually-hidden" role="status">
        {results.length === 0 ? (emptyLabel ?? t("No matching settings")) : (summary ?? "")}
      </p>
      {results.length === 0 ? (
        <p className="w-sres-empty">{emptyLabel ?? t("No matching settings")}</p>
      ) : (
        <ul role="list" aria-label={label ?? t("Search results")} className="w-sres-list">
          {results.map((r) => (
            <li key={r.id}>
              <button type="button" className="w-sres-item" onClick={() => onOpen(r.id)}>
                <span className="w-sres-title">{r.title}</span>
                <span className="w-sres-section">{r.section}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/* ---- Card ---- */

export interface WSettingsCardProps {
  /** Anchor for "jump to this card". */
  id?: string;
  title: string;
  /** The real heading level. Section titles are level 2, so a card is 2 or 3. */
  headingLevel: 2 | 3 | 4;
  description?: ReactNode;
  status?: WStatusChipProps;
  /** Buttons at the end of the heading line. */
  actions?: ReactNode;
  /** A first read is running: the rows are replaced by a line. */
  loading?: boolean;
  loadingLabel?: string;
  /** The card's own error line, or several (a standing notice and the last action's error). Read out when it appears. */
  error?: string | null | readonly (string | null | undefined)[];
  children?: ReactNode;
}

/** A section container: a real heading, a description, an optional status chip, then rows divided by hairlines. */
export function WSettingsCard({ id, title, headingLevel, description, status, actions, loading, loadingLabel, error, children }: WSettingsCardProps) {
  const hid = useId();
  const H = `h${headingLevel}` as "h2" | "h3" | "h4";
  const errors = (Array.isArray(error) ? error : [error]).filter((e): e is string => !!e);
  return (
    <section id={id} className="w-scard" aria-labelledby={hid} aria-busy={loading ? true : undefined}>
      <header className="w-scard-head">
        <H className="w-scard-title" id={hid}>
          {title}
        </H>
        {status ? <WStatusChip {...status} /> : null}
        {actions ? <div className="w-scard-actions">{actions}</div> : null}
      </header>
      {description ? <div className="w-scard-desc">{description}</div> : null}
      <div className="w-scard-alert" role="alert">
        {errors.map((e) => (
          <p className="w-field-error" key={e}>
            <WIcon name="warning" size={16} />
            <span>{e}</span>
          </p>
        ))}
      </div>
      {loading ? (
        <p className="w-scard-loading" role="status">
          {loadingLabel ?? t("Loading…")}
        </p>
      ) : (
        <div className="w-scard-rows">{children}</div>
      )}
    </section>
  );
}

/* ---- Row frame ---- */

export interface WSettingsRowCommon {
  /** Element id, so a search result or a link can jump here. */
  id?: string;
  label: string;
  description?: string;
  disabled?: boolean;
  /** Said in words under the description while the row is disabled. A disabled row with no reason is a dead end. */
  disabledReason?: string;
  /** "You jumped here from search": a fill and a leading bar, and focus moves to the row. */
  highlighted?: boolean;
  /** What is wrong with the value, under the control. */
  error?: string | null;
}

interface FrameProps {
  common: WSettingsRowCommon;
  /** The label element, built by the row so it can be a `label` for its control. */
  labelNode: ReactNode;
  descId?: string;
  reasonId?: string;
  errorId?: string;
  control?: ReactNode;
  /** Below the label, full width (a field, a list). */
  stacked?: boolean;
  /** Under the control, full width. */
  below?: ReactNode;
  className?: string;
  /** Make the control area a named group that is described by the row's description, reason and error. */
  group?: { labelId: string; describedBy?: string };
}

function RowFrame({ common, labelNode, descId, reasonId, errorId, control, stacked, below, className, group }: FrameProps) {
  const ref = useRef<HTMLDivElement>(null);
  const { highlighted, description, disabled, disabledReason, error } = common;
  useEffect(() => {
    if (!highlighted) return;
    const el = ref.current;
    el?.focus?.({ preventScroll: true });
    el?.scrollIntoView?.({ block: "nearest" });
  }, [highlighted]);
  const cls = ["w-setting", "w-sr", stacked ? "w-setting-stack" : "", highlighted ? "w-setting-jumped" : "", disabled ? "w-setting-off" : "", className ?? ""].filter(Boolean).join(" ");
  return (
    <div className={cls} id={common.id} ref={ref} tabIndex={highlighted ? -1 : undefined}>
      <div className="w-setting-text">
        {labelNode}
        {description ? (
          <span className="w-setting-desc" id={descId}>
            {description}
          </span>
        ) : null}
        {disabled && disabledReason ? (
          <span className="w-setting-reason" id={reasonId}>
            <WIcon name="info" size={14} />
            <span>{disabledReason}</span>
          </span>
        ) : null}
      </div>
      {control ? (
        <div className="w-setting-control" role={group ? "group" : undefined} aria-labelledby={group?.labelId} aria-describedby={group?.describedBy}>
          {control}
        </div>
      ) : null}
      {below}
      <div className="w-setting-error" id={errorId} aria-live="polite">
        {error ? (
          <p className="w-field-error">
            <WIcon name="warning" size={16} />
            <span>{error}</span>
          </p>
        ) : null}
      </div>
    </div>
  );
}

/** The ids a row's control points at: its description, the reason it is off, and its error. */
function useRowIds(common: WSettingsRowCommon) {
  const base = useId();
  const ids = {
    control: `${base}-c`,
    label: `${base}-l`,
    desc: common.description ? `${base}-d` : undefined,
    reason: common.disabled && common.disabledReason ? `${base}-r` : undefined,
    error: `${base}-e`,
    extra: `${base}-x`,
  };
  const described = [ids.desc, ids.reason, common.error ? ids.error : undefined].filter(Boolean).join(" ") || undefined;
  return { ids, described };
}

/* ---- Rows ---- */

export interface WSettingsToggleRowProps extends WSettingsRowCommon {
  checked: boolean;
  onToggle: (next: boolean) => void;
  /** "switch" (default) is a switch. "checkbox" is a checkbox before the label, with the description as its muted note. */
  kind?: "switch" | "checkbox";
}

export function WSettingsToggleRow({ kind = "switch", checked, onToggle, ...common }: WSettingsToggleRowProps) {
  const { ids, described } = useRowIds(common);
  if (kind === "checkbox") {
    return (
      <RowFrame
        common={{ ...common, description: undefined }}
        descId={undefined}
        reasonId={ids.reason}
        errorId={ids.error}
        className="w-setting-checkrow"
        labelNode={
          <label className="w-setting-checklabel">
            <input type="checkbox" checked={checked} disabled={common.disabled} aria-describedby={described} onChange={(e) => onToggle(e.target.checked)} />
            <span className="w-setting-text-inner">
              <span className="w-setting-label">{common.label}</span>
              {common.description ? (
                <span className="w-setting-desc" id={ids.desc}>
                  {common.description}
                </span>
              ) : null}
            </span>
          </label>
        }
      />
    );
  }
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <span className="w-switch-wrap">
          <button
            type="button"
            role="switch"
            className="w-switch"
            aria-checked={checked}
            aria-labelledby={ids.label}
            aria-describedby={described}
            disabled={common.disabled}
            onClick={() => onToggle(!checked)}
          />
        </span>
      }
    />
  );
}

export interface WSettingsSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface WSettingsSelectRowProps extends WSettingsRowCommon {
  value: string;
  options: readonly WSettingsSelectOption[];
  onChange: (value: string) => void;
  /** Shown beside the select, such as a Refresh button. */
  trailing?: ReactNode;
}

/** A native select with its label tied to it. For two to four choices use WSettingsChoiceRow. */
export function WSettingsSelectRow({ value, options, onChange, trailing, ...common }: WSettingsSelectRowProps) {
  const { ids, described } = useRowIds(common);
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <label className="w-setting-label" htmlFor={ids.control}>
          {common.label}
        </label>
      }
      control={
        <span className="w-setting-inline">
          <select
            id={ids.control}
            className="w-input w-select"
            value={value}
            disabled={common.disabled}
            aria-describedby={described}
            aria-invalid={common.error ? true : undefined}
            onChange={(e) => onChange(e.target.value)}
          >
            {options.map((o) => (
              <option key={o.value} value={o.value} disabled={o.disabled}>
                {o.label}
              </option>
            ))}
          </select>
          {trailing}
        </span>
      }
    />
  );
}

export interface WSettingsChoiceRowProps<V extends string> extends WSettingsRowCommon {
  value: V;
  options: readonly { value: V; label: string }[];
  onChange: (value: V) => void;
  /** Arrows move focus only; Space or Return chooses (see WSegmented). For a choice that turns a safeguard off. */
  manual?: boolean;
}

/** Two to four exclusive choices side by side (WSegmented), in a row. Disabled keeps the choice visible and says why. */
export function WSettingsChoiceRow<V extends string>({ value, options, onChange, manual, ...common }: WSettingsChoiceRowProps<V>) {
  const { ids, described } = useRowIds(common);
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={<WSegmented label={common.label} value={value} options={options} onChange={onChange} disabled={common.disabled} manual={manual} describedBy={described} />}
    />
  );
}

/** The row's label as plain text, or as a real heading of the given level (for a block that is its own part of the page). */
function LabelText({ id, level, children }: { id: string; level?: 4 | 5; children: ReactNode }) {
  if (level === 4) return <h4 className="w-setting-label w-setting-heading" id={id}>{children}</h4>;
  if (level === 5) return <h5 className="w-setting-label w-setting-heading" id={id}>{children}</h5>;
  return <span className="w-setting-label" id={id}>{children}</span>;
}

/** A row with a control the caller supplies: several buttons, a status with an action, anything else. `heading` makes the
 *  label a real heading of that level. */
export function WSettingsControlRow({ children, stacked, heading, ...common }: WSettingsRowCommon & { children: ReactNode; stacked?: boolean; heading?: 4 | 5 }) {
  const { ids, described } = useRowIds(common);
  return (
    <RowFrame
      common={common}
      group={described ? { labelId: ids.label, describedBy: described } : undefined}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      stacked={stacked}
      labelNode={
        <LabelText id={ids.label} level={heading}>
          {common.label}
        </LabelText>
      }
      control={children}
    />
  );
}

export interface WSettingsStatusRowProps extends WSettingsRowCommon {
  status: WStatusChipProps;
  /** A button that acts on this state ("Try again"). */
  action?: ReactNode;
  /** The state changes while the page is open: say each new state to a screen reader. */
  live?: boolean;
}

export function WSettingsStatusRow({ status, action, live, ...common }: WSettingsStatusRowProps) {
  const { ids, described } = useRowIds(common);
  return (
    <RowFrame
      common={common}
      group={described ? { labelId: ids.label, describedBy: described } : undefined}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <span className="w-setting-inline">
          {live ? (
            <span role="status">
              <WStatusChip {...status} />
            </span>
          ) : (
            <WStatusChip {...status} />
          )}
          {action}
        </span>
      }
    />
  );
}

/* ---- Number ---- */

export interface WSettingsNumberRowProps extends WSettingsRowCommon, NumberLimits {
  /** The stored number. It stays in the box even while the companion checkbox has the box switched off. */
  value: number;
  /** Called with a valid, changed number on blur or Return. Never called for an empty or invalid box. */
  onCommit: (next: number) => void;
  /** "days", "minutes": drawn after the box and read with it. */
  unit?: string;
  /** A checkbox beside the box that switches the box off, such as "Keep forever". The number is not touched. */
  companion?: { label: string; checked: boolean; onChange: (next: boolean) => void };
  /** Replaces the default validation sentence. */
  invalidMessage?: string;
  /** Nothing is stored for this number yet: the box only shows a remembered one. Any valid number is then a change
   *  (even the one shown), and "Already set" is never said. */
  nothingStored?: boolean;
}

function problemText(problem: NumberProblem, min?: number, max?: number): string {
  if (problem === "fraction") return t("Enter a whole number.");
  if (min !== undefined && max !== undefined) return t("Enter a number from {min} to {max}.", { min, max });
  if (min !== undefined) return t("Enter a number of at least {min}.", { min });
  if (max !== undefined) return t("Enter a number of at most {max}.", { max });
  return t("Enter a number.");
}

/** What the row says about a stored number outside the allowed range (an old value, or one a newer version wrote). */
function rangeNote(value: number, min?: number, max?: number, unit?: string): string {
  const shown = unit ? `${value} ${unit}` : String(value);
  if (min !== undefined && max !== undefined) return t("The stored value, {value}, is outside {min} to {max}. Enter a number in that range to replace it.", { value: shown, min, max });
  return t("The stored value, {value}, is outside the allowed range. Enter a new number to replace it.", { value: shown });
}

/** A number box that commits on blur or Return, and never writes an empty or invalid value: clearing the box shows a
 *  line saying what is allowed and leaves the stored number alone. Escape puts the stored number back. */
export function WSettingsNumberRow({ value, onCommit, unit, companion, invalidMessage, nothingStored, min, max, integer = true, money, ...common }: WSettingsNumberRowProps) {
  const { ids } = useRowIds(common);
  const [draft, setDraft] = useState<string | null>(null);
  const [problem, setProblem] = useState<NumberProblem | null>(null);
  // The same number committed again writes nothing; the row says so, so the press is not a silent no-op.
  const [same, setSame] = useState<number | null>(null);
  const shown = draft ?? String(value);
  const off = common.disabled || companion?.checked;
  const stale = !off && ((min !== undefined && value < min) || (max !== undefined && value > max));
  const commit = () => {
    if (draft === null) return;
    const result = commitNumberDraft(draft, nothingStored ? null : value, { min, max, integer, money });
    setProblem(result.problem);
    if (result.problem === null) setDraft(null);
    if (result.emit !== null) onCommit(result.emit);
    setSame(result.problem === null && result.emit === null ? value : null);
  };
  const message = problem ? (invalidMessage ?? problemText(problem, min, max)) : (common.error ?? null);
  const errId = `${ids.error}-n`;
  // The box is off while its companion box is ticked; say why, and tie the line to the box.
  const offWhy = companion?.checked && !common.disabled ? `${ids.error}-off` : undefined;
  return (
    <RowFrame
      common={{ ...common, error: message }}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={errId}
      below={
        <div role="status" className="w-setting-numnote">
          {offWhy ? <WSettingsNote id={offWhy}>{t("Not used while {option} is ticked.", { option: companion!.label })}</WSettingsNote> : null}
          {same !== null ? (
            <WSettingsNote>{unit ? t("Already set to {value} {unit}.", { value: same, unit }) : t("Already set to {value}.", { value: same })}</WSettingsNote>
          ) : stale ? (
            <WSettingsNote>{rangeNote(value, min, max, unit)}</WSettingsNote>
          ) : null}
        </div>
      }
      labelNode={
        <label className="w-setting-label" htmlFor={ids.control}>
          {common.label}
        </label>
      }
      control={
        <span className="w-setting-inline">
          <span className="w-numfield">
            <input
              id={ids.control}
              className="w-input w-num"
              type="text"
              inputMode={integer ? "numeric" : "decimal"}
              autoComplete="off"
              value={shown}
              disabled={off}
              aria-describedby={[ids.desc, ids.reason, unit ? ids.extra : undefined, offWhy, message ? errId : undefined].filter(Boolean).join(" ") || undefined}
              aria-invalid={problem ? true : undefined}
              onChange={(e) => {
                setDraft(e.target.value);
                setProblem(null);
                setSame(null);
              }}
              onBlur={commit}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  e.stopPropagation();
                  commit();
                } else if (e.key === "Escape" && draft !== null) {
                  e.preventDefault();
                  e.stopPropagation();
                  setDraft(null);
                  setProblem(null);
                }
              }}
            />
            {unit ? (
              <span className="w-unit" id={ids.extra}>
                {unit}
              </span>
            ) : null}
          </span>
          {companion ? (
            <label className="w-check w-check-inline">
              <input type="checkbox" checked={companion.checked} disabled={common.disabled} onChange={(e) => companion.onChange(e.target.checked)} />
              <span>{companion.label}</span>
            </label>
          ) : null}
        </span>
      }
    />
  );
}

/* ---- Text ---- */

export interface WSettingsTextRowProps extends WSettingsRowCommon {
  value: string;
  /** Called with the new text on blur (and Return in a single line, Ctrl+Return in a text area), only when it changed. Return
   *  a promise (or false) so the row knows the save failed: it then keeps the typed text and tries again on the next blur,
   *  because the text still differs from the stored value. */
  onCommit: (next: string) => void | boolean | Promise<void | boolean>;
  placeholder?: string;
  /** A text area with this many rows. */
  rows?: number;
  mono?: boolean;
  /** An empty box is not stored: the row says so and keeps the stored text. */
  required?: boolean;
  requiredMessage?: string;
  /** Returns what is wrong with the text, or null. A wrong value is shown and not stored. */
  validate?: (text: string) => string | null;
}

export function WSettingsTextRow({ value, onCommit, placeholder, rows, mono, required, requiredMessage, validate, ...common }: WSettingsTextRowProps) {
  const { ids } = useRowIds(common);
  const [draft, setDraft] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const shown = draft ?? value;
  const commit = () => {
    if (draft === null) return;
    if (required && draft.trim() === "") {
      setProblem(requiredMessage ?? t("This can't be empty."));
      return;
    }
    const bad = validate?.(draft) ?? null;
    setProblem(bad);
    if (bad) return;
    if (draft === value) {
      setDraft(null);
      return;
    }
    // The typed text stays until the save is known to have worked; a failed save keeps it, so the next blur saves again.
    const sent = draft;
    let result: ReturnType<typeof onCommit>;
    try {
      result = onCommit(sent);
    } catch {
      return;
    }
    if (result instanceof Promise) {
      result.then(
        (ok) => {
          if (ok !== false) setDraft((now) => (now === sent ? null : now));
        },
        () => {},
      );
    } else if (result !== false) setDraft(null);
  };
  const message = problem ?? common.error ?? null;
  const errId = `${ids.error}-t`;
  const props = {
    id: ids.control,
    className: mono ? "w-input w-input-mono" : "w-input",
    value: shown,
    placeholder,
    disabled: common.disabled,
    autoComplete: "off",
    "aria-describedby": [ids.desc, ids.reason, message ? errId : undefined].filter(Boolean).join(" ") || undefined,
    "aria-invalid": message ? (true as const) : undefined,
    onBlur: commit,
  };
  const onKey = (e: KeyboardEvent<HTMLElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Escape" && draft !== null) {
      e.preventDefault();
      e.stopPropagation();
      setDraft(null);
      setProblem(null);
    } else if (e.key === "Enter" && (!rows || e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      commit();
    }
  };
  return (
    <RowFrame
      common={{ ...common, error: message }}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={errId}
      stacked
      labelNode={
        <label className="w-setting-label" htmlFor={ids.control}>
          {common.label}
        </label>
      }
      control={
        rows ? (
          <textarea {...props} rows={rows} spellCheck={false} onKeyDown={onKey} onChange={(e) => (setDraft(e.target.value), setProblem(null))} />
        ) : (
          <input {...props} type="text" onKeyDown={onKey} onChange={(e) => (setDraft(e.target.value), setProblem(null))} />
        )
      }
    />
  );
}

/* ---- Secret ---- */

export interface WSettingsSecretRowProps extends WSettingsRowCommon {
  /** Whether the app holds a value. The value itself is never passed in and never drawn. */
  saved: boolean;
  /** The typed value, trimmed, never empty. Return `true` (or a promise of it) once the app holds the value: the typed
   *  text is cleared and editing ends at once. Anything else, a `false`, a rejection or no answer at all, is a failure
   *  and the text stays so it can be corrected. A key is never thrown away on a guess. */
  onSave: (value: string) => boolean | Promise<boolean>;
  onRemove?: () => void;
  /** Ask first: "Remove this key?" The question, and the part draws an inline two-step confirm. */
  removeQuestion?: string;
  /** The line shown while saved, such as "Key saved". */
  savedText?: string;
  /** How a saved value is masked. Decoration only: the real value is not here. */
  mask?: string;
  hint?: string;
  placeholder?: string;
  /** A state in words beside the buttons: "Checking the key…", "Key works". */
  status?: string;
  /** A save or a removal is in progress. */
  busy?: boolean;
  saveLabel?: string;
}

/** A key or token. Empty: a password box and Save. Saved: a masked line with Replace and Remove, and no value in the
 *  page. Editing: the box again, with Cancel. */
export function WSettingsSecretRow({ saved, onSave, onRemove, removeQuestion, savedText, mask = "••••••••", hint, placeholder, status, busy, saveLabel, ...common }: WSettingsSecretRowProps) {
  const { ids } = useRowIds(common);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [confirming, setConfirming] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const view = secretView(saved, editing);
  const alive = useRef(true);
  const sending = useRef(false);
  // The typed text goes the moment the app holds the value, and with the row: it must not linger in the page.
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      sending.current = false;
    };
  }, []);
  useEffect(() => {
    if (saved) {
      setDraft("");
      setEditing(false);
    }
  }, [saved]);
  useEffect(() => {
    if (editing) input.current?.focus();
  }, [editing]);
  const submit = () => {
    const v = secretToSave(draft);
    if (v === null || busy || sending.current) return;
    sending.current = true;
    const done = (ok: boolean) => {
      sending.current = false;
      if (ok && alive.current) {
        setDraft("");
        setEditing(false);
      }
    };
    let result: ReturnType<typeof onSave>;
    try {
      result = onSave(v);
    } catch {
      return done(false);
    }
    Promise.resolve(result).then(
      (r) => done(r === true),
      () => done(false),
    );
  };
  const cancel = () => {
    setDraft("");
    setEditing(false);
    setTimeout(() => document.getElementById(ids.control)?.focus(), 0);
  };
  const off = common.disabled || busy;
  const body =
    view === "saved" ? (
      <div className="w-secret-saved">
        <p className="w-secret-line">
          <WIcon name="check" size={16} />
          <span className="w-secret-state">{savedText ?? t("Saved")}</span>
          <span className="w-secret-mask" aria-hidden="true">
            {mask}
          </span>
        </p>
        {confirming && removeQuestion ? (
          <WInlineConfirm
            question={removeQuestion}
            confirmLabel={t("Remove")}
            onConfirm={() => {
              setConfirming(false);
              onRemove?.();
            }}
            onCancel={() => setConfirming(false)}
            returnFocusTo={ids.control}
          />
        ) : (
          <div className="w-row-actions">
            <WButton id={ids.control} onClick={() => setEditing(true)} disabled={off} aria-label={t("Replace {name}", { name: common.label })}>
              {t("Replace")}
            </WButton>
            {onRemove ? (
              <WButton variant="quiet" onClick={() => (removeQuestion ? setConfirming(true) : onRemove())} disabled={off} aria-label={t("Remove {name}", { name: common.label })}>
                {t("Remove")}
              </WButton>
            ) : null}
          </div>
        )}
      </div>
    ) : (
      <div className="w-secret-edit">
        <input
          ref={input}
          id={ids.control}
          className="w-input w-secret-input"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={draft}
          placeholder={placeholder}
          disabled={off}
          aria-describedby={[ids.desc, ids.reason, hint ? ids.extra : undefined, common.error ? ids.error : undefined].filter(Boolean).join(" ") || undefined}
          aria-invalid={common.error ? true : undefined}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Enter") {
              e.preventDefault();
              e.stopPropagation();
              submit();
            } else if (e.key === "Escape" && view === "editing") {
              e.preventDefault();
              e.stopPropagation();
              cancel();
            }
          }}
        />
        <div className="w-row-actions">
          <WButton variant="primary" onClick={submit} disabled={off || secretToSave(draft) === null} aria-describedby={!off && secretToSave(draft) === null ? `${ids.error}-s` : undefined}>
            {saveLabel ?? t("Save")}
          </WButton>
          <span className="w-visually-hidden" id={`${ids.error}-s`}>
            {t("Type or paste the key to turn Save on.")}
          </span>
          {view === "editing" ? <WButton onClick={cancel}>{t("Cancel")}</WButton> : null}
        </div>
      </div>
    );
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      stacked
      labelNode={
        view === "saved" ? (
          <span className="w-setting-label" id={ids.label}>
            {common.label}
          </span>
        ) : (
          <label className="w-setting-label" htmlFor={ids.control}>
            {common.label}
          </label>
        )
      }
      control={
        <>
          {body}
          {hint ? (
            <WSettingsNote id={ids.extra}>{hint}</WSettingsNote>
          ) : null}
          {status ? (
            <p className="w-setting-status" role="status">
              {status}
            </p>
          ) : null}
        </>
      }
    />
  );
}

/* ---- Lists ---- */

export interface WSettingsListAdd {
  /** The field's name: "App to exclude". */
  label: string;
  placeholder?: string;
  addLabel?: string;
  /** Entries already in the list, so a repeat is refused. */
  existing?: readonly string[];
  /** Return a sentence to show it as the error and keep the text; return nothing to accept it and clear the field. */
  onAdd: (value: string) => string | null | void | Promise<string | null | void>;
  disabled?: boolean;
}

export interface WSettingsListProps extends WSettingsRowCommon {
  children?: ReactNode;
  /** Make the list's label a real heading of this level. */
  heading?: 4 | 5;
  /** No entries: the line is shown in place of the list. */
  empty?: string;
  isEmpty?: boolean;
  filter?: { label: string; value: string; onChange: (value: string) => void; placeholder?: string };
  add?: WSettingsListAdd;
  /** Anything else under the list (suggestion chips, a Refresh button). */
  footer?: ReactNode;
}

function AddField({ add }: { add: WSettingsListAdd }) {
  const id = useId();
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    if (busy) return;
    const checked = checkAddDraft(text, add.existing);
    if (!checked.ok) {
      setError(checked.problem === "empty" ? t("Enter a value to add.") : t("{value} is already in the list.", { value: text.trim() }));
      return;
    }
    setBusy(true);
    try {
      const answer = await add.onAdd(checked.value);
      if (typeof answer === "string" && answer) setError(answer);
      else {
        setError(null);
        setText("");
      }
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="w-listadd">
      <div className="w-listadd-line">
        <input
          className="w-input"
          type="text"
          aria-label={add.label}
          aria-describedby={error ? `${id}-e` : undefined}
          aria-invalid={error ? true : undefined}
          autoComplete="off"
          placeholder={add.placeholder}
          value={text}
          disabled={add.disabled}
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              e.stopPropagation();
              void submit();
            }
          }}
        />
        <WButton onClick={() => void submit()} disabled={add.disabled || busy}>
          {add.addLabel ?? t("Add")}
        </WButton>
      </div>
      <div id={`${id}-e`} aria-live="polite">
        {error ? (
          <p className="w-field-error">
            <WIcon name="warning" size={16} />
            <span>{error}</span>
          </p>
        ) : null}
      </div>
    </div>
  );
}

/** A list the person edits: an optional filter, the rows, an empty line, and an add field with its own checks. */
export function WSettingsList({ children, empty, isEmpty, filter, add, footer, heading, ...common }: WSettingsListProps) {
  const { ids } = useRowIds(common);
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      stacked
      labelNode={
        <LabelText id={ids.label} level={heading}>
          {common.label}
        </LabelText>
      }
      control={
        <div className="w-list">
          {filter ? (
            <input
              className="w-input"
              type="search"
              aria-label={filter.label}
              placeholder={filter.placeholder}
              value={filter.value}
              disabled={common.disabled}
              onChange={(e) => filter.onChange(e.target.value)}
            />
          ) : null}
          {isEmpty ? (
            <p className="w-list-empty">{empty}</p>
          ) : (
            <ul className="w-list-rows" role="list" aria-labelledby={ids.label}>
              {children}
            </ul>
          )}
          {add ? <AddField add={add} /> : null}
          {footer}
        </div>
      }
    />
  );
}

export interface WSettingsListRowProps {
  title: string;
  /** Lines the person must read before acting (a code to compare, a warning), at the primary text size under the title. */
  details?: readonly { text: string; id?: string }[];
  caption?: string;
  chip?: WStatusChipProps;
  /** Buttons at the end of the row, or a WInlineConfirm in their place. */
  actions?: ReactNode;
  /** A switch at the end of the row, named by the title. */
  toggle?: { checked: boolean; onChange: (next: boolean) => void; disabled?: boolean };
  /** Rename in place: the title becomes a box with Save and Cancel. */
  edit?: { label: string; value: string; onSave: (value: string) => string | null | void; onCancel: () => void };
  highlighted?: boolean;
  id?: string;
}

function InlineEdit({ edit }: { edit: NonNullable<WSettingsListRowProps["edit"]> }) {
  const id = useId();
  const [text, setText] = useState(edit.value);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select?.();
  }, []);
  const save = () => {
    const v = text.trim();
    if (v === "") {
      setError(t("This can't be empty."));
      return;
    }
    const answer = edit.onSave(v);
    if (typeof answer === "string" && answer) setError(answer);
  };
  return (
    <div className="w-inedit">
      <div className="w-inedit-line">
        <input
          ref={input}
          className="w-input"
          type="text"
          aria-label={edit.label}
          aria-describedby={error ? `${id}-e` : undefined}
          aria-invalid={error ? true : undefined}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Enter") {
              e.preventDefault();
              e.stopPropagation();
              save();
            } else if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              edit.onCancel();
            }
          }}
        />
        <WButton variant="primary" onClick={save}>
          {t("Save")}
        </WButton>
        <WButton onClick={edit.onCancel}>{t("Cancel")}</WButton>
      </div>
      <div id={`${id}-e`} aria-live="polite">
        {error ? (
          <p className="w-field-error">
            <WIcon name="warning" size={16} />
            <span>{error}</span>
          </p>
        ) : null}
      </div>
    </div>
  );
}

export function WSettingsListRow({ title, details, caption, chip, actions, toggle, edit, highlighted, id }: WSettingsListRowProps) {
  const base = useId();
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (!highlighted) return;
    ref.current?.focus?.({ preventScroll: true });
    ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [highlighted]);
  return (
    <li className={highlighted ? "w-lrow w-setting-jumped" : "w-lrow"} id={id} ref={ref} tabIndex={highlighted ? -1 : undefined}>
      {edit ? (
        <InlineEdit edit={edit} />
      ) : (
        <>
          <div className="w-lrow-text">
            <span className="w-lrow-title" id={`${base}-t`}>
              {title}
              {chip ? <WStatusChip {...chip} /> : null}
            </span>
            {details?.map((d) => (
              <span className="w-lrow-detail" id={d.id} key={d.text}>
                {d.text}
              </span>
            ))}
            {caption ? <span className="w-lrow-caption">{caption}</span> : null}
          </div>
          {toggle || actions ? (
            <div className="w-lrow-actions">
              {actions}
              {toggle ? (
                <span className="w-switch-wrap">
                  <button
                    type="button"
                    role="switch"
                    className="w-switch"
                    aria-checked={toggle.checked}
                    aria-labelledby={`${base}-t`}
                    disabled={toggle.disabled}
                    onClick={() => toggle.onChange(!toggle.checked)}
                  />
                </span>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </li>
  );
}

/* ---- Links ---- */

export interface WSettingsLinkRowProps extends WSettingsRowCommon {
  /** "external": a link to a web page, opened in the browser. "section": a jump to another part of Settings. */
  kind: "external" | "section";
  /** External: the address. */
  href?: string;
  /** The words of the link or the destination ("Open the privacy policy", "Models"). Defaults to the label. */
  linkText?: string;
  /** Section: the jump. External: called first, and may stop the browser opening (to hand the address to the app). */
  onPress?: () => void;
}

export function WSettingsLinkRow({ kind, href, linkText, onPress, ...common }: WSettingsLinkRowProps) {
  const { ids, described } = useRowIds(common);
  if (kind === "section") {
    return (
      <RowFrame
        common={{ ...common, description: undefined }}
        descId={undefined}
        reasonId={ids.reason}
        errorId={ids.error}
        className="w-setting-click"
        labelNode={
          <button type="button" className="w-setting-press" disabled={common.disabled} aria-describedby={described} onClick={onPress}>
            <span className="w-setting-text">
              <span className="w-setting-label">{common.label}</span>
              {common.description ? (
                <span className="w-setting-desc" id={ids.desc}>
                  {common.description}
                </span>
              ) : null}
            </span>
            <span className="w-setting-value">
              {linkText ?? ""}
              <WIcon name="chevronRight" size={16} />
            </span>
          </button>
        }
      />
    );
  }
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <a
          className="w-link"
          href={href}
          rel="noopener noreferrer"
          aria-describedby={described}
          // The app opens the address (click, Return and middle click alike); the page itself never navigates or opens a window.
          onClick={(e) => {
            e.preventDefault();
            onPress?.();
          }}
          onAuxClick={(e) => {
            if (e.button !== 1) return;
            e.preventDefault();
            onPress?.();
          }}
        >
          <span>{linkText ?? common.label}</span>
          <WIcon name="external" size={16} />
          <span className="w-visually-hidden">{t("Opens in your browser")}</span>
        </a>
      }
    />
  );
}

/* ---- Read-only values ---- */

export interface WSettingsValueItem {
  key: string;
  label: string;
  value: string;
  /** Show a Copy button for this value. */
  copy?: boolean;
  /** Draw the value in the mono face (paths, ids). Long values wrap anywhere; they never widen the page. */
  mono?: boolean;
}

export interface WSettingsValueListProps {
  /** What the list is: "About this PC". */
  label: string;
  items: readonly WSettingsValueItem[];
  /** Copies a value. Defaults to the clipboard. Return a promise (or false) so "Copied" is only said when it worked. */
  onCopy?: (item: WSettingsValueItem) => void | boolean | Promise<void | boolean>;
}

/** Writes text to the clipboard and says whether it worked: false when there is no clipboard or the write is refused. */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function WSettingsValueList({ label, items, onCopy }: WSettingsValueListProps) {
  const [copied, setCopied] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const copy = (item: WSettingsValueItem) => {
    const done = async (): Promise<boolean> => {
      if (!onCopy) return copyToClipboard(item.value);
      try {
        return (await onCopy(item)) !== false;
      } catch {
        return false;
      }
    };
    void done().then((ok) => {
      setCopied(ok ? item.key : null);
      setFailed(ok ? null : item.key);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setCopied(null);
        setFailed(null);
      }, 2000);
    });
  };
  return (
    <dl className="w-values" aria-label={label}>
      {items.map((item) => (
        <div className="w-values-row" key={item.key}>
          <dt>{item.label}</dt>
          <dd className={item.mono ? "w-values-mono" : undefined}>
            <span className="w-values-text">{item.mono ? breakAtSeparators(item.value) : item.value}</span>
            {item.copy ? <WIconButton icon="copy" label={t("Copy {name}", { name: item.label })} onClick={() => copy(item)} /> : null}
            {copied === item.key ? (
              <span className="w-values-copied" role="status">
                {t("Copied")}
              </span>
            ) : null}
            {failed === item.key ? (
              <span className="w-values-copied w-values-copyfail" role="alert">
                {t("Could not copy")}
              </span>
            ) : null}
          </dd>
        </div>
      ))}
    </dl>
  );
}

/* ---- Progress ---- */

export interface WSettingsProgressRowProps extends WSettingsRowCommon {
  /** 0 to 1, or null while the size is not known. */
  progress: number | null;
  /** "312 MB of 1.2 GB". Read with the bar. */
  valueText?: string;
  onCancel?: () => void;
  cancelLabel?: string;
  /** A line about the work: "Cancelling…". */
  status?: string;
  /** The bar is past its limit: it is drawn in the problem colour, and the caller says so in words beside it. */
  over?: boolean;
  /** What a screen reader hears as the work changes ("Downloading…", "Done"), in a live region the sighted do not see: the
   *  percent is not announced on every step. */
  announce?: string;
}

export function WSettingsProgressRow({ progress, valueText, onCancel, cancelLabel, status, over, announce, ...common }: WSettingsProgressRowProps) {
  const { ids } = useRowIds(common);
  const known = progress !== null && Number.isFinite(progress);
  const percent = known ? Math.round(Math.min(1, Math.max(0, progress as number)) * 100) : null;
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      stacked
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <div className="w-prow">
          <div className="w-prow-head">
            <span className="w-prow-value">{valueText ?? (percent !== null ? t("{percent}%", { percent }) : "")}</span>
            {onCancel ? (
              <WButton variant="quiet" onClick={onCancel} aria-label={`${cancelLabel ?? t("Cancel")}, ${common.label}`}>
                {cancelLabel ?? t("Cancel")}
              </WButton>
            ) : null}
          </div>
          <div
            className={[known ? "w-progress-track" : "w-progress-track w-progress-unknown", over ? "w-progress-over" : ""].filter(Boolean).join(" ")}
            role="progressbar"
            aria-labelledby={ids.label}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent ?? undefined}
            aria-valuetext={valueText}
          >
            <div className="w-progress-fill" style={known ? { width: `${percent}%` } : undefined} />
          </div>
          {status ? (
            <p className="w-setting-status" role="status">
              {status}
            </p>
          ) : null}
          {announce !== undefined ? (
            <span className="w-visually-hidden" role="status">
              {announce}
            </span>
          ) : null}
        </div>
      }
    />
  );
}

/* ---- Disclosure ---- */

export interface WSettingsDisclosureProps {
  title: string;
  /** One line shown beside the title while closed. */
  summary?: string;
  children: ReactNode;
  /** Controlled. Leave both out and it keeps its own state. */
  open?: boolean;
  onToggle?: (open: boolean) => void;
  defaultOpen?: boolean;
  id?: string;
}

/** A button that opens and closes a group of rows. `aria-expanded` says which. */
export function WSettingsDisclosure({ title, summary, children, open, onToggle, defaultOpen, id }: WSettingsDisclosureProps) {
  const base = useId();
  const [own, setOwn] = useState(!!defaultOpen);
  const isOpen = open ?? own;
  const toggle = () => {
    const next = !isOpen;
    if (open === undefined) setOwn(next);
    onToggle?.(next);
  };
  return (
    <div className="w-disc" id={id}>
      <button type="button" className="w-disc-button" aria-expanded={isOpen} aria-controls={`${base}-p`} onClick={toggle}>
        <WIcon name="chevronRight" size={16} className={isOpen ? "w-disc-chev w-disc-open" : "w-disc-chev"} />
        <span className="w-disc-title">{title}</span>
        {summary && !isOpen ? <span className="w-disc-summary">{summary}</span> : null}
      </button>
      <div className="w-disc-panel" id={`${base}-p`} role="group" aria-label={title} hidden={!isOpen}>
        {children}
      </div>
    </div>
  );
}

/* ---- Shortcut ---- */

export interface WSettingsShortcutRowProps extends WSettingsRowCommon {
  /** The current combination, "Ctrl+Shift+Space", or null when none is set. */
  keys: string | null;
  /** The row is waiting for the new combination. The caller owns the capture. */
  recording: boolean;
  onRecord: () => void;
  onClear?: () => void;
  onCancel: () => void;
  recordLabel?: string;
  /** Words while recording. */
  recordingText?: string;
}

export function WSettingsShortcutRow({ keys, recording, onRecord, onClear, onCancel, recordLabel, recordingText, ...common }: WSettingsShortcutRowProps) {
  const { ids } = useRowIds(common);
  const parts = keys ? keys.split("+") : [];
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <div className="w-short">
          <span className="w-short-keys" aria-label={keys ? keys.replace(/\+/g, " plus ") : t("Not set")}>
            {parts.length === 0 ? (
              <span className="w-short-none">{t("Not set")}</span>
            ) : (
              parts.map((p, i) => (
                <kbd key={i} className="w-kbd" aria-hidden="true">
                  {p}
                </kbd>
              ))
            )}
          </span>
          {recording ? (
            <>
              <span className="w-short-wait" role="status">
                {recordingText ?? t("Press the new shortcut")}
              </span>
              <WButton onClick={onCancel}>{t("Cancel")}</WButton>
            </>
          ) : (
            <>
              <WButton onClick={onRecord} disabled={common.disabled} aria-label={`${recordLabel ?? t("Record")}, ${common.label}`}>
                {recordLabel ?? t("Record")}
              </WButton>
              {onClear && keys ? (
                <WButton variant="quiet" onClick={onClear} disabled={common.disabled} aria-label={`${t("Clear")}, ${common.label}`}>
                  {t("Clear")}
                </WButton>
              ) : null}
            </>
          )}
        </div>
      }
    />
  );
}

/* ---- The original row, now with the shared layout ---- */

export interface WSettingsRowProps extends WSettingsRowCommon {
  /** toggle: a switch. checkbox: a checkbox with the description as its muted note. value: a press row with the
   *  current value and a chevron. button and destructive: one button at the end. */
  variant: "toggle" | "checkbox" | "value" | "button" | "destructive";
  /** Toggle and checkbox: on or off. */
  checked?: boolean;
  onToggle?: (next: boolean) => void;
  /** Value: the current value, shown before the chevron. */
  value?: string;
  /** Value, button, destructive: what happens on press. */
  onPress?: () => void;
  /** Button and destructive: the button's words. Defaults to the label. */
  actionLabel?: string;
}

export function WSettingsRow({ variant, checked, onToggle, value, onPress, actionLabel, ...common }: WSettingsRowProps) {
  const { ids, described } = useRowIds(common);
  if (variant === "toggle" || variant === "checkbox") {
    return <WSettingsToggleRow kind={variant === "checkbox" ? "checkbox" : "switch"} checked={!!checked} onToggle={(next) => onToggle?.(next)} {...common} />;
  }
  if (variant === "value") return <WSettingsLinkRow kind="section" linkText={value} onPress={onPress} {...common} />;
  return (
    <RowFrame
      common={common}
      descId={ids.desc}
      reasonId={ids.reason}
      errorId={ids.error}
      className={variant === "destructive" ? "w-setting-destructive" : undefined}
      labelNode={
        <span className="w-setting-label" id={ids.label}>
          {common.label}
        </span>
      }
      control={
        <WButton
          variant={variant === "destructive" ? "destructive" : "secondary"}
          onClick={onPress}
          disabled={common.disabled}
          aria-describedby={described}
          aria-label={actionLabel ? `${actionLabel}, ${common.label}` : undefined}
        >
          {actionLabel ?? common.label}
        </WButton>
      }
    />
  );
}
