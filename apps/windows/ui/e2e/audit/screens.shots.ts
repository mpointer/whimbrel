// Draws every screen the fake bridge can reach and saves a PNG of each, in light and dark, at a small and a large
// window size. Output: docs/ui-redesign/screenshots/<AUDIT_SET>/windows-browser/. The shell's commands are answered
// by e2e/fakeBridge.ts plus e2e/audit/fixtures.ts (fictional data: Alex Rivera, Northwind, Acme). A command nobody
// answers returns null and is logged as UNANSWERED, so a screen that crashes or draws blank shows up in the run.
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test, type Page } from "@playwright/test";
import { BUILT_IN, installBridge, type SkinDto } from "../fakeBridge";
import { answers, dynamic, helpers, livePatch, recordingPatch, type LiveKind } from "./fixtures";

const SET = process.env.AUDIT_SET || "before";
/** The after set draws the redesigned launch screens: the hidden flag is on for them. */
const AFTER = SET === "after";
// AUDIT_OUT sends the pictures elsewhere (a temporary folder, to compare a rerun against the committed set).
const OUT = process.env.AUDIT_OUT ? `${process.env.AUDIT_OUT.replace(/\/$/, "")}/` : fileURLToPath(new URL(`../../../../../docs/ui-redesign/screenshots/${SET}/windows-browser/`, import.meta.url));
mkdirSync(OUT, { recursive: true });

/** The pictures are taken at one fixed moment (Wed 18 Nov 2026, 10:22 in the page's time zone, which the audit config
 *  pins to UTC), so the greeting, the "in 9 minutes" lines and every time label are the same on every run. The page's
 *  `Date` is fixed; timers still run. */
const NOW = Date.UTC(2026, 10, 18, 10, 22, 0);

const SIZES = [
  { name: "small", width: 900, height: 640 },
  { name: "large", width: 1440, height: 900 },
];
const LARGE = SIZES[1];
const SMALL = SIZES[0];
const TABS = ["today", "meetings", "library", "topics", "automations", "settings"];
const SECTIONS = ["status", "general", "profile", "capture", "meetings", "ai", "topics", "devices", "connectors"];
/** The shell's other windows, by label (src/main.tsx), at about the size the shell opens them. */
const WINDOWS = [
  { label: "popup", width: 320, height: 260 },
  { label: "alert", width: 380, height: 200 },
  { label: "question-alert", width: 380, height: 220 },
  { label: "budget-alert", width: 380, height: 200 },
  { label: "disk-alert", width: 380, height: 200 },
  { label: "still-here", width: 380, height: 200 },
  { label: "hotkey-panel", width: 560, height: 420 },
];

type Mode = "light" | "dark";

/** The skin a screen is drawn in: the shipped Whimbrel skin unless a test asks for another. */
interface Skin {
  id: string;
  palette: string;
  /** Home's element choices: draws a made-up skin (Thread's colors and shape) that makes them, under the id. */
  options?: Record<string, string>;
}
const WHIMBREL: Skin = { id: "whimbrel", palette: "default" };

/** A skin nobody ships: Thread's palettes and shape with the element choices the test names. */
function madeUp(skin: Skin): SkinDto {
  const thread = BUILT_IN.find((d) => d.skin.id === "thread")!;
  return { ...thread, builtIn: false, skin: { ...thread.skin, id: skin.id, name: skin.id, today: { archetype: "standard", options: skin.options! } } };
}

