// Parts of a one-column flow: launch, first run, anything that walks a person through steps. The page is one calm
// column on the page color; each part follows docs/ui-redesign/screens/onboarding.md. Colors come from the skin.

import { Children, useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode, type SyntheticEvent } from "react";
import { t } from "../l10n";
import { WButton } from "./controls";
import { WIcon, type WIconName } from "./WIcon";
import "./system.css";

/* ---- Keys ---- */

/** What a key press means on a flow page. Pure, so it can be tested without a browser. Return does the primary
 *  action, Escape goes back or cancels. Return is left alone on a button, a link, a checkbox and a text area (where it
 *  is a new line unless Ctrl or Cmd is held), while a field has not already taken it, and while a dialog or an input
 *  method is open. */
export interface FlowKeyEvent {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  defaultPrevented?: boolean;
  composing?: boolean;
  /** The element the key went to: its tag, its `type` and its `role`. */
  target: { tag: string; type?: string; role?: string; inDialog?: boolean };
}

export function flowKeyAction(e: FlowKeyEvent): "primary" | "back" | null {
  if (e.defaultPrevented || e.composing || e.target.inDialog) return null;
  if (e.key === "Escape") return "back";
  if (e.key !== "Enter") return null;
  const tag = e.target.tag.toLowerCase();
  const modified = !!(e.ctrlKey || e.metaKey);
  if (tag === "textarea") return modified ? "primary" : null;
  if (tag === "button" || tag === "a" || tag === "select" || tag === "summary") return null;
  if (tag === "input" && (e.target.type === "checkbox" || e.target.type === "radio" || e.target.type === "button")) return null;
  if (e.target.role === "radio" || e.target.role === "menuitem" || e.target.role === "switch") return null;
  return "primary";
}

/* ---- The page ---- */

export interface WFlowPageProps {
  /** "Step 2 of 4". Leave out when the flow has one step. */
  step?: { current: number; total: number };
  title: string;
  /** One short paragraph saying what this step is for. */
  intro?: ReactNode;
  /** The work of the step. */
  children?: ReactNode;
  /** Bottom right. The primary action goes last, and there is one primary only. */
  actions?: ReactNode;
  /** Under the actions, in the secondary text role. */
  finePrint?: ReactNode;
  /** Something comes before the title, such as a notice about what just happened. */
  lead?: ReactNode;
  /** Working: a quiet bar under the title, and the words read out. */
  busyLabel?: string;
  /** Return runs this. Pass it only while the primary action is allowed. */
  onPrimary?: () => void;
  /** Escape runs this, where the step allows going back or cancelling. */
  onBack?: () => void;
  /** Goes with the page's keys; a dialog inside the page takes its own. */
  className?: string;
}

export function WFlowPage({ step, title, intro, children, actions, finePrint, lead, busyLabel, onPrimary, onBack, className }: WFlowPageProps) {
  const titleRef = useRef<HTMLHeadingElement>(null);
  const stepKey = `${step?.current ?? 0}:${title}`;
  // Focus goes to the title when a step appears, so a screen reader starts at the top of it.
  useEffect(() => {
    titleRef.current?.focus({ preventScroll: true });
  }, [stepKey]);

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const el = e.target as HTMLElement;
    const action = flowKeyAction({
      key: e.key,
      ctrlKey: e.ctrlKey,
      metaKey: e.metaKey,
      defaultPrevented: e.defaultPrevented,
      composing: e.nativeEvent.isComposing,
      target: { tag: el.tagName ?? "", type: (el as HTMLInputElement).type, role: el.getAttribute?.("role") ?? undefined, inDialog: !!el.closest?.('[role="alertdialog"]') },
    });
    if (action === "primary" && onPrimary) {
      e.preventDefault();
      onPrimary();
    } else if (action === "back" && onBack) {
      e.preventDefault();
      onBack();
    }
  };

  return (
    <div className="w-sys w-flow-root">
      <main className={className ? `w-flow ${className}` : "w-flow"} onKeyDown={onKeyDown}>
        {lead}
        {step ? (
          <p className="w-flow-step" role="status">
            {t("Step {step} of {total}", { step: step.current, total: step.total })}
          </p>
        ) : null}
        <h1 className="w-flow-title" tabIndex={-1} ref={titleRef}>
          {title}
        </h1>
        {intro ? <div className="w-flow-intro">{intro}</div> : null}
        {busyLabel ? (
          <div className="w-flow-busy" role="status" aria-busy="true">
            <div className="w-quiet-bar" aria-hidden="true" />
            <span className="w-visually-hidden">{busyLabel}</span>
          </div>
        ) : null}
        {Children.toArray(children).length > 0 ? <div className="w-flow-body">{children}</div> : null}
        {actions ? <div className="w-flow-actions">{actions}</div> : null}
        {finePrint ? <div className="w-flow-fine">{finePrint}</div> : null}
      </main>
    </div>
  );
}

