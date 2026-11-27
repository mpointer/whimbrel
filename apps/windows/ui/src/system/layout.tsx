import type { ReactNode } from "react";
import { t } from "../l10n";
import { WButton, WSpeakerChip } from "./controls";
import { WIcon, type WIconName } from "./WIcon";
import "./system.css";

/* ---- Sidebar ---- */

export interface WSidebarItemProps {
  icon: WIconName;
  /** Replaces the icon (the selected skin's own glyph for the section). */
  glyph?: ReactNode;
  label: string;
  selected?: boolean;
  badge?: number | string;
  /** "Ctrl+1" */
  shortcut?: string;
  onPress: () => void;
  /** The button's element id (the shell's tests and the matrix find sections by it). */
  id?: string;
  /** Names the badge for assistive technology ("2 actions waiting for approval"). */
  badgeLabel?: string;
}

export function WSidebarItem({ icon, glyph, label, selected, badge, shortcut, onPress, id, badgeLabel }: WSidebarItemProps) {
  return (
    <li>
      <button
        type="button"
        id={id}
        className="w-sidebar-item"
        aria-current={selected ? "page" : undefined}
        aria-keyshortcuts={shortcut ? shortcut.replace(/\+/g, "+") : undefined}
        onClick={onPress}
      >
        {glyph ?? <WIcon name={icon} />}
        <span className="w-sidebar-label">{label}</span>
        {badge !== undefined ? (
          <span className="w-badge" role={badgeLabel ? "status" : undefined} aria-label={badgeLabel}>
            {badge}
          </span>
        ) : null}
        {shortcut ? <span className="w-sidebar-hint">{shortcut}</span> : null}
      </button>
    </li>
  );
}

/** `footerClass` adds a class to the foot, for the hooks a host screen looks for (`sidebar-footer`). */
export function WSidebar({ items, footer, label, footerClass, between }: { items: ReactNode; footer?: ReactNode; label?: string; footerClass?: string; between?: ReactNode }) {
  return (
    <nav className="w-sidebar" aria-label={label ?? t("Main")}>
      <ul className="w-sidebar-list">{items}</ul>
      {between}
      {footer ? <div className={footerClass ? `w-sidebar-footer ${footerClass}` : "w-sidebar-footer"}>{footer}</div> : null}
    </nav>
  );
}

/* ---- Spine ---- */

export type WNodeState = "upcoming" | "now" | "past" | "recording";

export interface WSpineStopProps {
  /** "10:00 AM" */
  time: string;
  /** A small second line under the time, such as a title. */
  timeNote?: string;
  node: WNodeState;
  /** The line below this node is solid when time has passed along it. */
  connector?: "solid" | "dashed";
  last?: boolean;
  /** The stop's name for assistive technology, with its time: "10:31 AM, Northwind security review, in 9 minutes". */
  label?: string;
  /** The stop is the hero (the next or live meeting). The ruled time line draws no rule beside it. */
  hero?: boolean;
  children: ReactNode;
}

export function WSpineStop({ time, timeNote, node, connector = "dashed", last, label, hero, children }: WSpineStopProps) {
  const className = ["w-spine-stop", last ? "w-spine-last" : "", hero ? "w-spine-hero" : ""].filter(Boolean).join(" ");
  return (
    <li className={className} data-node={node} aria-label={label}>
      <span className="w-spine-time">
        {time}
        {timeNote ? <span className="w-spine-sub">{timeNote}</span> : null}
      </span>
      <span className={connector === "solid" ? "w-spine-rail w-spine-connector-solid" : "w-spine-rail"} aria-hidden="true">
        <span className={`w-spine-node w-spine-node-${node}`} />
      </span>
      <div className="w-spine-body">{children}</div>
    </li>
  );
}

/** `ruled` draws the same stops as rows between rules, with no rail or nodes (the skin's time line choice). */
export function WSpine({ label, children, variant = "line" }: { label: string; children: ReactNode; variant?: "line" | "ruled" }) {
  return (
    <ol className={variant === "ruled" ? "w-spine w-spine-ruled" : "w-spine"} aria-label={label}>
      {children}
    </ol>
  );
}

/** The rule a skin draws between Home's sections (`decoration.dividers`, drawn by system.css under `data-dividers`). It is
 *  decoration, hidden from Narrator, and draws nothing under the `none` style; callers leave it out for Thread. */
export function WDivider() {
  return <div className="w-divider" aria-hidden="true" />;
}

/** The line that marks the present: "Now  Memory on since 8:02 AM". */
export function WNowMarker({ text }: { text: string }) {
  return (
    <div className="w-now">
      <span className="w-now-label">
        <b>{t("Now")}</b>
        <span>{text}</span>
      </span>
      <span className="w-now-rule" aria-hidden="true" />
    </div>
  );
}

/** The present as a stop of the time line: announced once as "Now, 10:22 AM", never focusable. */
export function WSpineNow({ time, text, connector = "solid", last }: { time: string; text?: string; connector?: "solid" | "dashed"; last?: boolean }) {
  return (
    <li className={last ? "w-spine-stop w-spine-last" : "w-spine-stop"} data-node="now" aria-label={t("Now, {time}", { time })}>
      <span className="w-spine-time" aria-hidden="true">
        {time}
      </span>
      <span className={connector === "solid" ? "w-spine-rail w-spine-connector-solid" : "w-spine-rail"} aria-hidden="true">
        <span className="w-spine-node w-spine-node-now" />
      </span>
      <div className="w-spine-body" aria-hidden="true">
        <WNowMarker text={text ?? ""} />
      </div>
    </li>
  );
}

/* ---- Meeting card ---- */

