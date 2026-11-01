import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";
import { composite, parseColor, ratio, type Rgb } from "./contrast";
import { ARCHETYPES, BUILT_IN, catalogText, installBridge, worstCase, type SkinDto } from "./fakeBridge";

// No skin may make Today overrun, overlap or become unreadable. Every installed skin × palette × light/dark,
// and the spec's worst-case skin on every layout with and without imagery, at two window widths. A failure is
// a layout bug to fix in src/today, not a check to relax (plan Task 9).
//
// No language may either (localization plan W6, W8). English and German, which runs longest, run every case.
// French (France and Canada), Spanish and Portuguese run the worst-case skin at the narrow width, where long
// words fail first. The tags are resolved
// ones, as `language_settings` gives them. Overruns are looked for in the tab labels, the header and the
// sidebar's model download line as well.

const WIDTHS = [900, 1400];
const HEIGHT = 2400; // tall enough that Today never scrolls inside the window
const MIN_CONTRAST = 4.5;
const FULL_LOCALES = ["en-US", "de-DE"];
const SUBSET_LOCALES = ["fr-FR", "fr-CA", "es-ES", "pt-BR"];

interface TextBox {
  text: string;
  color: string;
  opacity: number;
  x: number;
  y: number;
  w: number;
  h: number;
}
interface Measured {
  overflows: string[];
  escapes: string[];
  intersections: string[];
  sections: string[];
  pageScrollsSideways: boolean;
  boxes: TextBox[];
}

/** Runs in the page: what overruns, what overlaps, which sections are there, and where every piece of text is.
 *  Overruns are looked for in the whole window (the tab labels and the header too); the rest only in the content. */
