import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WAskBar, WButton, WNextMeeting, WSectionLabel, WSpine, WSpineNow, WSpineStop } from ".";

const html = (n: React.ReactElement) => renderToStaticMarkup(n);
const next = { title: "Northwind security review", time: "10:31 AM", soon: "in 9 min", attendees: ["Sam Okafor"], primaryLabel: "Start", onPrimary: () => {} };

describe("Home parts", () => {
  it("brackets a label for the eye but not for a reader", () => {
    const out = html(<WSectionLabel variant="bracketed">Open threads</WSectionLabel>);
    expect(out).toContain('aria-hidden="true">[ </span>');
    expect(out).toContain("w-label-bracketed");
    expect(html(<WSectionLabel variant="smallCaps">x</WSectionLabel>)).toContain("w-label-smallCaps");
  });
  it("draws the next meeting three ways with the same words", () => {
    for (const variant of ["card", "scoreboard", "infobox"] as const) {
      const out = html(<WNextMeeting variant={variant} {...next} when="Today, 10:31 AM" />);
      expect(out, variant).toContain("Northwind security review");
      expect(out, variant).toContain("Start");
      expect(out, variant).toContain("Sam Okafor");
    }
    expect(html(<WNextMeeting variant="scoreboard" {...next} />)).toContain("w-next-digits");
    expect(html(<WNextMeeting variant="infobox" {...next} when="Today" />)).toContain("<dt>When</dt>");
  });
  it("draws an action as a link without changing its name", () => {
    expect(html(<WButton link>Open</WButton>)).toContain("w-button-link");
    expect(html(<WButton>Open</WButton>)).not.toContain("w-button-link");
  });
  it("names the ask field and offers one submit", () => {
    const out = html(<WAskBar variant="prompt" value="" onChange={() => {}} onSubmit={() => {}} placeholder="Ask" label="Ask Whimbrel" submitLabel="Ask" hint="Ctrl+K" />);
    expect(out).toContain('aria-label="Ask Whimbrel"');
    expect(out).toContain("w-ask-prompt");
    expect(out).toContain("Ctrl+K");
  });
  it("announces the marker once and keeps stops as list items with names", () => {
    const out = html(
      <WSpine label="Today" variant="ruled">
        <WSpineStop time="10:31 AM" node="upcoming" label="10:31 AM, Northwind security review, in 9 min">
          x
        </WSpineStop>
        <WSpineNow time="10:22 AM" />
      </WSpine>,
    );
    expect(out).toContain("<ol");
    expect(out).toContain('aria-label="10:31 AM, Northwind security review, in 9 min"');
    expect(out).toContain('aria-label="Now, 10:22 AM"');
    expect(out).toContain("w-spine-ruled");
  });
});
