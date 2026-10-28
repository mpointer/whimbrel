import type { CSSProperties, ReactNode } from "react";
import {
  REFERENCE_CONTENTS,
  REFERENCE_INFOBOX_WIDTH,
  earlierRows,
  infoboxPlacement,
  nonEmptySections,
  schedule,
  scheduleItems,
  upNext,
  voiceText,
  type EventRow,
  type SectionId,
} from "../layoutModel";
import { BracketLink, DevicesLine, HeldLine, OpenLoopLines, PrepLine, RecapLines, ScheduleLines, SkinImage, SkinText, useWidth, type TodayLayoutProps } from "./parts";

const anchor = (section: SectionId) => `today-${section}`;

/** An encyclopedia article: a serif title over a rule, a Contents box, an infobox for up next (floated right at
 *  wide content widths, stacked above otherwise), ruled sections, and actions as bracketed links. Places every
 *  section. */
export function Reference(props: TodayLayoutProps) {
  const { model, voice, heading } = props;
  const present = nonEmptySections(model);
  const next = upNext(model);
  const rest = schedule(model, true);
  const items = scheduleItems(rest);
  const [ref, width] = useWidth<HTMLDivElement>();

  /** A section that is drawn, in page order, with its Contents entry. */
  const entries: { section: SectionId; title: string }[] = REFERENCE_CONTENTS.flatMap((section): { section: SectionId; title: string }[] => {
    if (!present.includes(section)) return [];
    switch (section) {
      case "composer":
        return [{ section, title: "Ask" }];
      case "upNext":
        return next ? [{ section, title: voiceText(voice, "upNextHeader") }] : [];
      case "schedule":
        return items.length > 0 ? [{ section, title: voiceText(voice, "upcomingHeader") }] : [];
      case "earlier":
        return [{ section, title: voiceText(voice, "earlierHeader") }];
      case "threads":
        return [{ section, title: "Open loops" }];
      case "recap":
        return model.recap ? [{ section, title: voiceText(voice, "recapHeader") }] : [];
      case "devices":
        return [{ section, title: "Devices" }];
      default:
        return [];
    }
  });

  const rule = <div className="ref-rule" aria-hidden="true" />;
  const section = (which: SectionId, title: string, rows: ReactNode) => (
    <section key={which} data-section={which} id={anchor(which)} className="ref-section" aria-label={title}>
      <div className="ref-section__head">
        <SkinText role="display" lines={2} size={21} as="h2" className="tone-primary">{title}</SkinText>
        {rule}
      </div>
      {rows}
    </section>
  );

  const contents = (
    <nav className="ref-contents" aria-label="Contents">
      <SkinText role="label" size={13} className="tone-primary">Contents</SkinText>
      {entries.map((entry, index) => (
        <button key={entry.section} type="button" className="ref-contents__entry" title={`Go to ${entry.title}`} onClick={() => document.getElementById(anchor(entry.section))?.scrollIntoView({ block: "start" })}>
          <span className="tone-secondary" aria-hidden="true">{index + 1}</span>
          <SkinText size={13} className="tone-link">{entry.title}</SkinText>
        </button>
      ))}
    </nav>
  );

  return (
    <div className="arch arch--reference">
      <header className="arch-header ref-title">
        <div className="arch-header__text">
          <SkinText role="display" lines={2} size={30} as="h1" className="tone-primary arch-title">{heading.title}</SkinText>
          {heading.kicker && <SkinText role="label" size={12} className="tone-secondary">{heading.kicker}</SkinText>}
          {rule}
        </div>
        <SkinImage imagery={props.imagery} skinId={props.skinId} placement="badge" />
      </header>
      <div ref={ref} className="ref-intro" data-placement={infoboxPlacement(width)} style={{ "--infobox-width": `${REFERENCE_INFOBOX_WIDTH}px` } as CSSProperties}>
        {next ? (
          <>
            <Infobox row={next} props={props} title={voiceText(voice, "upNextHeader")} />
            {contents}
          </>
        ) : (
          entries.length > 0 && contents
        )}
      </div>
      {present.includes("composer") && <div id={anchor("composer")}>{props.composer}</div>}
      {present.includes("schedule") && items.length > 0 && section("schedule", voiceText(voice, "upcomingHeader"), <ScheduleLines items={items} props={props} shape="inline" bracket />)}
      {present.includes("earlier") &&
        section(
          "earlier",
          voiceText(voice, "earlierHeader"),
          earlierRows(model).map((row) => <HeldLine key={row.id} row={row} props={props} shape="inline" bracket />),
        )}
      {present.includes("threads") && section("threads", "Open loops", <OpenLoopLines props={props} linkTone />)}
      {present.includes("recap") && model.recap && section("recap", voiceText(voice, "recapHeader"), <RecapLines recap={model.recap} props={props} tone="primary" bracket />)}
      {present.includes("devices") && section("devices", "Devices", <DevicesLine model={model} />)}
    </div>
  );
}

function Infobox({ row, props, title }: { row: EventRow; props: TodayLayoutProps; title: string }) {
  return (
    <aside data-section="upNext" id={anchor("upNext")} className="ref-infobox" aria-label={title}>
      <SkinText role="label" size={13} className="ref-infobox__title">{title}</SkinText>
      <div className="ref-infobox__body">
        <SkinText role="display" lines={3} size={17} className="tone-primary">{row.title}</SkinText>
        <SkinText role="numerals" className="tone-primary">{row.timeText}</SkinText>
        {row.relative && <SkinText role="label" className="tone-secondary">{row.relative}</SkinText>}
        {row.attendees && <SkinText role="label" lines={2} className="tone-secondary">{row.attendees}</SkinText>}
        <PrepLine prep={row.prep} props={props} bracket />
        {row.action === "start" && <BracketLink label={props.starting === row.id ? "Starting…" : "Start"} primary disabled={props.starting !== null} onClick={() => props.actions.start(row.id)} />}
      </div>
    </aside>
  );
}