function measure(): Measured {
  const content = document.querySelector("main") as HTMLElement;
  const name = (el: Element) => `${el.tagName.toLowerCase()}.${String(el.getAttribute("class") ?? "").split(" ")[0]} "${(el.textContent ?? "").trim().slice(0, 40)}"`;
  const hasOwnText = (el: Element) => [...el.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? "").trim() !== "");
  const hidden = (el: Element) => {
    for (let e: Element | null = el; e; e = e.parentElement) {
      const s = getComputedStyle(e);
      if (s.display === "none" || s.visibility === "hidden" || Number(s.opacity) === 0 || e.getAttribute("aria-hidden") === "true") return true;
    }
    return false;
  };
  // Text a skin deliberately shortens with an ellipsis is clipped by design; its box still has to fit (checked below).
  const clamps = (el: Element) => {
    const s = getComputedStyle(el);
    return s.textOverflow === "ellipsis" || (s as unknown as Record<string, string>).webkitLineClamp !== "none";
  };
  const scrolls = (el: Element) => /(auto|scroll)/.test(getComputedStyle(el).overflow + getComputedStyle(el).overflowX + getComputedStyle(el).overflowY);

  const clipped = (rect: DOMRect, from: Element): DOMRect | null => {
    let [left, top, right, bottom] = [rect.left, rect.top, rect.right, rect.bottom];
    for (let e: Element | null = from; e; e = e.parentElement) {
      const s = getComputedStyle(e);
      if (e !== from && s.overflowX === "visible" && s.overflowY === "visible") continue;
      const b = e.getBoundingClientRect();
      if (e === from && !clamps(e)) continue;
      [left, top, right, bottom] = [Math.max(left, b.left), Math.max(top, b.top), Math.min(right, b.right), Math.min(bottom, b.bottom)];
    }
    return right > left && bottom > top ? new DOMRect(left, top, right - left, bottom - top) : null;
  };

  const overflows: string[] = [];
  const escapes: string[] = [];
  const boxes: TextBox[] = [];
  const sectionEls = [...content.querySelectorAll("[data-section]")];
  for (const el of document.body.querySelectorAll("*")) {
    if (!hasOwnText(el) || hidden(el)) continue;
    const display = getComputedStyle(el).display;
    if (display !== "inline" && !clamps(el) && !scrolls(el)) {
      if (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) overflows.push(`${name(el)} scroll ${el.scrollWidth}x${el.scrollHeight} in ${el.clientWidth}x${el.clientHeight}`);
    }
    if (!content.contains(el)) continue;
    const section = el.closest("[data-section]");
    let opacity = 1;
    for (let e: Element | null = el; e; e = e.parentElement) opacity *= Number(getComputedStyle(e).opacity);
    for (const node of el.childNodes) {
      if (node.nodeType !== Node.TEXT_NODE || (node.textContent ?? "").trim() === "") continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const rect of range.getClientRects()) {
        // Only what is visible: text an ellipsis or an overflow-hidden box cuts off is not on the page.
        const r = clipped(rect, el);
        if (!r || r.width < 1 || r.height < 1) continue;
        if (section && !clamps(el)) {
          const s = section.getBoundingClientRect();
          if (r.left < s.left - 1 || r.right > s.right + 1 || r.top < s.top - 1 || r.bottom > s.bottom + 1) escapes.push(`${name(el)} leaves its ${section.getAttribute("data-section")} section`);
        }
        const interactive = el.closest("button, input, textarea, select");
        if ((interactive as HTMLButtonElement | null)?.disabled) continue;
        // A glyph from an icon font is not text (it needs 3:1, not 4.5:1).
        if (getComputedStyle(el).fontFamily.includes("Segoe Fluent Icons")) continue;
        boxes.push({ text: `${el.tagName.toLowerCase()}.${String(el.getAttribute("class") ?? "").split(" ")[0]} in ${section?.getAttribute("data-section") ?? "-"} "${(node.textContent ?? "").trim().slice(0, 24)}"`, color: getComputedStyle(el).color, opacity, x: r.left + scrollX, y: r.top + scrollY, w: r.width, h: r.height });
      }
    }
  }

  const intersections: string[] = [];
  for (let i = 0; i < sectionEls.length; i++)
    for (let j = i + 1; j < sectionEls.length; j++) {
      const [a, b] = [sectionEls[i], sectionEls[j]];
      if (a.contains(b) || b.contains(a)) continue;
      const [ra, rb] = [a.getBoundingClientRect(), b.getBoundingClientRect()];
      const w = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
      const h = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
      if (w > 1 && h > 1) intersections.push(`${a.getAttribute("data-section")} and ${b.getAttribute("data-section")} overlap by ${Math.round(w)}x${Math.round(h)}`);
    }

  return {
    overflows,
    escapes,
    intersections,
    sections: sectionEls.map((e) => e.getAttribute("data-section") ?? ""),
    pageScrollsSideways: document.documentElement.scrollWidth > innerWidth + 1,
    boxes,
  };
}

/** Every text box's contrast against what is really behind it. The page is drawn once with its text made
 *  transparent, so gradients, glows and images count, and each box is judged against its worst pixel. */
async function contrastFailures(page: Page, boxes: TextBox[]): Promise<string[]> {
  await page.addStyleTag({ content: "*, *::before, *::after { color: transparent !important; -webkit-text-fill-color: transparent !important; text-shadow: none !important; caret-color: transparent !important; } svg text { fill: transparent !important; }" });
  const png = PNG.sync.read(await page.screenshot({ fullPage: true }));
  const failures: string[] = [];
  for (const box of boxes) {
    const fg = parseColor(box.color);
    if (!fg) continue;
    const x0 = Math.max(0, Math.floor(box.x));
    const y0 = Math.max(0, Math.floor(box.y));
    const x1 = Math.min(png.width, Math.ceil(box.x + box.w));
    const y1 = Math.min(png.height, Math.ceil(box.y + box.h));
    const step = Math.max(1, Math.floor(Math.sqrt(((x1 - x0) * (y1 - y0)) / 400)));
    let worst = Infinity;
    for (let y = y0; y < y1; y += step)
      for (let x = x0; x < x1; x += step) {
        const i = (y * png.width + x) * 4;
        const behind: Rgb = [png.data[i], png.data[i + 1], png.data[i + 2]];
        worst = Math.min(worst, ratio(composite(fg.rgb, fg.alpha * box.opacity, behind), behind));
      }
    if (worst < MIN_CONTRAST) failures.push(`${box.text} ${worst.toFixed(2)}:1`);
  }
  return failures;
}