async function open(page: Page, mode: Mode, label = "main", patch: Record<string, unknown> = {}, wait = 900, skin: Skin = WHIMBREL) {
  await page.emulateMedia({ colorScheme: mode });
  await installBridge(page, { skins: skin.options ? [...BUILT_IN, madeUp(skin)] : BUILT_IN, skinId: skin.id, paletteId: skin.palette, mode, lang: "en-US" });
  page.on("pageerror", (e) => console.log(`[pageerror ${label}] ${e.message.split("\n")[0]}`));
  page.on("console", (m) => m.type() === "error" && console.log(`[console ${label}] ${m.text().slice(0, 240)}`));
  const dyn = Object.fromEntries(Object.entries(dynamic).map(([k, f]) => [k, f.toString()]));
  const now = NOW;
  await page.clock.setFixedTime(NOW);
  // After the bridge: this window's label, the fixtures, and any answers this screen needs changed.
  await page.addInitScript(
    ({ label, patch, dyn, extra, fx }) => {
      const w = window as unknown as Record<string, unknown>;
      w.__fx = fx;
      // The event plugin's own bookkeeping, which the fake bridge does not provide (listen/unlisten work, nothing arrives).
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      const t = (w as { __TAURI_INTERNALS__: { metadata: unknown; invoke: (cmd: string, args?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
      t.metadata = { currentWindow: { label }, currentWebview: { label, windowLabel: label } };
      const base = t.invoke;
      const fns: Record<string, (a: unknown) => unknown> = {};
      for (const [k, src] of Object.entries(dyn)) fns[k] = (0, eval)(`(${src})`);
      const all = { ...extra, ...patch } as Record<string, unknown>;
      const seen = new Set<string>();
      t.invoke = async (cmd, args) => {
        // "__hang__" never answers (a screen's loading state); "__fail__:text" rejects (its error state).
        if (cmd in all) {
          const v = all[cmd];
          if (v === "__hang__") return new Promise(() => {});
          if (typeof v === "string" && v.startsWith("__fail__:")) throw v.slice(9);
          return v;
        }
        if (cmd in fns) return fns[cmd](args);
        const r = await base(cmd, args);
        if (r === null && !seen.has(cmd) && !cmd.startsWith("plugin:")) {
          seen.add(cmd);
          console.error(`UNANSWERED ${cmd} ${JSON.stringify(args ?? null).slice(0, 100)}`);
        }
        return r;
      };
    },
    { label, patch, dyn, extra: answers(now), fx: helpers(now) },
  );
  await page.goto("/");
  await page.waitForTimeout(wait);
}

/** A screenshot as the window shows it; with `tall`, the window is first made as tall as the scrolling content so the
 *  whole page is in the picture (the content area scrolls inside the window, so Playwright's fullPage would not). */
async function shot(page: Page, name: string, tall = false) {
  if (tall) {
    const vp = page.viewportSize()!;
    const extra = await page.evaluate(() => {
      const c = document.querySelector(".content");
      return c ? Math.max(0, c.scrollHeight - c.clientHeight) : 0;
    });
    if (extra > 4) {
      await page.setViewportSize({ width: vp.width, height: Math.min(vp.height + extra, 5000) });
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${OUT}${name}.png` });
      await page.setViewportSize(vp);
      return;
    }
  }
  await page.screenshot({ path: `${OUT}${name}.png` });
}

const settle = (page: Page, ms = 700) => page.waitForTimeout(ms);
const tab = (page: Page, id: string) => page.locator(`#tab-${id}`).click();

for (const mode of ["light", "dark"] as const) {
  // --- the six tabs, at both sizes -------------------------------------------------------------------------------------------
  for (const size of SIZES)
    for (const id of TABS)
      test(`${id} ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode);
        await tab(page, id);
        await settle(page);
        // Meetings: the transcript is closed until asked for, so open it to show the whole workspace.
        if (id === "meetings") {
          await page.getByRole("button", { name: "Show", exact: true }).first().click();
          await settle(page, 400);
        }
        await shot(page, `${id}-${mode}-${size.name}`, size === LARGE);
      });

  // --- Library, every mode ------------------------------------------------------------------------------------------------------
  for (const size of SIZES)
    for (const lib of ["recent", "episodes", "keyword", "semantic", "ask", "ask-new", "digest", "capture"])
      test(`library ${lib} ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode);
        await tab(page, "library");
        await settle(page, 500);
        const modeButton = (n: string) => page.locator(".mode-picker").getByRole("button", { name: n, exact: true }).click();
        if (lib === "recent") {
          // as opened
        } else if (lib === "episodes") await modeButton("Episodes");
        else if (lib === "keyword" || lib === "semantic") {
          await modeButton(lib === "keyword" ? "Keyword" : "Semantic");
          await page.getByRole("searchbox").fill("Northwind renewal");
          await page.getByRole("button", { name: "Search", exact: true }).click();
        } else if (lib === "ask" || lib === "ask-new") {
          await modeButton("Ask");
          await settle(page, 400);
          if (lib === "ask") await page.locator(".conversation-title").first().click();
        } else if (lib === "digest") await modeButton("Digest");
        else if (lib === "capture") await page.locator(".row-card").nth(1).click();
        await settle(page, 700);
        await shot(page, `library-${lib}-${mode}-${size.name}`, size === LARGE);
      });

  test(`topics empty ${mode}`, async ({ page }) => {
    await page.setViewportSize(LARGE);
    await open(page, mode, "main", { topics_page_list: [] });
    await tab(page, "topics");
    await settle(page);
    await shot(page, `topics-empty-${mode}-large`);
  });

  // --- Settings: each section (the nav items are buttons with id settings-<section>) ------------------------------------------
  for (const section of SECTIONS)
    test(`settings ${section} ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode);
      await tab(page, "settings");
      await settle(page, 500);
      await page.locator(`#settings-${section}`).click();
      await settle(page, 900);
      await shot(page, `settings-${section}-${mode}-large`, true);
    });

  // --- recording in progress -------------------------------------------------------------------------------------------------------
  for (const size of SIZES) {
    test(`meetings recording ${mode} ${size.name}`, async ({ page }) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", recordingPatch(NOW));
      await tab(page, "meetings");
      await settle(page, 1200);
      await shot(page, `meetings-recording-${mode}-${size.name}`, size === LARGE);
    });
  }
  test(`today recording ${mode}`, async ({ page }) => {
    await page.setViewportSize(LARGE);
    await open(page, mode, "main", recordingPatch(NOW));
    await settle(page, 1200);
    await shot(page, `today-recording-${mode}-large`, true);
  });

  // --- the small windows --------------------------------------------------------------------------------------------------------------
  for (const w of WINDOWS)
    test(`${w.label} ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: w.width, height: w.height });
      await open(page, mode, w.label);
      await page.screenshot({ path: `${OUT}window-${w.label}-${mode}.png` });
    });
  test(`popup recording ${mode}`, async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 260 });
    await open(page, mode, "popup", { tray_status: { captureEnabled: true, line: "Capturing · 1,482 today", recordingStartedUnixMs: NOW - 7 * 60_000 } });
    await page.screenshot({ path: `${OUT}window-popup-recording-${mode}.png` });
  });

  // --- launch stages -------------------------------------------------------------------------------------------------------------------
  // With AUDIT_SET=after the hidden redesign flag is on (the fixture answer `redesign_enabled: true`), so the launch
  // draws the Thread layout. Every other set draws the old screens, which is what the before pictures show.
  const launch = (stage: unknown, extra: Record<string, unknown> = {}) => ({
    launch_state: { stage, busy: false, waitingForUser: true, generation: 1 },
    redesign_enabled: AFTER,
    ...extra,
  });
  const WORDS = "orbit lantern meadow copper harbor velvet anchor pebble summit marble thistle ember quartz willow saddle nimbus tundra fabric glacier hollow ivory juniper kettle lagoon".split(" ");
  const SETUP = { setup_begin: { kind: "made", words: WORDS }, setup_save_file: { kind: "saved", path: "D:\\Backups\\whimbrel-recovery.json", sameDiskAsDatabase: false }, setup_verify: true, setup_saved_to: null };
  const PHRASE = "correct horse battery staple";
  const LAUNCH: [string, unknown][] = [
    ["starting", { kind: "starting" }],
    ["onboarding", { kind: "onboarding", reusesKey: false }],
    ["migrating", { kind: "migrating", overall: 0.42 }],
    ["recovery-needs-key", { kind: "recovery", reason: "needsKey" }],
    ["recovery-key-rejected", { kind: "recovery", reason: "keyRejected" }],
    ["key-unavailable", { kind: "keyUnavailable", stillUnavailable: false }],
    ["screen-damaged", { kind: "screen", lastAction: null, offers: ["retry", "quit"], screen: { kind: "damaged", reason: "damaged" } }],
    ["screen-key-storage", { kind: "screen", lastAction: null, offers: ["retry", "quit"], screen: { kind: "keyStorageUnavailable" } }],
    ["screen-encrypted-missing", { kind: "screen", lastAction: null, offers: ["restoreRollbackCopy", "resetEncryptionStatus", "quit"], screen: { kind: "encryptedFileMissing", backupPresent: true, encryptedCopyPresent: false } }],
  ];
  // Where a stage is drawn: the before set has these sizes only; the after set draws every stage at both.
  const launchSizes = (name: string) => (AFTER ? SIZES : name === "onboarding" ? SIZES : [SMALL]);
  for (const [name, stage] of LAUNCH)
    for (const size of launchSizes(name))
      test(`launch ${name} ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        // "starting" shows its spinner only after a second.
        await open(page, mode, "main", launch(stage), name === "starting" ? 1700 : 900);
        await shot(page, `launch-${name}-${mode}-${size.name}`);
      });

  if (!AFTER) {
    // The old recovery setup is one long page: all four steps at once, filled in down to the test.
    test(`launch onboarding words ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: LARGE.width, height: 1500 });
      await open(page, mode, "main", launch({ kind: "onboarding", reusesKey: false }, SETUP));
      const inputs = page.locator("input[type=password]");
      await inputs.nth(0).fill(PHRASE);
      await inputs.nth(1).fill(PHRASE);
      await page.getByRole("button", { name: "Create recovery key" }).click();
      await settle(page, 600);
      await page.getByRole("button", { name: /Save recovery file/ }).click();
      await settle(page, 400);
      await page.getByLabel("I wrote down all 24 words, in order").check();
      await page.getByLabel("Passphrase", { exact: true }).last().fill(PHRASE);
      await page.getByRole("button", { name: "Test", exact: true }).click();
      await settle(page, 400);
      await shot(page, `launch-onboarding-words-${mode}-large`);
    });
  } else {
    // The new setup is four pages. This walks them: the words page keeps the old name; the others are extra.
    for (const size of SIZES)
      test(`launch onboarding steps ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode, "main", launch({ kind: "onboarding", reusesKey: false }, SETUP));
        const first = page.getByLabel("Passphrase", { exact: true });
        await first.fill(PHRASE);
        await page.getByLabel("Type it again").fill("correct horse");
        await settle(page, 300);
        await shot(page, `launch-onboarding-mismatch-${mode}-${size.name}`);
        await page.getByLabel("Type it again").fill(PHRASE);
        await page.keyboard.press("Enter");
        await settle(page, 600);
        await shot(page, `launch-onboarding-file-${mode}-${size.name}`);
        await page.getByRole("button", { name: /Save recovery file/ }).click();
        await settle(page, 400);
        await shot(page, `launch-onboarding-file-saved-${mode}-${size.name}`);
        await page.getByRole("button", { name: "Next step" }).click();
        await page.getByLabel("I wrote down all 24 words, in order").check();
        await settle(page, 300);
        await shot(page, `launch-onboarding-words-${mode}-${size.name}`);
        await page.getByRole("button", { name: "Next step" }).click();
        await settle(page, 300);
        await shot(page, `launch-onboarding-test-${mode}-${size.name}`);
        await page.getByLabel("Passphrase", { exact: true }).fill(PHRASE);
        await page.keyboard.press("Enter");
        await settle(page, 400);
        await page.getByLabel("I have kept my recovery file or my 24 words somewhere other than this PC").check();
        await settle(page, 300);
        await shot(page, `launch-onboarding-ready-${mode}-${size.name}`);
        await page.getByRole("button", { name: "Back" }).click();
        await page.getByRole("button", { name: "Back" }).click();
        await page.getByRole("button", { name: "Back" }).click();
        await page.getByRole("button", { name: "Start over…" }).click();
        await settle(page, 300);
        await shot(page, `launch-onboarding-start-over-${mode}-${size.name}`);
      });

    // The recovery screen with the 24 words chosen and a failed attempt, and a blocking screen with its question open.
    test(`launch recovery words ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", launch({ kind: "recovery", reason: "needsKey" }, { launch_recovery_unlock_words: "badWords" }));
      await page.getByRole("radio", { name: "24 words" }).click();
      await page.getByLabel("24 words", { exact: true }).fill("orbit lantern meadow");
      await page.getByRole("button", { name: "Unlock with words" }).click();
      await settle(page, 400);
      await shot(page, `launch-recovery-words-error-${mode}-large`);
    });
    test(`launch question ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", launch(LAUNCH[8][1]));
      await page.getByRole("button", { name: "Reset encryption status…" }).click();
      await settle(page, 300);
      await shot(page, `launch-screen-encrypted-missing-question-${mode}-small`);
    });

    // The one blocking screen with three real offers (the shell's own list for a failed plain open), whose row is the
    // widest the launch draws.
    test(`launch open failed ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", launch({ kind: "screen", lastAction: null, offers: ["retry", "quit", "continueWithoutStore"], screen: { kind: "error", error: { kind: "plainOpenFailed" } } }));
      await shot(page, `launch-screen-open-failed-${mode}-small`);
    });

    // Text at 200%, which the spec requires the screens to hold: the root font size doubled, on the longest page
    // (the words) and on a blocking screen.
    const zoom = async (page: Page) => {
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(300);
    };
    test(`launch words zoom ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", launch({ kind: "onboarding", reusesKey: false }, SETUP));
      await page.getByLabel("Passphrase", { exact: true }).fill(PHRASE);
      await page.getByLabel("Type it again").fill(PHRASE);
      await page.keyboard.press("Enter");
      await settle(page, 500);
      await page.getByRole("button", { name: "Next step" }).click();
      await zoom(page);
      await shot(page, `launch-onboarding-words-zoom200-${mode}-small`);
    });
    test(`launch screen zoom ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", launch(LAUNCH[8][1]));
      await zoom(page);
      await shot(page, `launch-screen-encrypted-missing-zoom200-${mode}-small`);
    });

    // The no-store banner, over the app and in the tray popup.
    const noStore = { launch_state: { stage: { kind: "openedWithoutStore" }, busy: false, waitingForUser: false, generation: 1 }, redesign_enabled: true };
    test(`app no-store banner ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", noStore);
      await settle(page, 400);
      await shot(page, `app-no-store-banner-${mode}-large`);
    });
    test(`popup no-store banner ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: 320, height: 260 });
      await open(page, mode, "popup", noStore);
      await page.screenshot({ path: `${OUT}window-popup-no-store-${mode}.png` });
    });

    // The three first-run banners over the app (Arm notice, expired Google sign-in, the first retention notice).
    test(`first-run banners ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      const now = NOW;
      await open(page, mode, "main", {
        redesign_enabled: true,
        running_emulated: true,
        retention_settings: { acknowledged: false, lastRun: null, settings: { captureDays: 90, conversationDays: null, meetingDays: 365 } },
        retention_preview: { captures: 214, conversations: 0, meetings: 3, orphanBytes: 0, orphans: 0 },
        calendar_accounts: { accessIssues: [], blocked: null, emails: ["alex.rivera@contoso.example.test"], lastSyncedUnixMs: now - 12 * 60_000, nextMeeting: null, nextStartUnixMs: null, signInExpired: ["alex.rivera@contoso.example.test"], syncError: null, upcoming: 0 },
      });
      await settle(page, 600);
      await shot(page, `first-run-banners-${mode}-large`);
    });

    // The same screens in two other skins: the layout is the flag's, the colors, fonts and shape are the skin's.
    const SKINS: [string, Skin, Mode[]][] = [
      ["thread", { id: "thread", palette: "thread" }, ["light", "dark"]],
      ["retro-terminal", { id: "retro-terminal", palette: "phosphor" }, ["dark"]],
    ];
    for (const [label, skin, modes] of SKINS)
      if (modes.includes(mode))
        for (const [name, stage] of LAUNCH.filter(([n]) => ["onboarding", "recovery-needs-key", "screen-encrypted-missing"].includes(n)))
          test(`launch ${name} skin ${label} ${mode}`, async ({ page }) => {
            await page.setViewportSize(SMALL);
            await open(page, mode, "main", launch(stage), 900, skin);
            await shot(page, `launch-${name}-skin-${label}-${mode}-small`);
          });

    // ===== Home and the app shell, redesigned (screen 2). The flag is on, so every picture below is the new look. ====
    const ON = { redesign_enabled: true };
    const THREAD: Skin = { id: "thread", palette: "thread" };
    const EMPTY_DAY = {
      today_schedule: { events: [], earlierToday: [], recording: false },
      next_meeting_prep: null,
      today_upcoming: { days: [], syncError: null },
      library_day_blocks: [],
      approvals_count: 0,
      meeting_most_relevant: null,
      today_overview: { day: "2026-11-18", devices: [], openLoops: [], recap: null, briefRecap: null, latestMeeting: null },
      today_recap: { cached: false, day: "2026-11-18", emptyDay: true, openLoops: [], summary: "" },
    };
    const paused = { capture_status: { enabled: true, excluded: 41, lastApp: "Microsoft Excel", lastError: null, lastStoredAtMs: NOW - 4 * 60_000, ocrRuns: 120, paused: "capture is paused for a set time", resumeAtUnixMs: NOW + 28 * 60_000, running: true, stored: 1482, unchanged: 9120 } };
    const off = { capture_status: { enabled: false, excluded: 41, lastApp: null, lastError: null, lastStoredAtMs: null, ocrRuns: 120, paused: "capture is turned off", running: true, stored: 1482, unchanged: 9120 } };

    for (const size of SIZES) {
      test(`home ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode, "main", ON, 1100, THREAD);
        await shot(page, `home-${mode}-${size.name}`, size === LARGE);
      });
      test(`home skin whimbrel ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode, "main", ON, 1100);
        await shot(page, `home-skin-whimbrel-${mode}-${size.name}`, size === LARGE);
      });
    }

    // Each variant of each element, on a made-up skin that changes only that one choice.
    const VARIANTS: [string, string][] = [
      ["nextMeeting", "scoreboard"], ["nextMeeting", "infobox"], ["timeline", "ruled"], ["labels", "bracketed"],
      ["labels", "smallCaps"], ["ask", "prompt"], ["actions", "links"],
    ];
    for (const [key, value] of VARIANTS)
      test(`home variant ${key} ${value} ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        const options = { nextMeeting: "card", timeline: "line", labels: "plain", ask: "field", actions: "buttons", [key]: value };
        await open(page, mode, "main", ON, 1100, { id: `made-up-${key}-${value}`, palette: "thread", options });
        await shot(page, `home-variant-${key}-${value}-${mode}-large`, true);
      });

    // The old layouts' skins, converted by their archetype alone (no element keys): Retro Terminal, Almanac, Reference.
    for (const id of ["retro-terminal", "almanac", "reference"]) {
      const dto = BUILT_IN.find((d) => d.skin.id === id);
      if (!dto) continue;
      const pal = dto.palettes.find((x) => (mode === "dark" ? x.dark : x.light)) ?? dto.palettes[0];
      if (!(mode === "dark" ? pal.dark : pal.light)) continue;
      test(`home skin ${id} ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        await open(page, mode, "main", ON, 1100, { id, palette: pal.id });
        await shot(page, `home-skin-${id}-${mode}-large`, true);
      });
    }

    // States.
    test(`home empty ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...EMPTY_DAY }, 1100, THREAD);
      await shot(page, `home-empty-${mode}-large`);
    });
    test(`home no calendar ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      const none = { accessIssues: [], blocked: null, emails: [], lastSyncedUnixMs: null, nextMeeting: null, nextStartUnixMs: null, signInExpired: [], syncError: null, upcoming: 0 };
      await open(page, mode, "main", { ...ON, ...EMPTY_DAY, calendar_accounts: none }, 1100, THREAD);
      await shot(page, `home-no-calendar-${mode}-large`);
    });
    test(`home loading ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, today_overview: "__hang__" }, 700, THREAD);
      await shot(page, `home-loading-${mode}-large`);
    });
    test(`home error ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, today_overview: "__fail__:The memory database is busy." }, 900, THREAD);
      await shot(page, `home-error-${mode}-large`);
    });
    test(`home capture paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...paused, library_day_blocks: [] }, 1100, THREAD);
      await shot(page, `home-capture-paused-${mode}-large`);
    });
    test(`home capture off ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...off, library_day_blocks: [] }, 1100, THREAD);
      await shot(page, `home-capture-off-${mode}-large`);
    });
    test(`home recording ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...recordingPatch(NOW) }, 1300, THREAD);
      await shot(page, `home-recording-${mode}-large`, true);
    });
    test(`home processing ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, meeting_recording_status: { degraded: null, failure: null, meetingId: 1, processing: { label: "Writing the summary", meetingId: 1, stage: "summarizing" }, startedUnixMs: null, state: "idle" } }, 1300, THREAD);
      await shot(page, `home-processing-${mode}-large`);
    });
    test(`home zoom 200 ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", ON, 1100, THREAD);
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(400);
      await shot(page, `home-zoom200-${mode}-small`, true);
    });

    // Interactions: the capture menu, the filter menus, a filtered search that lands in Library, Ctrl+K and Ctrl+2.
    test(`home capture menu ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", ON, 1100, THREAD);
      await page.locator(".w-capture-button").click();
      await settle(page, 300);
      await shot(page, `home-capture-menu-${mode}-large`);
    });
    test(`home capture menu paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...paused }, 1100, THREAD);
      await page.locator(".w-capture-button").click();
      await settle(page, 300);
      await shot(page, `home-capture-menu-paused-${mode}-large`);
    });
    test(`home filters ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", ON, 1100, THREAD);
      await page.getByRole("button", { name: "Apps", exact: true }).click();
      await settle(page, 300);
      await shot(page, `home-filter-apps-${mode}-large`);
      await page.getByRole("menuitemradio", { name: "Excel" }).click();
      await page.getByRole("button", { name: "Dates", exact: true }).click();
      await settle(page, 300);
      await shot(page, `home-filter-dates-${mode}-large`);
      await page.getByRole("menuitemradio", { name: "Last 7 days" }).click();
      await page.getByRole("searchbox", { name: "Ask Whimbrel" }).fill("Northwind renewal");
      await settle(page, 300);
      await shot(page, `home-filter-chosen-${mode}-large`);
      await page.getByRole("button", { name: "Search", exact: true }).click();
      await settle(page, 900);
      await shot(page, `home-filter-library-${mode}-large`);
    });
    test(`shell shortcuts ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", ON, 1100, THREAD);
      await page.keyboard.press("Control+2");
      await settle(page, 500);
      await shot(page, `shell-meetings-${mode}-large`);
      await page.keyboard.press("Control+k");
      await settle(page, 500);
      await shot(page, `shell-ctrl-k-${mode}-large`);
    });
    for (const id of ["library", "settings"])
      test(`shell ${id} ${mode}`, async ({ page }) => {
        await page.setViewportSize(SMALL);
        await open(page, mode, "main", ON, 1100, THREAD);
        await tab(page, id);
        await settle(page, 700);
        await shot(page, `shell-${id}-${mode}-small`);
      });
    test(`shell recording meetings ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...ON, ...recordingPatch(NOW) }, 1100, THREAD);
      await tab(page, "automations");
      await settle(page, 800);
      await shot(page, `shell-recording-automations-${mode}-large`);
    });
    test(`popup pause ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: 320, height: 260 });
      await open(page, mode, "popup", { ...ON, tray_status: { captureEnabled: true, line: "Capturing · Synced 10:10 AM", recordingStartedUnixMs: null, resumeAtUnixMs: null } });
      await page.screenshot({ path: `${OUT}window-popup-pause-${mode}.png` });
    });
    test(`popup paused ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: 320, height: 260 });
      await open(page, mode, "popup", { ...ON, tray_status: { captureEnabled: true, line: "Capture paused · Synced 10:10 AM", recordingStartedUnixMs: null, resumeAtUnixMs: NOW + 28 * 60_000 } });
      await page.screenshot({ path: `${OUT}window-popup-paused-${mode}.png` });
    });

    // ===== The live meeting screen (screen 3). The flag is on; Home gives its place to the screen while a meeting records. ====
    const live = (kind: LiveKind = "recording", more: Record<string, unknown> = {}) => ({ ...ON, ...livePatch(NOW, kind), ...more });
    for (const size of SIZES)
      test(`live recording ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode, "main", live(), 1500, THREAD);
        await shot(page, `live-recording-${mode}-${size.name}`, true);
      });
    for (const kind of ["paused", "resuming", "problem", "processing", "starting", "readOnly", "noTranscript", "marks"] as LiveKind[])
      test(`live ${kind} ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        // Read only is another device's meeting, reached from the Meetings tab; the rest take Home's place.
        await open(page, mode, "main", live(kind), 1500, THREAD);
        if (kind === "readOnly") {
          await tab(page, "meetings");
          await settle(page, 1200);
        }
        await shot(page, `live-${kind.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}-${mode}-large`, true);
      });
    test(`live no notice ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", live("recording", { recording_notice_settings: { enabled: false, text: "" } }), 1500, THREAD);
      await shot(page, `live-no-notice-${mode}-large`);
    });
    test(`live meetings tab ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", live(), 1300, THREAD);
      await tab(page, "meetings");
      await settle(page, 1400);
      await shot(page, `live-meetings-tab-${mode}-large`, true);
    });
    test(`live zoom 200 ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", live(), 1500, THREAD);
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(500);
      await shot(page, `live-zoom200-${mode}-small`, true);
    });
    test(`live paused zoom 200 ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", live("paused"), 1500, THREAD);
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(500);
      await shot(page, `live-paused-zoom200-${mode}-small`, true);
    });
    test(`live skin whimbrel ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", live(), 1500);
      await shot(page, `live-skin-whimbrel-${mode}-large`, true);
    });
    test(`shell recording paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", live("paused"), 1100, THREAD);
      await tab(page, "library");
      await settle(page, 800);
      await shot(page, `shell-recording-paused-library-${mode}-large`);
    });
    for (const [name, kind] of [["recording", "recording"], ["paused", "paused"]] as const)
      test(`popup recording ${name} ${mode}`, async ({ page }) => {
        await page.setViewportSize({ width: 320, height: 300 });
        const status = livePatch(NOW, kind).meeting_recording_status;
        await open(page, mode, "popup", { ...ON, meeting_recording_status: status, tray_status: { captureEnabled: true, line: "Capturing · Synced 10:10 AM", recordingPaused: kind === "paused", pausedSinceUnixMs: kind === "paused" ? NOW - 2 * 60_000 : null, recordingStartedUnixMs: NOW - 14 * 60_000 - 8_000, resumeAtUnixMs: null } });
        await page.screenshot({ path: `${OUT}window-popup-recording-${name}-${mode}.png` });
      });
    for (const kind of ["recording", "paused"] as LiveKind[])
      test(`in-meeting window ${kind} ${mode}`, async ({ page }) => {
        await page.setViewportSize({ width: 320, height: 260 });
        await open(page, mode, "meeting-bar", live(kind));
        await page.screenshot({ path: `${OUT}window-in-meeting-${kind}-${mode}.png` });
      });
    test(`still paused ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width: 380, height: 200 });
      await open(page, mode, "still-here", live("paused"));
      await page.screenshot({ path: `${OUT}window-still-paused-${mode}.png` });
    });
  }
}
