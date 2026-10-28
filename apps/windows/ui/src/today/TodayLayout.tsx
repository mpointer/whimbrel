import type { ReactNode } from "react";
import { Almanac } from "./archetypes/Almanac";
import { Field } from "./archetypes/Field";
import { Glass } from "./archetypes/Glass";
import { Reference } from "./archetypes/Reference";
import { Terminal } from "./archetypes/Terminal";
import { DevicesLine, EventLine, HeldLine, OpenLoopLines, RecapLines, ScheduleLines, SkinImage, type TodayLayoutProps } from "./archetypes/parts";
import { earlierRows, leftovers, schedule, scheduleItems, upNext, voiceText, type ArchetypeId } from "./layoutModel";
import "./archetypes.css";

export type SkinnedArchetype = Exclude<ArchetypeId, "standard">;

const LAYOUTS: Record<SkinnedArchetype, (props: TodayLayoutProps) => ReactNode> = {
  terminal: Terminal,
  glass: Glass,
  field: Field,
  almanac: Almanac,
  reference: Reference,
};

/** A plain list of the sections a layout did not place, in Whimbrel's own styling. */
function Leftovers({ archetype, props }: { archetype: SkinnedArchetype; props: TodayLayoutProps }) {
  const { model, voice } = props;
  const block = (id: string, title: string, rows: ReactNode) => (
    <section key={id} data-section={id} className="arch-leftover" aria-label={title}>
      <h2 className="section-label">{title}</h2>
      {rows}
    </section>
  );
  const parts = leftovers(archetype, model).flatMap((section): ReactNode[] => {
    switch (section) {
      case "upNext": {
        const next = upNext(model);
        return next ? [block(section, voiceText(voice, "upNextHeader"), <EventLine row={next} props={props} shape="grid" />)] : [];
      }
      case "schedule":
        return [block(section, voiceText(voice, "upcomingHeader"), <ScheduleLines items={scheduleItems(schedule(model, true))} props={props} shape="grid" />)];
      case "earlier":
        return [block(section, voiceText(voice, "earlierHeader"), earlierRows(model).map((row) => <HeldLine key={row.id} row={row} props={props} shape="grid" />))];
      case "threads":
        return [block(section, "Open loops", <OpenLoopLines props={props} />)];
      case "recap":
        return model.recap ? [block(section, voiceText(voice, "recapHeader"), <RecapLines recap={model.recap} props={props} tone="secondary" />)] : [];
      case "devices":
        return [
          <div key={section} className="arch-leftover">
            <DevicesLine model={model} />
          </div>,
        ];
      default:
        return []; // the greeting and the composer are placed by every layout
    }
  });
  return parts.length > 0 ? <div className="arch-leftovers">{parts}</div> : null;
}

/**
 * Today under a skin's layout: the archetype's own arrangement, then a plain list of the sections it leaves
 * out, over the skin's background image when it has one. Standard is Windows' own Today (TodayView), so this
 * draws the other five.
 */
export function TodayLayout({ archetype, ...props }: TodayLayoutProps & { archetype: SkinnedArchetype }) {
  const Layout = LAYOUTS[archetype];
  return (
    <div className="arch-page" data-layout={archetype}>
      <SkinImage imagery={props.imagery} skinId={props.skinId} placement="background" />
      <div className="arch-body">
        <Layout {...props} />
        <Leftovers archetype={archetype} props={props} />
      </div>
    </div>
  );
}
