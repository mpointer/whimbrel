import type { ReactNode } from "react";
import { earlierRows, nonEmptySections, schedule, scheduleItems, upNext, voiceText } from "../layoutModel";
import { DevicesLine, EventLine, HeldLine, OpenLoopLines, RecapLines, ScheduleLines, SkinImage, SkinText, applyCaps, type TodayLayoutProps } from "./parts";

/** A centred page inside a double-rule border: small-caps headers, a fleuron between sections, the skin's
 *  serif throughout. Places every section. */
export function Almanac(props: TodayLayoutProps) {
  const { model, voice, caps, heading } = props;
  const present = nonEmptySections(model);
  const next = upNext(model);
  const items = scheduleItems(schedule(model, true));

  /** A fleuron between two thin rules. The glyph is decoration, hidden from screen readers. */
  const divider = props.dividers ? (
    <div className="alm-divider" aria-hidden="true">
      <span className="alm-divider__rule" />
      <span className="alm-divider__glyph tone-accent">&#x2766;</span>
      <span className="alm-divider__rule" />
    </div>
  ) : null;

  const section = (id: string, title: string, rows: ReactNode) => (
    <section key={id} data-section={id} className="alm-section" aria-label={title}>
      {divider}
      <SkinText role="label" size={13} smallCaps={caps === "small"} className="tone-accent alm-title">{applyCaps(caps, title)}</SkinText>
      {rows}
    </section>
  );

  return (
    <div className="arch arch--almanac">
      <header className="arch-header alm-header">
        <div className="arch-header__text">
          {heading.kicker && <SkinText role="label" size={12} smallCaps className="tone-secondary">{heading.kicker}</SkinText>}
          <SkinText role="display" lines={2} size={26} as="h1" className="tone-primary arch-title">{heading.title}</SkinText>
        </div>
        <SkinImage imagery={props.imagery} skinId={props.skinId} placement="badge" />
      </header>
      {props.composer}
      {present.includes("upNext") && next && section("upNext", voiceText(voice, "upNextHeader"), <EventLine row={next} props={props} shape="stack" accentTime />)}
      {present.includes("schedule") && items.length > 0 && section("schedule", voiceText(voice, "upcomingHeader"), <ScheduleLines items={items} props={props} shape="stack" />)}
      {present.includes("earlier") &&
        section(
          "earlier",
          voiceText(voice, "earlierHeader"),
          earlierRows(model).map((row) => <HeldLine key={row.id} row={row} props={props} shape="stack" />),
        )}
      {present.includes("threads") && section("threads", "Open loops", <OpenLoopLines props={props} />)}
      {present.includes("recap") && model.recap && section("recap", voiceText(voice, "recapHeader"), <RecapLines recap={model.recap} props={props} tone="secondary" />)}
      <div className="alm-section">
        {divider}
        <DevicesLine model={model} align="center" />
      </div>
    </div>
  );
}
