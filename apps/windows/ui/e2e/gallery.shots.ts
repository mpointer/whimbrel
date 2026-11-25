// Full-page PNGs of gallery.html: five modes at 100% text, and light at 200%.
// Output: docs/ui-redesign/screenshots/system/windows/gallery-<mode>-<size>.png
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const OUT = fileURLToPath(new URL("../../../../docs/ui-redesign/screenshots/system/windows/", import.meta.url));
mkdirSync(OUT, { recursive: true });

const SHOTS: { mode: string; size: number; scheme: "light" | "dark"; extra?: { forcedColors?: "active" } }[] = [
  { mode: "system", size: 100, scheme: "light" },
  { mode: "light", size: 100, scheme: "light" },
  { mode: "dark", size: 100, scheme: "dark" },
  { mode: "hc-light", size: 100, scheme: "light" },
  // Light scheme on purpose: with the OS in dark, the generated dark block outranks .w-thread.w-mode-hc-dark (see the report).
  { mode: "hc-dark", size: 100, scheme: "light" },
  { mode: "light", size: 200, scheme: "light" },
];

for (const s of SHOTS) {
  test(`gallery ${s.mode} ${s.size}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: s.scheme, ...s.extra });
    page.on("pageerror", (e) => console.log(`[pageerror] ${e.message}`));
    await page.goto(`/gallery.html?mode=${s.mode}&size=${s.size}`);
    await page.waitForSelector(".g-root");
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${OUT}gallery-${s.mode}-${s.size}.png`, fullPage: true });
  });
}

// Settings parts: each group on its own, in light, dark, forced colors, and at 200% text in a narrow window.
// Output: docs/ui-redesign/screenshots/after/windows-browser/gallery/gallery-settings-<group>-<variant>.png
// Run:    GALLERY_PORT=4198 npx playwright test -c e2e/gallery.config.ts --grep settings
const SETTINGS_OUT = fileURLToPath(new URL("../../../../docs/ui-redesign/screenshots/after/windows-browser/gallery/", import.meta.url));
mkdirSync(SETTINGS_OUT, { recursive: true });

const SETTINGS_VARIANTS: { name: string; mode: string; size: number; scheme: "light" | "dark"; width: number; forced?: boolean }[] = [
  { name: "light", mode: "light", size: 100, scheme: "light", width: 1100 },
  { name: "dark", mode: "dark", size: 100, scheme: "dark", width: 1100 },
  { name: "forced", mode: "light", size: 100, scheme: "light", width: 1100, forced: true },
  { name: "200-narrow", mode: "light", size: 200, scheme: "light", width: 480 },
];

for (const v of SETTINGS_VARIANTS) {
  test(`gallery settings ${v.name}`, async ({ page }) => {
    await page.setViewportSize({ width: v.width, height: 900 });
    await page.emulateMedia({ colorScheme: v.scheme, ...(v.forced ? { forcedColors: "active" as const } : {}) });
    page.on("pageerror", (e) => console.log(`[pageerror] ${e.message}`));
    await page.goto(`/gallery.html?mode=${v.mode}&size=${v.size}&only=settings`);
    await page.waitForSelector("[data-shot]");
    await page.evaluate(() => document.fonts.ready);
    await page.addStyleTag({ content: ".g-head{position:static !important}" });
    const groups = await page.locator("[data-shot]").evaluateAll((els) => els.map((e) => e.getAttribute("data-shot") as string));
    for (const g of groups) {
      await page.locator(`[data-shot="${g}"]`).screenshot({ path: `${SETTINGS_OUT}gallery-settings-${g}-${v.name}.png` });
    }
    if (v.name === "200-narrow") {
      // No sideways scrolling anywhere: not on the page, and not inside any case.
      const wide = await page.evaluate(() => {
        const out: string[] = [];
        if (document.documentElement.scrollWidth > window.innerWidth) out.push(`page ${document.documentElement.scrollWidth} > ${window.innerWidth}`);
        document.querySelectorAll("[data-case]").forEach((el) => {
          const label = el.querySelector(".g-label")?.textContent ?? "case";
          if (el.scrollWidth > el.clientWidth + 1) out.push(`${label}: ${el.scrollWidth} > ${el.clientWidth}`);
        });
        document.querySelectorAll("body *").forEach((el) => {
          const r = el.getBoundingClientRect();
          if (r.right > window.innerWidth + 1 && getComputedStyle(el).visibility !== "hidden" && r.width > 0) out.push(`${(el.getAttribute("class") ?? el.tagName).slice(0, 60)} reaches ${Math.round(r.right)}`);
        });
        return [...new Set(out)].slice(0, 20);
      });
      expect(wide).toEqual([]);
    }
  });
}

