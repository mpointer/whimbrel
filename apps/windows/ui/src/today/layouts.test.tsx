import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { MeetingRow, ScheduleEvent } from "../generated/types";
import { WHIMBREL } from "../skin";
import type { TodayLayoutProps } from "./archetypes/parts";
import type { TodayModel } from "./layoutModel";
import { TodayLayout, type SkinnedArchetype } from "./TodayLayout";

const MIN = 60_000;
const NOW = new Date(2026, 9, 22, 9, 0, 0).getTime();
const dev = { deviceId: "PC", label: "This PC", thisDevice: true };

const event = (id: string, minutes: number, over: Partial<ScheduleEvent> = {}): ScheduleEvent => ({
  eventId: id,
  title: `Event ${id}`,
  startUnixMs: NOW + minutes * MIN,
  endUnixMs: NOW + (minutes + 30) * MIN,
  attendees: ["ann@acme.test"],
  recordedMeetingId: null,
  ...over,
});
const meeting = (id: number, minutes: number, title: string): MeetingRow => ({ device: dev, hasSummary: true, id, status: "done", title, trigger: "calendar", tsUnixMs: NOW + minutes * MIN });

const rich: TodayModel = {
  now: NOW,
  name: "Mike",
  schedule: { events: [event("a", 10), event("b", 40), event("c", 70, { recordedMeetingId: 5 })], earlierToday: [meeting(1, -60, "Held today")], recording: false },
  next: null,
  earlier: [meeting(2, -26 * 60, "Held yesterday")],
  openLoops: [{ id: 1, subject: "Loop subject", status: "waiting_on_me", lastMessageFrom: "x", lastActivityTsUnixMs: NOW }],
  recap: { kind: "digest", text: "Recap text" },
  devices: [{ device: dev, captures: 2, lastTsUnixMs: NOW }],
};

const empty: TodayModel = { ...rich, schedule: { events: [], earlierToday: [], recording: false }, earlier: [], openLoops: [], recap: null, devices: [] };

function props(model: TodayModel, over: Partial<TodayLayoutProps> = {}): TodayLayoutProps {
  return {
    model,
    voice: WHIMBREL.skin.voice,
    imagery: [],
    skinId: "whimbrel",
    caps: "none",
    composer: <div>COMPOSER</div>,
    actions: { start: () => {}, openMeeting: () => {}, retryRecap: () => {}, toggleBrief: () => {} },
    starting: null,
    brief: { open: false, card: <div>BRIEF</div> },
    stripes: false,
    dividers: true,
    motion: false,
    heading: { kicker: "Thursday, October 22", title: "Good morning, Mike" },
    ...over,
  };
}

const render = (archetype: SkinnedArchetype, model: TodayModel, over?: Partial<TodayLayoutProps>) => renderToStaticMarkup(<TodayLayout archetype={archetype} {...props(model, over)} />);

/** The positions of these texts in the markup, which must be found and rise in this order. */
function inOrder(html: string, texts: string[]): void {
  let from = 0;
  for (const text of texts) {
    const at = html.indexOf(text, from);
    expect(at, `${text} after position ${from}`).toBeGreaterThanOrEqual(0);
    from = at + text.length;
  }
}

const ALL: SkinnedArchetype[] = ["terminal", "glass", "field", "almanac", "reference"];

