import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";

// The printed recovery page fits in every language (brief 3.4, port of the Mac's
// EncryptionPrintPageTests.testTheWholePageFitsInEveryLanguage): rendered in print media with the
// longest BIP-39 words, at A4 and at US Letter, in en, fr, de, es and pt; nothing overflows the page
// and the notice and footer are fully visible. The Windows catalog has no translations for these keys
// yet, so the French, German, Spanish and Portuguese text comes from the Mac catalog (same English
// keys; the Mac footer, which says "Mac", stands in for the Windows footer's length).

const ROOT = fileURLToPath(new URL("../../../../", import.meta.url));
const MAC = JSON.parse(readFileSync(`${ROOT}apps/macos/Whimbrel/Localizable.xcstrings`, "utf8")) as {
  strings: Record<string, { localizations?: Record<string, { stringUnit?: { value: string } }> }>;
};

const TITLE = "Whimbrel recovery words";
const NOTICE =
  "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.";
const FOOTER_WINDOWS = "Keep this page somewhere safe and offline. Anyone who has it can read the memory database of the PC it was made on.";
const FOOTER_MAC = "Keep this page somewhere safe and offline. Anyone who has it can read the memory database of the Mac it was made on.";

/** The 24 longest BIP-39 English words (8 letters). */
const WORDS =
  "universe umbrella transfer tortoise tomorrow together surround surprise struggle strategy squirrel solution shoulder sentence security scorpion scissors response resource resemble remember question purchase property".split(
    " ",
  );

function mac(key: string, lang: string): string {
  const v = MAC.strings[key]?.localizations?.[lang]?.stringUnit?.value;
  if (!v) throw new Error(`the Mac catalog has no ${lang} for ${key}`);
  return v;
}

const LANGS = ["en", "fr", "de", "es", "pt"] as const;
function texts(lang: (typeof LANGS)[number]) {
  if (lang === "en") return { title: TITLE, notice: NOTICE, footer: FOOTER_WINDOWS };
  return { title: mac(TITLE, lang), notice: mac(NOTICE, lang), footer: mac(FOOTER_MAC, lang) };
}

/** The page's printable area at 96 px per inch, inside the 12 mm @page margin (app.css). */
const MARGIN_PX = (12 / 25.4) * 96;
const PAPERS = {
  A4: { width: (210 / 25.4) * 96 - 2 * MARGIN_PX, height: (297 / 25.4) * 96 - 2 * MARGIN_PX, pdf: "A4" },
  Letter: { width: 8.5 * 96 - 2 * MARGIN_PX, height: 11 * 96 - 2 * MARGIN_PX, pdf: "Letter" },
} as const;

/** The launch is in the onboarding; Create answers with the longest words. */
async function openSetup(page: Page) {
  await page.addInitScript(
    ({ words }) => {
      const answers: Record<string, unknown> = {
        launch_state: { stage: { kind: "onboarding", reusesKey: false }, busy: false, waitingForUser: true, generation: 1 },
        setup_begin: { kind: "made", words },
        language_settings: null,
      };
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
        transformCallback: () => 0,
        invoke: async (cmd: string) => (cmd in answers ? answers[cmd] : null),
        convertFileSrc: (s: string) => s,
      };
    },
    { words: WORDS },
  );
  await page.goto("/");
  const fields = page.locator('input[type="password"]');
  await fields.nth(0).fill("a long enough passphrase");
  await fields.nth(1).fill("a long enough passphrase");
  await page.getByRole("button", { name: "Create recovery key" }).click();
  await expect(page.locator(".print-page")).toHaveCount(1);
}

interface Fit {
  pageHeight: number;
  overflowX: string[];
  overflowY: string[];
  noticeVisible: boolean;
  footerVisible: boolean;
}