// Settings parts in a real browser: the keys and the focus, which the unit tests cannot see.
test("gallery settings keys: section list, dialog trap, typed confirm, inline confirm, secret", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 900 });
  page.on("pageerror", (e) => console.log(`[pageerror] ${e.message}`));
  await page.goto("/gallery.html?mode=light&size=100&only=settings");
  await page.waitForSelector("[data-shot]");
  const active = () => page.evaluate(() => { const a = document.activeElement as HTMLElement | null; return a ? `${a.tagName}|${(a.textContent ?? "").trim().slice(0, 40)}|${a.getAttribute("aria-label") ?? ""}` : "none"; });

  // Section list: one Tab stop on the open section, arrows move focus, Return opens.
  const nav = page.locator('[data-shot="nav"] .g-frame-wide nav');
  const items = nav.locator(".w-snav-item");
  expect(await items.evaluateAll((els) => els.map((e) => (e as HTMLElement).tabIndex))).toEqual([-1, 0, -1, -1, -1, -1]);
  await items.nth(1).focus();
  await page.keyboard.press("ArrowDown");
  expect(await active()).toContain("Meetings and recordings");
  await page.keyboard.press("End");
  expect(await active()).toContain("Storage and keeping data");
  await page.keyboard.press("Home");
  expect(await active()).toContain("General");
  expect(await nav.locator('[aria-current="page"]').textContent()).toContain("Capture and privacy");
  await page.keyboard.press("Enter");
  expect(await nav.locator('[aria-current="page"]').textContent()).toContain("General");
  await page.keyboard.press("Tab");
  expect(await nav.evaluate((n) => n.contains(document.activeElement))).toBe(false);
  await page.keyboard.press("Shift+Tab");
  expect(await active()).toContain("General");
  // Narrow layout: the select shows and the list does not.
  const narrow = page.locator('[data-shot="nav"] .g-frame-narrow nav');
  expect(await narrow.locator("select").isVisible()).toBe(true);
  expect(await narrow.locator(".w-snav-items").isVisible()).toBe(false);
  expect(await nav.locator("select").isVisible()).toBe(false);

  // Typed confirm: focus starts in the box; Return does nothing; Tab stays inside, fields included.
  const dlg = page.locator('[data-shot="dialogs"] [role="alertdialog"]').first();
  const typed = dlg.locator('input[type="text"]');
  await typed.focus();
  await page.keyboard.press("Enter");
  expect(await dlg.isVisible()).toBe(true);
  const confirm = dlg.getByRole("button", { name: "Delete everything" });
  expect(await confirm.isDisabled()).toBe(true);
  await page.keyboard.press("Tab");
  expect(await active()).toContain("Cancel");
  await page.keyboard.press("Tab"); // the confirm button is off, so Tab wraps to the first control: the checkbox
  expect(await active()).toContain("INPUT");
  expect(await dlg.evaluate((d) => d.contains(document.activeElement))).toBe(true);
  await page.keyboard.press("Shift+Tab");
  expect(await active()).toContain("Cancel");
  await typed.fill("DELETE");
  expect(await confirm.isEnabled()).toBe(true);
  await typed.press("Enter");
  expect(await dlg.isVisible()).toBe(true);
  await typed.press("Escape");
  expect(await page.locator('[data-shot="dialogs"] [role="alertdialog"]').count()).toBe(1);

  // Inline confirm: focus goes to Cancel; Escape cancels and focus returns to the button.
  const row = page.locator('[data-shot="lists"] .w-lrow', { hasText: "Test Browser" });
  await row.getByRole("button", { name: "Remove" }).click();
  expect(await active()).toContain("Cancel");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(60);
  expect(await row.getByRole("button", { name: "Remove" }).count()).toBe(1);
  expect(await active()).toContain("Remove");

  // Secret: saving takes the typed value out of the page.
  const secret = page.locator('[data-shot="fields"] .w-setting', { hasText: "Example provider key" });
  await secret.locator('input[type="password"]').fill("sk-example-typed-9f3a");
  await secret.locator('input[type="password"]').press("Enter");
  expect(await secret.locator('input[type="password"]').count()).toBe(0);
  expect(await page.evaluate(() => document.body.innerHTML.includes("sk-example-typed-9f3a"))).toBe(false);
  expect(await secret.textContent()).toContain("Saved");

  // Number: clearing the box shows the line and keeps the stored number after Escape.
  const num = page.locator('[data-shot="fields"] .w-setting', { hasText: "Screen text" }).locator("input.w-num");
  await num.fill("");
  await num.blur();
  expect(await page.locator('[data-shot="fields"]').textContent()).toContain("Enter a number from 1 to 3,650.");
  await num.focus();
  await num.press("Escape");
  expect(await num.inputValue()).toBe("30");
});
