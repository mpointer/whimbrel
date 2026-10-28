import type { ReactNode } from "react";
import { nonEmptySections, schedule, scheduleItems, upNext, voiceText } from "../layoutModel";
import { EventLine, RecapLines, ScheduleLines, SkinImage, SkinText, type TodayLayoutProps } from "./parts";

/** Translucent cards (a blurred backdrop under a `backgroundCard` tint) over a gradient from `accentPrimary`
 *  to `backgroundPage`. A big thin greeting, the Ask pill, then Up next, Later today and the recap as cards.
 *  Places greeting, composer, up next, schedule and recap; the rest follows as a plain list. */
export function Glass(props: TodayLayoutProps) {
  const { model, voice, heading } = props;
  const present = nonEmptySections(model);
  const next = upNext(model);
  const items = scheduleItems(schedule(model, true));

  const card = (id: string, title: string, rows: ReactNode) => (
    <section key={id} data-section={id} className="glass-card" aria-label={title}>
      <SkinText role="label" size={12} className="tone-secondary">{title}</SkinText>
      {rows}
    </section>
  );

  return (
    <div className="arch arch--glass">
      <header className="arch-header glass-header">
        <div className="arch-header__text">
          {heading.kicker && <SkinText role="label" size={13} className="tone-secondary">{heading.kicker}</SkinText>}
          <SkinText role="display" lines={3} size={44} as="h1" className="tone-primary arch-title">{heading.title}</SkinText>
        </div>
        <SkinImage imagery={props.imagery} skinId={props.skinId} placement="badge" />
      </header>
      {/* The composer's own card, at the skin's radius, is the pill. */}
      {props.composer}
      {present.includes("upNext") && next && card("upNext", voiceText(voice, "upNextHeader"), <EventLine row={next} props={props} shape="grid" />)}
      {present.includes("schedule") && items.length > 0 && card("schedule", voiceText(voice, "upcomingHeader"), <ScheduleLines items={items} props={props} shape="grid" />)}
      {present.includes("recap") && model.recap && card("recap", voiceText(voice, "recapHeader"), <RecapLines recap={model.recap} props={props} tone="primary" />)}
    </div>
  );
}