describe("the skinned layouts draw", () => {
  it("a full day without failing, with the greeting, the composer and the rows", () => {
    for (const a of ALL) {
      const html = render(a, rich);
      expect(html, a).toContain("Good morning, Mike");
      expect(html, a).toContain("COMPOSER");
      expect(html, a).toContain("Event a");
      expect(html, a).toContain("Recap text");
    }
  });

  it("an empty day without failing, still with the greeting and the devices footer", () => {
    for (const a of ALL) {
      const html = render(a, empty);
      expect(html, a).toContain("Good morning, Mike");
      expect(html, a).toContain("Nothing yet today");
      expect(html, a).not.toContain("Event a");
    }
  });

  it("Terminal, Almanac and Reference place the sections in the Mac's order", () => {
    inOrder(render("terminal", rich), ["Good morning", "COMPOSER", "Up next", "Event a", "Event b", "Held today", "Earlier", "Held yesterday", "Open loops", "Loop subject", "Daily recap", "Recap text", "captures"]);
    inOrder(render("almanac", rich), ["Good morning", "COMPOSER", "Up next", "Event a", "Event b", "Held today", "Earlier", "Held yesterday", "Open loops", "Daily recap", "Recap text", "captures"]);
    // Reference: title, then the infobox and Contents, then the composer, then the body sections.
    inOrder(render("reference", rich), ["Good morning", "Contents", "COMPOSER", "Event b", "Held today", "Held yesterday", "Loop subject", "Recap text", "captures"]);
  });

  it("Glass places greeting, composer, up next, schedule and recap, then lists the rest", () => {
    inOrder(render("glass", rich), ["Good morning", "COMPOSER", "Up next", "Event a", "Event b", "Daily recap", "Recap text", "Earlier", "Held yesterday", "Open loops", "captures"]);
  });

  it("Field puts up next on a scoreboard, then the pitch and Earlier, then lists the rest", () => {
    const html = render("field", rich);
    inOrder(html, ["Good morning", "COMPOSER", "Up next", "Event a", "Earlier", "Held yesterday", "Open loops", "Recap text", "captures"]);
    expect(html).toContain("field-scoreboard");
  });

  it("never offers Start for a recorded event, and offers Open instead", () => {
    const html = render("terminal", rich);
    expect(html.match(/>Start</g)?.length).toBe(2); // up next and b; c has a recording
    expect(html).toContain(">Open<");
  });

  it("turns Start off while another is starting", () => {
    const html = render("terminal", rich, { starting: "b" });
    expect(html).toContain("Starting…");
    expect(html.match(/disabled/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it("shows only the brief for up next when it is open", () => {
    expect(render("terminal", rich)).not.toContain("BRIEF");
    const withPrep = { ...rich, next: { eventId: "a", title: "Event a", startUnixMs: NOW, brief: { summary: "s" } } as TodayModel["next"] };
    expect(render("terminal", withPrep, { brief: { open: true, card: <div>BRIEF</div> } })).toContain("BRIEF");
    expect(render("terminal", withPrep)).toContain("Open brief");
  });

  it("draws dividers only when the skin asks", () => {
    expect(render("terminal", rich, { dividers: true })).toContain("term-rule");
    expect(render("terminal", rich, { dividers: false })).not.toContain("term-rule");
    expect(render("almanac", rich, { dividers: false })).not.toContain("alm-divider");
    expect(render("field", rich, { dividers: false })).not.toContain("field-chalk");
  });

  it("brackets the actions in Reference", () => {
    const html = render("reference", rich);
    expect(html).toContain("bracket-link");
    expect(html).toContain('aria-label="Start"');
  });

  it("shows a failed recap with Try again, off while writing", () => {
    const failed: TodayModel = { ...rich, recap: { kind: "failed", message: "No provider", retryEnabled: false } };
    for (const a of ALL) {
      const html = render(a, failed);
      expect(html, a).toContain("No provider");
      expect(html, a).toContain("Try again");
      expect(html, a).toContain("disabled");
    }
  });

  it("adds a stripes band under the Field greeting only when the skin has one", () => {
    expect(render("field", rich, { stripes: true })).toContain("stripes-band");
    expect(render("field", rich, { stripes: false })).not.toContain("stripes-band");
  });

  it("makes the date the title when there is no greeting", () => {
    const html = render("terminal", rich, { heading: { kicker: null, title: "Thursday, October 22" } });
    expect(html).not.toContain("# Thursday");
    expect(html).toContain("Thursday, October 22");
  });
});