/** A page with nothing on it but a quiet bar and a line, for a wait the person did not start. */
export function WFlowWait({ label }: { label: string }) {
  return (
    <div className="w-sys w-flow-root w-flow-wait" role="status" aria-busy="true">
      <div className="w-quiet-bar" aria-hidden="true" />
      <p>{label}</p>
    </div>
  );
}

/** A box that groups the work of a step. */
export function WPanel({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={className ? `w-panel ${className}` : "w-panel"}>{children}</div>;
}

/* ---- Fields ---- */

export interface WTextFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: "text" | "password";
  /** The rule, said before anyone types. */
  help?: string;
  /** A live count or state tied to the field, such as "0 of at least 12 characters". */
  status?: { text: string; tone?: "ok" | "muted" };
  /** What is wrong, directly under the field. */
  error?: string | null;
  disabled?: boolean;
  autoComplete?: string;
  /** A text area with this many rows, in the mono face. */
  rows?: number;
  /** Return in this field does this instead of the page's primary action. */
  onEnter?: () => void;
}

export function WTextField({ label, value, onChange, type = "text", help, status, error, disabled, autoComplete, rows, onEnter }: WTextFieldProps) {
  const id = useId();
  const helpId = `${id}-help`;
  const statusId = `${id}-status`;
  const errorId = `${id}-error`;
  const described = [help ? helpId : null, status ? statusId : null, errorId].filter(Boolean).join(" ");
  const common = {
    id,
    value,
    disabled,
    autoComplete: autoComplete ?? "off",
    "aria-describedby": described,
    "aria-invalid": error ? true : undefined,
    onChange: (e: SyntheticEvent<HTMLInputElement | HTMLTextAreaElement>) => onChange((e.target as HTMLInputElement).value),
    onKeyDown: onEnter
      ? (e: KeyboardEvent<HTMLElement>) => {
          if (e.key === "Enter" && !e.nativeEvent.isComposing) {
            e.preventDefault();
            onEnter();
          }
        }
      : undefined,
  };
  return (
    <div className="w-field">
      <label className="w-field-label" htmlFor={id}>
        {label}
      </label>
      {help ? (
        <p className="w-field-help" id={helpId}>
          {help}
        </p>
      ) : null}
      {rows ? (
        <textarea className="w-input w-input-mono" rows={rows} spellCheck={false} autoCorrect="off" autoCapitalize="off" {...common} />
      ) : (
        <input className="w-input" type={type} {...common} />
      )}
      {status ? (
        <p className={status.tone === "ok" ? "w-field-status w-field-ok" : "w-field-status"} id={statusId}>
          {status.text}
        </p>
      ) : null}
      <div className="w-field-message" id={errorId} aria-live="polite">
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

export function WCheckbox({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange: (next: boolean) => void; disabled?: boolean }) {
  return (
    <label className={disabled ? "w-check w-check-disabled" : "w-check"}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

/* ---- Messages ---- */

const NOTICE_ICON: Record<"info" | "ok" | "warning" | "error", WIconName> = { info: "info", ok: "check", warning: "warning", error: "warning" };

/** A line that says how things stand: it keeps the words, and the glyph and the box say the tone, so color is never alone. */
export function WNotice({ tone = "info", strong, children }: { tone?: "info" | "ok" | "warning" | "error"; strong?: boolean; children: ReactNode }) {
  return (
    <div className={`w-notice w-notice-${tone}`} role={tone === "error" ? "alert" : "status"}>
      <WIcon name={NOTICE_ICON[tone]} />
      <p className={strong ? "w-notice-strong" : undefined}>{children}</p>
    </div>
  );
}

/* ---- A banner on a page that is already open ---- */

export interface WBannerProps {
  /** What the banner is, for a screen reader; used as its name unless there is a visible title. */
  label: string;
  tone?: "info" | "warning";
  /** A visible title (a level-2 heading). */
  title?: string;
  children: ReactNode;
  /** Bottom right, the primary action last. */
  actions?: ReactNode;
}

/** A notice that sits above the page content with something to say and something to do. */
export function WBanner({ label, tone = "info", title, children, actions }: WBannerProps) {
  const id = useId();
  return (
    <section className={`w-sys w-banner w-banner-${tone}`} role="region" aria-label={title ? undefined : label} aria-labelledby={title ? id : undefined}>
      {title ? (
        <h2 className="w-banner-title" id={id}>
          {title}
        </h2>
      ) : null}
      <div className="w-banner-body">{children}</div>
      {actions ? <div className="w-banner-actions">{actions}</div> : null}
    </section>
  );
}

/* ---- Progress ---- */

/** A determinate bar with its label and its percent. `value` is 0 to 1. `labelHidden` keeps the name for a screen
 *  reader when the page title already says what is going on. */
export function WProgressLine({ label, value, labelHidden }: { label: string; value: number; labelHidden?: boolean }) {
  const v = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
  const percent = Math.round(v * 100);
  return (
    <div className="w-progress">
      <div className="w-progress-head">
        <span className={labelHidden ? "w-visually-hidden" : undefined}>{label}</span>
        <span className="w-progress-value">{t("{percent}%", { percent })}</span>
      </div>
      <div className="w-progress-track" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
        <div className="w-progress-fill" style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

/* ---- Lists ---- */

const refuse = (e: SyntheticEvent) => e.preventDefault();

export interface WNumberedListProps {
  items: readonly string[];
  /** What the list is, for a screen reader. */
  label: string;
  columns?: number;
  /** Refuses selecting, copying, cutting, the context menu and dragging: for a list that is a secret. */
  protectedFromCopy?: boolean;
}

/** Numbered 1. to n., read one by one, filled down each column. */
export function WNumberedList({ items, label, columns = 3, protectedFromCopy }: WNumberedListProps) {
  const rows = Math.ceil(items.length / columns);
  const guard = protectedFromCopy ? { onCopy: refuse, onCut: refuse, onContextMenu: refuse, onDragStart: refuse } : {};
  return (
    <ol
      className={protectedFromCopy ? "w-numbered w-numbered-protected" : "w-numbered"}
      role="list"
      aria-label={label}
      style={{ gridTemplateRows: `repeat(${rows}, auto)`, gridTemplateColumns: `repeat(${columns}, minmax(0, max-content))` }}
      {...guard}
    >
      {items.map((item, i) => (
        <li key={i} className="w-numbered-item">
          <span className="w-numbered-n" aria-hidden="true">{`${i + 1}.`}</span>
          <span>{item}</span>
        </li>
      ))}
    </ol>
  );
}

/* ---- Choosing ---- */

export interface WSegmentedProps<V extends string> {
  label: string;
  value: V;
  options: readonly { value: V; label: string }[];
  onChange: (value: V) => void;
}

/** Two or three exclusive choices side by side. One tab stop; the arrow keys move and choose. */
export function WSegmented<V extends string>({ label, value, options, onChange }: WSegmentedProps<V>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const move = (from: number, step: number) => {
    const next = (from + step + options.length) % options.length;
    onChange(options[next].value);
    refs.current[next]?.focus();
  };
  return (
    <div className="w-segmented" role="radiogroup" aria-label={label}>
      {options.map((o, i) => (
        <button
          key={o.value}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          tabIndex={o.value === value ? 0 : -1}
          className={o.value === value ? "w-segment w-segment-on" : "w-segment"}
          onClick={() => onChange(o.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowRight" || e.key === "ArrowDown") {
              e.preventDefault();
              move(i, 1);
            } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
              e.preventDefault();
              move(i, -1);
            }
          }}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/* ---- A question that must be answered ---- */

export interface WDialogProps {
  title: string;
  body?: string;
  /** The buttons. Mark the safe one `data-initial` so it has focus when the dialog opens. */
  children: ReactNode;
  /** Escape. */
  onCancel: () => void;
}

/** An alert dialog over the page. Focus goes to the safe button, Tab stays inside, Escape cancels, and focus returns
 *  to where it was. The page's own keys ignore it. */
export function WDialog({ title, body, children, onCancel }: WDialogProps) {
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const target = box.current?.querySelector<HTMLElement>("[data-initial]") ?? box.current?.querySelector<HTMLElement>("button");
    target?.focus({ preventScroll: true });
    return () => before?.focus?.();
  }, []);
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else if (e.key === "Tab") {
      const items = [...(box.current?.querySelectorAll<HTMLElement>("button:not(:disabled)") ?? [])];
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  return (
    <div className="w-dialog-scrim">
      <div ref={box} className="w-dialog" role="alertdialog" aria-modal="true" aria-labelledby={`${id}-t`} aria-describedby={body ? `${id}-b` : undefined} onKeyDown={onKeyDown}>
        <h2 className="w-dialog-title" id={`${id}-t`}>
          {title}
        </h2>
        {body ? (
          <p className="w-dialog-body" id={`${id}-b`}>
            {body}
          </p>
        ) : null}
        <div className="w-dialog-actions">{children}</div>
      </div>
    </div>
  );
}

/* ---- More options ---- */

export interface WMoreMenuItem {
  key: string;
  label: string;
  onPress: () => void;
  disabled?: boolean;
}

/** The actions that do not fit in a row of three. */
export function WMoreMenu({ items, disabled }: { items: readonly WMoreMenuItem[]; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (open) first.current?.focus();
  }, [open]);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) box.current?.querySelector<HTMLElement>("button")?.focus();
  };
  return (
    <span
      className="w-more"
      ref={box}
      onBlur={(e) => {
        if (open && !box.current?.contains(e.relatedTarget as Node | null)) setOpen(false);
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          e.stopPropagation();
          close(true);
        }
      }}
    >
      <WButton variant="quiet" icon="chevron" aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => setOpen(!open)}>
        {t("More options")}
      </WButton>
      {open ? (
        <ul className="w-more-menu" role="menu">
          {items.map((item, i) => (
            <li key={item.key} role="none">
              <button
                ref={i === 0 ? first : undefined}
                type="button"
                role="menuitem"
                className="w-more-item"
                disabled={item.disabled}
                onClick={() => {
                  close(false);
                  item.onPress();
                }}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </span>
  );
}