interface Case {
  title: string;
  skins: SkinDto[];
  skinId: string;
  paletteId: string;
  mode: "light" | "dark";
  /** The layout the skin should draw (so the sections can be checked against it). */
  archetype: string;
  /** The language the shell reports, a resolved tag. */
  lang: string;
  widths: number[];
}

/** The modes a palette really has: one that ships a single variant draws it whatever the appearance. */
function modesOf(palette: { light?: unknown; dark?: unknown }): ("light" | "dark")[] {
  return [palette.light && "light", palette.dark && "dark"].filter(Boolean) as ("light" | "dark")[];
}

const cases: Case[] = [];
for (const lang of FULL_LOCALES)
  for (const dto of BUILT_IN)
    for (const palette of dto.palettes)
      for (const mode of modesOf(palette))
        cases.push({ title: `${lang} / ${dto.skin.id} / ${palette.id} / ${mode}`, skins: BUILT_IN, skinId: dto.skin.id, paletteId: palette.id, mode, archetype: dto.skin.today.archetype, lang, widths: WIDTHS });
for (const lang of [...FULL_LOCALES, ...SUBSET_LOCALES])
  for (const archetype of ARCHETYPES)
    for (const imagery of [false, true])
      for (const mode of ["light", "dark"] as const) {
        const dto = worstCase(archetype, imagery);
        if (!modesOf(dto.palettes[0]).includes(mode)) continue;
        const widths = FULL_LOCALES.includes(lang) ? WIDTHS : [WIDTHS[0]];
        cases.push({ title: `${lang} / worst case / ${archetype} / ${imagery ? "with" : "no"} imagery / ${mode}`, skins: [...BUILT_IN, dto], skinId: dto.skin.id, paletteId: dto.palettes[0].id, mode, archetype, lang, widths });
      }

/** What the fixture day gives a layout to place, each once, wherever the layout puts it. Reference also lists the
 *  devices line among its Contents; the others draw it as an unlabelled footer. */
const expectedSections = (archetype: string) => (archetype === "reference" ? ["devices"] : []).concat(["earlier", "recap", "schedule", "threads", "upNext"]).sort();

for (const width of WIDTHS)
  for (const c of cases.filter((each) => each.widths.includes(width)))
    test(`${c.title} @${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: HEIGHT });
      await installBridge(page, c);
      await page.goto("/");
      await page.locator(".today").first().waitFor();
      await page.locator("[data-skin]").first().waitFor();
      await expect(page.locator("html")).toHaveAttribute("data-archetype", c.archetype);
      // The page is really in the language asked for, or a pass would say nothing about it.
      const settingsLabel = catalogText("Settings", c.lang);
      if (!c.lang.startsWith("en")) expect(settingsLabel, "the catalog translates the Settings tab").not.toBe("Settings");
      await expect(page.locator("#tab-settings"), "the tabs are in the language asked for").toContainText(settingsLabel);
      // The fixture is a first launch, so the sidebar's model download line is drawn and checked too.
      await expect(page.locator(".sidebar-footer"), "the model download line is drawn").toBeVisible();
      await page.waitForTimeout(400); // images decoded, layout settled

      const m = await page.evaluate(measure);
      expect(m.boxes.length, "found the text on the page").toBeGreaterThan(10);
      expect(m.pageScrollsSideways, "the page scrolls sideways").toBe(false);
      expect(m.overflows, "text overruns its box").toEqual([]);
      expect(m.escapes, "text leaves its section").toEqual([]);
      expect(m.intersections, "sections overlap").toEqual([]);
      if (c.archetype !== "standard") expect([...new Set(m.sections)].sort(), "each section is drawn once").toEqual(expectedSections(c.archetype));
      expect(m.sections.length, "no section is drawn twice").toBe(new Set(m.sections).size);
      expect(await contrastFailures(page, m.boxes), `text under ${MIN_CONTRAST}:1`).toEqual([]);
    });