export interface WMeetingCardProps {
  variant: "next" | "upcoming" | "past";
  title: string;
  /** "10:00 AM" or "Today, 10:00 AM. Last met Tue, Nov 10." */
  when?: string;
  /** "in 8 minutes" */
  soon?: string;
  attendees: string[];
  /** One line for a past meeting. */
  gist?: string;
  /** Without it the row has no action (a past meeting nobody recorded). */
  primaryLabel?: string;
  onPrimary?: () => void;
  primaryHint?: string;
  secondaryLabel?: string;
  onSecondary?: () => void;
  /** Actions drawn as text links (the skin's `actions: links` choice). */
  links?: boolean;
  /** Drawn as a still picture: its buttons are not buttons (see WButton). */
  still?: boolean;
  /** Next only: drawn above the title (the live recording indicator). */
  indicator?: ReactNode;
  /** Next only: the tag's words when they are not "Next, in 8 minutes" (a meeting in progress). */
  tag?: string;
}

export function WMeetingCard(p: WMeetingCardProps) {
  if (p.variant === "next") {
    return (
      <article className="w-meeting w-meeting-next" aria-label={p.title}>
        {p.indicator}
        <span className="w-meeting-tag">{p.tag ?? (p.soon ? t("Next, {when}", { when: p.soon }) : t("Next"))}</span>
        <h3 className="w-meeting-title">{p.title}</h3>
        {p.when ? <p className="w-meeting-with">{p.when}</p> : null}
        <div className="w-meeting-people">
          {p.attendees.map((a) => (
            <WSpeakerChip key={a} name={a} />
          ))}
        </div>
        <div className="w-meeting-actions">
          {p.secondaryLabel ? (
            <WButton icon="document" link={p.links} still={p.still} onClick={p.onSecondary}>
              {p.secondaryLabel}
            </WButton>
          ) : null}
          {p.primaryLabel ? (
            <WButton variant="primary" icon="document" link={p.links} still={p.still} hint={p.primaryHint} onClick={p.onPrimary}>
              {p.primaryLabel}
            </WButton>
          ) : null}
        </div>
      </article>
    );
  }
  return (
    <article className="w-meeting" aria-label={p.title}>
      <h3 className="w-meeting-title">{p.title}</h3>
      {p.attendees.length ? (
        <span className="w-meeting-with">{t("with {names}", { names: p.attendees.join(", ") })}</span>
      ) : null}
      {p.soon ? <span className="w-meeting-tag">{p.soon}</span> : null}
      <span className="w-meeting-spacer" />
      {p.primaryLabel ? (
        <WButton variant={p.variant === "upcoming" ? "secondary" : "quiet"} link={p.links} still={p.still} onClick={p.onPrimary}>
          {p.primaryLabel}
        </WButton>
      ) : null}
      {p.variant === "past" && p.gist ? <p className="w-meeting-gist">{p.gist}</p> : null}
    </article>
  );
}

/* ---- Memory block ---- */

export interface WMemoryItem {
  app: "blue" | "green" | "purple" | "amber";
  icon: WIconName;
  name: string;
}

export function WMemoryBlock({ range, items }: { range: string; items: WMemoryItem[] }) {
  return (
    <section className="w-memory" aria-label={t("Memory, {range}", { range })}>
      <div className="w-memory-head">
        <span className="w-memory-range">{range}</span>
        <span className="w-memory-kind">{t("Memory")}</span>
      </div>
      <ul className="w-memory-items">
        {items.map((i) => (
          <li key={i.name} className="w-memory-item">
            <span className={i.app === "blue" ? "w-dot" : `w-dot w-dot-${i.app}`} aria-hidden="true" />
            <WIcon name={i.icon} size={16} />
            <span>{i.name}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/* ---- States ---- */

export interface WStateProps {
  title: string;
  help?: string;
  icon?: WIconName;
  actionLabel?: string;
  onAction?: () => void;
  /** The action stays on screen but cannot be pressed (for example while the work it starts is running). Default: enabled. */
  actionDisabled?: boolean;
  /** Which button the action is. Default: the secondary one. */
  actionKind?: "primary" | "secondary";
  /** The title is a heading of this level for screen readers. Default: a plain paragraph, as before. */
  titleLevel?: 2 | 3;
}

function State({ kind, icon, title, help, actionLabel, onAction, actionDisabled, actionKind, titleLevel }: WStateProps & { kind: "empty" | "error" }) {
  const Title = titleLevel === 2 ? "h2" : titleLevel === 3 ? "h3" : "p";
  return (
    <div className={`w-state w-state-${kind}`} role={kind === "error" ? "alert" : undefined}>
      <span className="w-state-glyph">
        <WIcon name={icon ?? (kind === "error" ? "warning" : "document")} />
      </span>
      <Title className="w-state-title">{title}</Title>
      {help ? <p className="w-state-help">{help}</p> : null}
      {actionLabel && onAction ? (
        <WButton variant={actionKind ?? "secondary"} disabled={actionDisabled} onClick={onAction}>
          {actionLabel}
        </WButton>
      ) : null}
    </div>
  );
}

export function WEmptyState(p: WStateProps) {
  return <State kind="empty" {...p} />;
}

/** Says what failed. Try again is the default action. */
export function WErrorState({ onAction, actionLabel, ...p }: WStateProps) {
  return <State kind="error" actionLabel={actionLabel ?? (onAction ? t("Try again") : undefined)} onAction={onAction} {...p} />;
}

/** A quiet bar, no spinner. Where cached content exists, show it instead and use this only for the first load. */
export function WLoadingState({ title, help }: { title: string; help?: string }) {
  return (
    <div className="w-state" role="status" aria-busy="true">
      <div className="w-quiet-bar" aria-hidden="true" />
      <p className="w-state-title">{title}</p>
      {help ? <p className="w-state-help">{help}</p> : null}
    </div>
  );
}
