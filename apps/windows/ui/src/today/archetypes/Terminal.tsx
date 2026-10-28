import type { ReactNode } from "react";
import { earlierRows, nonEmptySections, schedule, scheduleItems, upNext } from "../layoutModel";
import { voiceText } from "../layoutModel";
import { DevicesLine, EventLine, HeldLine, OpenLoopLines, RecapLines, ScheduleLines, SkinImage, SkinText, applyCaps, type TodayLayoutProps } from "./parts";

/** One mono column: `[ SECTION ]` labels, dotted rules, and an Ask framed as a shell prompt (`>`, a block
 *  cursor, then the real composer). Places every section. */
export function Terminal(props: TodayLayoutProps) {
  const { model, voice, caps, heading } = props;
  const present = nonEmptySections(model);
  const next = upNext(model);
  const rest = schedule(model, true);
  const items = scheduleItems(rest);
  const rule = props.dividers ? <hr className="term-rule" aria-hidden="true" /> : null;

  const section = (id: string, title: string, rows: ReactNode) => (
    <section key={id} data-section={id} className="term-section" aria-label={title}>
      {rule}
      <SkinText role="label" size={12} className="tone-accent term-title">{`[ ${applyCaps(caps, title)} ]`}</SkinText>
      {rows}
    </section>
  );

  return (
    <div className="arch arch--terminal">
      <header className="arch-header">
        <div className="arch-header__text">
          {heading.kicker && <SkinText role="label" size={12} className="tone-secondary">{`# ${heading.kicker}`}</SkinText>}
          <SkinText role="display" lines={2} as="h1" className="tone-primary arch-title">{heading.title}</SkinText>
        </div>
        <SkinImage imagery={props.imagery} skinId={props.skinId} placement="badge" />
      </header>
      <div className="term-prompt">
        <span className="term-gt tone-accent" aria-hidden="true">&gt;</span>
        <span className={props.motion ? "term-cursor term-cursor--blink" : "term-cursor"} aria-hidden="true" />
        <div className="term-composer">{props.composer}</div>
      </div>
      {present.includes("upNext") && next && section("upNext", voiceText(voice, "upNextHeader"), <EventLine row={next} props={props} shape="grid" accentTime />)}
      {present.includes("schedule") && items.length > 0 && section("schedule", voiceText(voice, "upcomingHeader"), <ScheduleLines items={items} props={props} shape="grid" accentTime />)}
      {present.includes("earlier") &&
        section(
          "earlier",
          voiceText(voice, "earlierHeader"),
          earlierRows(model).map((row) => <HeldLine key={row.id} row={row} props={props} shape="grid" />),
        )}
      {present.includes("threads") && section("threads", "Open loops", <OpenLoopLines props={props} marker=">" />)}
      {present.includes("recap") && model.recap && section("recap", voiceText(voice, "recapHeader"), <RecapLines recap={model.recap} props={props} tone="secondary" />)}
      <div className="term-footer">
        {rule}
        <DevicesLine model={model} />
      </div>
    </div>
  );
}
