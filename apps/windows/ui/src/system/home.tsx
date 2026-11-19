// The parts of Home that a skin gives a choice of look (docs/ui-redesign/skins.md): section labels, the ask bar and
// the next meeting. Each takes the variant as a prop, draws on the skin's variables, and keeps one set of names
// for assistive technology whichever look is chosen.
import { useId, type FormEvent, type ReactNode } from "react";
import { t } from "../l10n";
import { WButton, WSpeakerChip } from "./controls";
import { WMeetingCard } from "./layout";
import { WIcon } from "./WIcon";
import "./system.css";

export type WLabelsVariant = "plain" | "bracketed" | "smallCaps";
export type WAskVariant = "field" | "prompt";
export type WNextVariant = "card" | "scoreboard" | "infobox";

/** A section label. Bracketed puts the text between square brackets and small caps sets it in small capitals; the
 *  words are the same, and a screen reader reads the text without the brackets. */
export function WSectionLabel({ variant = "plain", id, children }: { variant?: WLabelsVariant; id?: string; children: ReactNode }) {
  return (
    <h2 id={id} className={`w-label w-label-${variant}`}>
      {variant === "bracketed" ? (
        <>
          <span aria-hidden="true">[ </span>
          {children}
          <span aria-hidden="true"> ]</span>
        </>
      ) : (
        children
      )}
    </h2>
  );
}

export interface WAskBarProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  placeholder: string;
  /** The field's name for assistive technology. */
  label: string;
  /** The submit button's words ("Ask"). */
  submitLabel: string;
  /** "Ctrl+K" while the field is empty; the field takes focus from the shortcut through `inputRef`. */
  hint?: string;
  /** The model the answer would come from, small and read-only. */
  modelLabel?: string;
  inputRef?: React.Ref<HTMLInputElement>;
  /** Names the filters that will apply, read after the field's name. */
  describedBy?: string;
  /** Under the field: the filter chips. */
  below?: ReactNode;
  variant?: WAskVariant;
}

/** The search and ask bar. `field` is a boxed field with a button; `prompt` is a command line: a prompt mark, the text
 *  and a return mark, with the same button for the keyboard-less. */
export function WAskBar({ value, onChange, onSubmit, placeholder, label, submitLabel, hint, modelLabel, inputRef, describedBy, below, variant = "field" }: WAskBarProps) {
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit();
  };
  const empty = value.trim() === "";
  return (
    <div className={`w-ask w-ask-${variant}`}>
      <form className="w-ask-form" role="search" onSubmit={submit}>
        {variant === "prompt" ? (
          <span className="w-ask-mark" aria-hidden="true">
            &gt;
          </span>
        ) : (
          <WIcon name="search" />
        )}
        <input ref={inputRef} type="search" value={value} aria-label={label} aria-describedby={describedBy} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
        {empty && hint ? <span className="w-kbd">{hint}</span> : null}
        {modelLabel ? <span className="w-ask-model">{modelLabel}</span> : null}
        <WButton type="submit" variant={variant === "prompt" ? "quiet" : "primary"} disabled={empty}>
          {submitLabel}
        </WButton>
      </form>
      {below ? <div className="w-ask-below">{below}</div> : null}
    </div>
  );
}

export interface WNextMeetingProps {
  variant?: WNextVariant;
  title: string;
  /** "10:00 AM", for the scoreboard's digits. */
  time: string;
  /** "Today, 10:00 AM. Last met Tue, Nov 10." */
  when?: string;
  /** "in 8 minutes" */
  soon?: string;
  attendees: string[];
  primaryLabel?: string;
  onPrimary?: () => void;
  primaryHint?: string;
  secondaryLabel?: string;
  onSecondary?: () => void;
  links?: boolean;
  /** Drawn above the title in every look (the live recording indicator). */
  indicator?: ReactNode;
  /** The tag's words when they are not "Next, in 8 minutes" (a meeting in progress). */
  tag?: string;
}

/** The next meeting, hero of Home. `card` is the system's next-meeting card. `scoreboard` leads with the start time in
 *  large numerals on the accent color. `infobox` is a boxed list of the facts. All three carry the same words. */
export function WNextMeeting({ variant = "card", title, time, when, soon, attendees, primaryLabel, onPrimary, primaryHint, secondaryLabel, onSecondary, links, indicator, tag: tagText }: WNextMeetingProps) {
  const heading = useId();
  if (variant === "card") {
    return (
      <WMeetingCard
        variant="next"
        title={title}
        when={when}
        soon={soon}
        attendees={attendees}
        primaryLabel={primaryLabel}
        onPrimary={onPrimary}
        primaryHint={primaryHint}
        secondaryLabel={secondaryLabel}
        onSecondary={onSecondary}
        links={links}
        indicator={indicator}
        tag={tagText}
      />
    );
  }
  const tag = tagText ?? (soon ? t("Next, {when}", { when: soon }) : t("Next"));
  const actions = (
    <div className="w-meeting-actions">
      {secondaryLabel ? (
        <WButton icon="document" link={links} onClick={onSecondary}>
          {secondaryLabel}
        </WButton>
      ) : null}
      {primaryLabel ? (
        <WButton variant="primary" icon="document" link={links} hint={primaryHint} onClick={onPrimary}>
          {primaryLabel}
        </WButton>
      ) : null}
    </div>
  );
  if (variant === "scoreboard") {
    return (
      <article className="w-next w-next-scoreboard" aria-labelledby={heading}>
        <div className="w-next-score" aria-hidden="true">
          <span className="w-next-digits">{time}</span>
          <span className="w-next-soon">{tagText ?? soon ?? t("Next")}</span>
        </div>
        <div className="w-next-main">
          {indicator}
          <span className="w-visually-hidden">{tag}</span>
          <h3 id={heading} className="w-next-title">
            {title}
          </h3>
          {when ? <p className="w-meeting-with">{when}</p> : null}
          <div className="w-meeting-people">
            {attendees.map((a) => (
              <WSpeakerChip key={a} name={a} />
            ))}
          </div>
          {actions}
        </div>
      </article>
    );
  }
  return (
    <article className="w-next w-next-infobox" aria-labelledby={heading}>
      <header className="w-next-head">
        <span>{tag}</span>
        {indicator}
      </header>
      <h3 id={heading} className="w-next-title">
        {title}
      </h3>
      <dl className="w-next-facts">
        {when ? (
          <div>
            <dt>{t("When")}</dt>
            <dd>{when}</dd>
          </div>
        ) : null}
        {attendees.length ? (
          <div>
            <dt>{t("With")}</dt>
            <dd>{attendees.join(", ")}</dd>
          </div>
        ) : null}
      </dl>
      {actions}
    </article>
  );
}