/** Runs in the page, in print media: what sticks out of the printable area. */
function measure(): Fit {
  const page = document.querySelector(".print-page") as HTMLElement;
  const w = window.innerWidth;
  const h = window.innerHeight;
  const name = (el: Element) => `${el.tagName.toLowerCase()}.${el.getAttribute("class") ?? ""} "${(el.textContent ?? "").trim().slice(0, 30)}"`;
  const overflowX: string[] = [];
  const overflowY: string[] = [];
  for (const el of [page, ...page.querySelectorAll("*")]) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    if (r.left < -0.5 || r.right > w + 0.5) overflowX.push(name(el));
    if (r.bottom > h + 0.5) overflowY.push(name(el));
    if (el instanceof HTMLElement && (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) && getComputedStyle(el).overflow !== "visible") {
      overflowX.push(`clipped ${name(el)}`);
    }
  }
  const visible = (sel: string) => {
    const el = page.querySelector(sel) as HTMLElement;
    const r = el.getBoundingClientRect();
    return r.height > 0 && r.top >= 0 && r.bottom <= h + 0.5 && getComputedStyle(el).visibility !== "hidden";
  };
  return { pageHeight: page.getBoundingClientRect().bottom, overflowX, overflowY, noticeVisible: visible(".print-page-notice"), footerVisible: visible(".print-page-footer") };
}

async function setTexts(page: Page, t: { title: string; notice: string; footer: string }) {
  await page.evaluate((t) => {
    (document.querySelector(".print-page h1") as HTMLElement).textContent = t.title;
    (document.querySelector(".print-page-notice") as HTMLElement).textContent = t.notice;
    (document.querySelector(".print-page-footer") as HTMLElement).textContent = t.footer;
  }, t);
}

/** Pages in a PDF Chromium printed. */
const pdfPages = (pdf: Buffer) => (pdf.toString("latin1").match(/\/Type\s*\/Page[^s]/g) ?? []).length;

test.describe("the printed recovery page", () => {
  for (const [paper, size] of Object.entries(PAPERS)) {
    for (const lang of LANGS) {
      test(`fits on one ${paper} page in ${lang}`, async ({ page }) => {
        await page.setViewportSize({ width: Math.floor(size.width), height: Math.floor(size.height) });
        await openSetup(page);
        await page.emulateMedia({ media: "print" });
        await setTexts(page, texts(lang));
        const fit = await page.evaluate(measure);
        expect(fit.overflowX, "nothing sticks out sideways or is clipped").toEqual([]);
        expect(fit.overflowY, "nothing runs off the bottom of the page").toEqual([]);
        expect(fit.noticeVisible && fit.footerVisible, "the notice and the footer are whole").toBe(true);
        // And as Chromium really prints it: one page.
        const pdf = await page.pdf({ format: size.pdf, printBackground: true, preferCSSPageSize: false, margin: { top: "12mm", bottom: "12mm", left: "12mm", right: "12mm" } });
        expect(pdfPages(pdf)).toBe(1);
        test.info().annotations.push({ type: "height", description: `${Math.round(fit.pageHeight)} of ${Math.floor(size.height)} px` });
      });
    }
  }

  test("the check bites: the page twice over does not fit, by either measure", async ({ page }) => {
    // A doubled notice alone still fits here (the page uses about 576 of 965 px on Letter, where the
    // Mac's used 545 of 648 pt), so the whole page is doubled: both checks must fail.
    const size = PAPERS.Letter;
    await page.setViewportSize({ width: Math.floor(size.width), height: Math.floor(size.height) });
    await openSetup(page);
    await page.emulateMedia({ media: "print" });
    const longest = LANGS.map(texts).sort((a, b) => b.notice.length - a.notice.length)[0];
    await setTexts(page, longest);
    await page.evaluate(() => {
      const p = document.querySelector(".print-page") as HTMLElement;
      p.append(...[...p.children].map((c) => c.cloneNode(true)));
    });
    const over = await page.evaluate(measure);
    expect(over.overflowY.length).toBeGreaterThan(0);
    const pdf = await page.pdf({ format: size.pdf, printBackground: true, margin: { top: "12mm", bottom: "12mm", left: "12mm", right: "12mm" } });
    expect(pdfPages(pdf)).toBeGreaterThan(1);
  });
});
