// Draws every screen the fake bridge can reach and saves a PNG of each, in light and dark, at a small and a large
// window size. Output: docs/ui-redesign/screenshots/after/windows-browser/ (AUDIT_OUT sends it elsewhere). The shell's commands are answered
// by e2e/fakeBridge.ts plus e2e/audit/fixtures.ts (fictional data: Alex Rivera, Northwind, Acme). A command nobody
// answers returns null and is logged as UNANSWERED, so a screen that crashes or draws blank shows up in the run.
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type Browser, type Page } from "@playwright/test";
import { BUILT_IN, installBridge, type SkinDto } from "../fakeBridge";
import { SETTINGS_SEARCH_ENTRIES } from "../../src/settings/settingsSearch";
import { answers, askScript, convPatch, dynamic, findPatch, helpers, livePatch, notePatch, recordingPatch, type AskScript, type ConvKind, type FindKind, type LiveKind, type NoteKind } from "./fixtures";

// The before set is frozen: its pictures are in docs/ui-redesign/screenshots/before/ and are never redrawn, because the
// old screens cannot be reached any more (the redesign switch is on for everyone, docs/ui-redesign/screens/switch-removal.md).
// Asking for it is an error, not a quiet no-op, so nobody overwrites the frozen set with new pictures.
const SET = process.env.AUDIT_SET || "after";
if (SET !== "after") throw new Error(`AUDIT_SET=${SET}: only "after" can be drawn; the before pictures are frozen (the old screens are gone from the build).`);
/** Always true now: kept so the blocks below that were written for the after set read as before. */
const AFTER = true;
// AUDIT_OUT sends the pictures elsewhere (a temporary folder, to compare a rerun against another run).
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
/** The tabs the restyle pass (step 10) draws with the flag on, in the block at the end of this file, under the same names. */
const RESTYLED = ["meetings", "topics", "automations"];
/** The shell's other windows, by label (src/main.tsx), at the size the shell opens them: the `SIZE` constants in
 *  src-tauri/src (tray.rs `POPUP_SIZE`, alert.rs, question_alert.rs, budget_alert.rs, disk_alert.rs,
 *  meetings/still_here.rs, hotkey_panel/window.rs, meetings/meeting_bar.rs), so a clipped window clips here too. */
const WINDOWS = [
  // The redesigned popup is taller: Recording, Memory with its choices, Needs attention and Next meeting (tray.rs `POPUP_SIZE_REDESIGN`).
  { label: "popup", width: 320, height: 560 },
  // The redesigned alert is a row taller for the countdown line (alert.rs `SIZE_REDESIGN`).
  { label: "alert", width: 360, height: 230 },
  { label: "question-alert", width: 380, height: 210 },
  { label: "budget-alert", width: 380, height: 170 },
  { label: "disk-alert", width: 380, height: 170 },
  { label: "still-here", width: 380, height: 170 },
  { label: "hotkey-panel", width: 560, height: 360 },
  { label: "meeting-bar", width: 320, height: 260 },
];
const WIN = Object.fromEntries(WINDOWS.map((w) => [w.label, { width: w.width, height: w.height }])) as Record<string, { width: number; height: number }>;
const MEETING_BAR = { width: 320, height: 260 };
/** A problem that can coexist with a pause (capture/audio.rs: a headset at call quality), and one that cannot (a stream that stopped delivering: a pause delivers nothing by design). */
const HEADSET = "Your headset switched to call quality, so the meeting's audio may sound muffled.";
const MIC_STALLED = "The microphone isn't delivering audio, so this meeting records system audio only.";

type Mode = "light" | "dark";

/** The skin a screen is drawn in: the shipped Whimbrel skin unless a test asks for another. */
interface Skin {
  /** null stores nothing, as on a fresh device. */
  id: string | null;
  palette: string | null;
  /** Whether the once-per-device move to Thread has been looked at (src/skin.ts). Stored as on a device that has run before,
   *  so a stored cream default is drawn as cream, as it was before the switch came off; false lets the page move it. */
  moveDone?: boolean;
  /** Home's element choices: draws a made-up skin (Thread's colors and shape) that makes them, under the id. */
  options?: Record<string, string>;
}
const WHIMBREL: Skin = { id: "whimbrel", palette: "default" };

/** A skin nobody ships: Thread's palettes and shape with the element choices the test names. */
function madeUp(skin: Skin): SkinDto {
  const thread = BUILT_IN.find((d) => d.skin.id === "thread")!;
  return { ...thread, builtIn: false, skin: { ...thread.skin, id: skin.id!, name: skin.id!, today: { archetype: "standard", options: skin.options! } } };
}

async function open(page: Page, mode: Mode, label = "main", patch: Record<string, unknown> = {}, wait = 900, skin: Skin = WHIMBREL) {
  await page.emulateMedia({ colorScheme: mode });
  await installBridge(page, { skins: skin.options ? [...BUILT_IN, madeUp(skin)] : BUILT_IN, skinId: skin.id, paletteId: skin.palette, mode, lang: "en-US", moveDone: skin.moveDone });
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
      // Channels that deliver: the base bridge's `transformCallback` returns 0 and nothing ever arrives. A screen that scripts an ask
      // (`__ask_script`) gets its events through the channel the page passed, each after its own delay; `chat_stop` ends it as Stop does.
      const callbacks: Record<number, (m: unknown) => void> = {};
      let nextId = 1;
      const timers: ReturnType<typeof setTimeout>[] = [];
      let playing = 0;
      const tt = t as unknown as { transformCallback: (cb: (m: unknown) => void) => number; unregisterCallback: (id: number) => void };
      tt.transformCallback = (cb) => {
        const id = nextId++;
        callbacks[id] = cb;
        return id;
      };
      tt.unregisterCallback = (id) => {
        delete callbacks[id];
      };
      const fns: Record<string, (a: unknown) => unknown> = {};
      for (const [k, src] of Object.entries(dyn)) fns[k] = (0, eval)(`(${src})`);
      const all = { ...extra, ...patch } as Record<string, unknown>;
      const seen = new Set<string>();
      t.invoke = async (cmd, args) => {
        const script = all.__ask_script as { at: number; event: unknown }[] | undefined;
        if ((cmd === "chat_ask_sourced" || cmd === "chat_ask_topic" || cmd === "meeting_ask" || cmd === "window_ask") && script) {
          const id = (args as { onEvent: { id: number } }).onEvent.id;
          playing = id;
          let index = 0;
          for (const step of script) timers.push(setTimeout(() => callbacks[id]?.({ index: index++, message: step.event }), step.at));
          // As the real shell does: the call returns as soon as the answer has started, before any event, and says nothing about its end.
          // (The hotkey panel's call returns where the text went.)
          return cmd === "window_ask" ? { kind: "started", requestId: 1, destination: { kind: "local" }, truncated: false } : null;
        }
        if (cmd === "chat_stop") {
          timers.forEach(clearTimeout);
          callbacks[playing]?.({ index: 99, message: { kind: "cancelled" } });
          return true;
        }
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
  // The after set's Settings tab is the redesigned one and is drawn by the Settings block at the end of this file
  // (settings-status-*, settings-<section>-*); this loop would draw it with the switch off, which is the old tab.
  for (const size of SIZES)
    for (const id of TABS.filter((tabId) => tabId !== "settings" && !RESTYLED.includes(tabId)))
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

  // --- Library, as opened (the Find page) ------------------------------------------------------------------------------------------
  // The old Library's modes (Episodes, Keyword, Semantic, Ask, Digest) are gone with the old screens; the Find pages are drawn by the
  // `find *` tests below. The committed `library-<mode>-*` pictures for those modes are from the old Library and are not redrawn.
  for (const size of SIZES)
    test(`library recent ${mode} ${size.name}`, async ({ page }) => {
      await page.setViewportSize(size);
      await open(page, mode);
      await tab(page, "library");
      await settle(page, 500);
      await settle(page, 700);
      await shot(page, `library-recent-${mode}-${size.name}`, size === LARGE);
    });

  // --- Settings: the redesigned tab is drawn by the Settings block at the end of this file (settings-status-*, settings-<section>-*). ---

  // --- recording in progress -------------------------------------------------------------------------------------------------------
  for (const size of SIZES) {
    test(`meetings recording ${mode} ${size.name}`, async ({ page }) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", recordingPatch(NOW));
      await tab(page, "meetings");
      await settle(page, 1200);
      // The meeting that records is the live screen: one recording line and one notice. The old card ("Record a meeting", and the
      // old workspace's notice) used to add a second of each.
      await expect(page.getByRole("region", { name: "Recording notice" })).toHaveCount(1);
      await expect(page.getByRole("status").filter({ hasText: /^Recording/ })).toHaveCount(1);
      await expect(page.getByRole("region", { name: "Let people know you're recording." })).toHaveCount(0);
      await expect(page.locator(".recording-line")).toHaveCount(0);
      await shot(page, `meetings-recording-${mode}-${size.name}`, size === LARGE);
    });
  }
  // While this PC records, the notice stays on Meetings whichever meeting is selected: with another meeting selected the live
  // screen (which draws the notice for the recorded meeting) is not showing, so the Meetings screen draws it. Still exactly one.
  test(`meetings recording, another meeting selected ${mode}`, async ({ page }) => {
    await page.setViewportSize(LARGE);
    await open(page, mode, "main", recordingPatch(NOW));
    await tab(page, "meetings");
    await settle(page, 1200);
    const rows = page.locator(".meeting-item__hit");
    const other = rows.filter({ hasNot: page.locator('[aria-current="true"]') }).and(page.locator(':not([aria-current="true"])')).last();
    await other.click();
    await settle(page, 1200);
    await expect(page.getByRole("region", { name: "Recording notice" })).toHaveCount(1);
    await expect(page.getByText("You are recording. Tell the others before you go on.")).toBeVisible();
    await shot(page, `meetings-recording-other-selected-${mode}-large`, true);
  });
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
  // --- launch stages -------------------------------------------------------------------------------------------------------------------
  // The launch draws the Thread layout (the redesign switch is on for everyone and nothing asks the shell for it).
  const launch = (stage: unknown, extra: Record<string, unknown> = {}) => ({
    launch_state: { stage, busy: false, waitingForUser: true, generation: 1 },
    ...extra,
  });
  const WORDS = "orbit lantern meadow copper harbor velvet anchor pebble summit marble thistle ember quartz willow saddle nimbus tundra fabric glacier hollow ivory juniper kettle lagoon".split(" ");
  const SETUP = { setup_begin: { kind: "made", words: WORDS }, setup_save_file: { kind: "saved", path: "D:\\Backups\\Whimbrel recovery key.txt", sameDiskAsDatabase: false }, setup_verify: true, setup_saved_to: null };
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
  // Every stage is drawn at both sizes.
  for (const [name, stage] of LAUNCH)
    for (const size of SIZES)
      test(`launch ${name} ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        // "starting" shows its spinner only after a second.
        await open(page, mode, "main", launch(stage), name === "starting" ? 1700 : 900);
        await shot(page, `launch-${name}-${mode}-${size.name}`);
      });

  {
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
    const noStore = { launch_state: { stage: { kind: "openedWithoutStore" }, busy: false, waitingForUser: false, generation: 1 } };
    test(`app no-store banner ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", noStore);
      await settle(page, 400);
      await shot(page, `app-no-store-banner-${mode}-large`);
    });

    // The three first-run banners over the app (Arm notice, expired Google sign-in, the first retention notice).
    test(`first-run banners ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      const now = NOW;
      await open(page, mode, "main", {
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

    // ===== Home and the app shell, redesigned (screen 2). Every picture below is the new look. ====
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
        await open(page, mode, "main", {}, 1100, THREAD);
        await shot(page, `home-${mode}-${size.name}`, size === LARGE);
      });
      test(`home skin whimbrel ${mode} ${size.name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await open(page, mode, "main", {}, 1100);
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
        await open(page, mode, "main", {}, 1100, { id: `made-up-${key}-${value}`, palette: "thread", options });
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
        await open(page, mode, "main", {}, 1100, { id, palette: pal.id });
        await shot(page, `home-skin-${id}-${mode}-large`, true);
      });
    }

    // States.
    test(`home empty ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...EMPTY_DAY }, 1100, THREAD);
      await shot(page, `home-empty-${mode}-large`);
    });
    test(`home no calendar ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      const none = { accessIssues: [], blocked: null, emails: [], lastSyncedUnixMs: null, nextMeeting: null, nextStartUnixMs: null, signInExpired: [], syncError: null, upcoming: 0 };
      await open(page, mode, "main", { ...EMPTY_DAY, calendar_accounts: none }, 1100, THREAD);
      await shot(page, `home-no-calendar-${mode}-large`);
    });
    test(`home loading ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { today_overview: "__hang__" }, 700, THREAD);
      await shot(page, `home-loading-${mode}-large`);
    });
    test(`home error ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { today_overview: "__fail__:The memory database is busy." }, 900, THREAD);
      await shot(page, `home-error-${mode}-large`);
    });
    test(`home capture paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...paused, library_day_blocks: [] }, 1100, THREAD);
      await shot(page, `home-capture-paused-${mode}-large`);
    });
    test(`home capture off ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...off, library_day_blocks: [] }, 1100, THREAD);
      await shot(page, `home-capture-off-${mode}-large`);
    });
    test(`home recording ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...recordingPatch(NOW) }, 1300, THREAD);
      await shot(page, `home-recording-${mode}-large`, true);
    });
    test(`home processing ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { meeting_recording_status: { degraded: null, failure: null, meetingId: 1, processing: { label: "Writing the summary", meetingId: 1, stage: "summarizing" }, startedUnixMs: null, state: "idle" } }, 1300, THREAD);
      await shot(page, `home-processing-${mode}-large`);
    });
    test(`home zoom 200 ${mode}`, async ({ page }) => {
      await page.setViewportSize(SMALL);
      await open(page, mode, "main", {}, 1100, THREAD);
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(400);
      await shot(page, `home-zoom200-${mode}-small`, true);
    });

    // The main window at Windows text size 200%: about 550x360 CSS px (a 1100x720 window; the CSS viewport shrinks with the text size, the
    // root font stays 16 px), deviceScaleFactor 2. The top navigation shows, the header rows scroll away with the content, and one
    // compact row stays pinned with the recording state and the Memory state; the first content row is in the first screen.
    // Two cases: Today idle (nothing recording: the Memory state is pinned) and a recording (Home gives its place to the live screen;
    // the recording state and the Memory state are both pinned).
    for (const [kind, patch] of [["idle", {}], ["recording", livePatch(NOW, "recording")]] as [string, Record<string, unknown>][])
      test(`home text 200 short ${kind} ${mode}`, async ({ browser }) => {
        const context = await browser.newContext({ deviceScaleFactor: 2, viewport: { width: 550, height: 360 }, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
        const page = await context.newPage();
        const name = kind === "idle" ? "home-text200-short" : "home-text200-short-recording";
        const recording = kind === "recording";
        try {
          await open(page, mode, "main", { ...patch }, 1400, THREAD);
          await page.screenshot({ path: `${OUT}${name}-${mode}.png` });
          const probe = () =>
            page.evaluate(() => {
              const vh = window.innerHeight;
              const vw = document.documentElement.clientWidth;
              const seen = (el: Element | null) => {
                if (!el) return false;
                const r = el.getBoundingClientRect();
                if (r.width < 4 || r.height < 4 || r.left < -0.5 || r.top < -0.5 || r.right > vw + 0.5 || r.bottom > vh + 0.5) return false;
                const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
                return !!top && (el.contains(top) || top.contains(el));
              };
              const content = document.querySelector("#panel");
              const first = content ? Array.from(content.querySelectorAll<HTMLElement>("h2, h3, button, a, li, article, section, p")).find((e) => e.getBoundingClientRect().height > 8) : null;
              return {
                dpr: window.devicePixelRatio,
                recording: seen(document.querySelector(".w-shell-pinned .w-rec")),
                memory: seen(document.querySelector(".w-shell-pinned .w-capture-button")),
                firstTop: Math.round(first ? first.getBoundingClientRect().top : Infinity),
                wide: document.documentElement.scrollWidth > vw + 1,
                twice: Array.from(document.querySelectorAll(".w-rec-bar")).filter((e) => e.getBoundingClientRect().height > 0).length,
                vh,
              };
            });
          const at0 = await probe();
          if (at0.dpr !== 2) throw new Error(`${name}: devicePixelRatio is ${at0.dpr}`);
          if (recording && !at0.recording) throw new Error(`${name}: the recording state is not visible in the pinned row`);
          if (recording && at0.twice !== 1) throw new Error(`${name}: the recording bar is drawn ${at0.twice} times`);
          if (!at0.memory) throw new Error(`${name}: the Memory state is not visible in the pinned row`);
          if (at0.wide) throw new Error(`${name}: the page is wider than the window`);
          // The first content row has room in the first screen (at least 48 px of it is in view), not only its top edge.
          if (at0.firstTop > at0.vh - 48) throw new Error(`${name}: the first content row starts at ${at0.firstTop} of ${at0.vh}`);
          // Scrolled down, the pinned row is still there with both states.
          await page.evaluate(() => {
            for (const el of Array.from(document.querySelectorAll<HTMLElement>(".w-shell, .stage, .w-content, .th-live"))) el.scrollTop = el.scrollHeight;
          });
          await page.waitForTimeout(300);
          const down = await probe();
          if ((recording && !down.recording) || !down.memory) throw new Error(`${name}: scrolled down, the pinned row lost ${recording && !down.recording ? "the recording state " : ""}${down.memory ? "" : "the Memory state"}`);
          await page.screenshot({ path: `${OUT}${name}-scrolled-${mode}.png` });
        } finally {
          await context.close();
        }
      });

    // Interactions: the capture menu, the filter menus, a filtered search that lands in Library, Ctrl+K and Ctrl+2.
    test(`home capture menu ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", {}, 1100, THREAD);
      await page.locator(".w-capture-button").click();
      await settle(page, 300);
      await shot(page, `home-capture-menu-${mode}-large`);
    });
    test(`home capture menu paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...paused }, 1100, THREAD);
      await page.locator(".w-capture-button").click();
      await settle(page, 300);
      await shot(page, `home-capture-menu-paused-${mode}-large`);
    });
    test(`home filters ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", {}, 1100, THREAD);
      await page.getByRole("button", { name: "Apps", exact: true }).click();
      await settle(page, 300);
      await shot(page, `home-filter-apps-${mode}-large`);
      await page.getByRole("menuitemradio", { name: "Excel" }).click();
      await page.getByRole("button", { name: "Dates", exact: true }).click();
      await settle(page, 300);
      await shot(page, `home-filter-dates-${mode}-large`);
      await page.getByRole("menuitemradio", { name: "Last 7 days" }).click();
      await page.getByRole("searchbox", { name: "Search your work memory" }).fill("Northwind renewal");
      await settle(page, 300);
      await shot(page, `home-filter-chosen-${mode}-large`);
      await page.getByRole("button", { name: "Search", exact: true }).click();
      await settle(page, 900);
      await shot(page, `home-filter-library-${mode}-large`);
    });
    test(`shell shortcuts ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", {}, 1100, THREAD);
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
        await open(page, mode, "main", {}, 1100, THREAD);
        await tab(page, id);
        await settle(page, 700);
        await shot(page, `shell-${id}-${mode}-small`);
      });
    test(`shell recording meetings ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { ...recordingPatch(NOW) }, 1100, THREAD);
      await tab(page, "automations");
      await settle(page, 800);
      await shot(page, `shell-recording-automations-${mode}-large`);
    });
    // ===== The live meeting screen (screen 3). The flag is on; Home gives its place to the screen while a meeting records. ====
    const live = (kind: LiveKind = "recording", more: Record<string, unknown> = {}) => ({ ...livePatch(NOW, kind), ...more });
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
    for (const kind of ["recording", "paused"] as LiveKind[])
      test(`in-meeting window ${kind} ${mode}`, async ({ page }) => {
        await page.setViewportSize(MEETING_BAR);
        await open(page, mode, "meeting-bar", live(kind));
        await page.screenshot({ path: `${OUT}window-in-meeting-${kind}-${mode}.png` });
      });
    test(`still paused ${mode}`, async ({ page }) => {
      await page.setViewportSize(WIN["still-here"]);
      await open(page, mode, "still-here", live("paused"));
      await page.screenshot({ path: `${OUT}window-still-paused-${mode}.png` });
    });


    // ===== The menu and small windows, redesigned (screen 7). Each picture is at the size the shell opens the window. ====
    const popup = async (page: Page, patch: Record<string, unknown>, name: string, zoom = false) => {
      await page.setViewportSize(WIN.popup);
      await open(page, mode, "popup", { ...patch });
      await page.waitForTimeout(300);
      if (zoom) {
        await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
        await page.waitForTimeout(300);
      }
      await page.screenshot({ path: `${OUT}window-popup-${name}-${mode}.png` });
    };
    const CAP = (more: Record<string, unknown>) => ({ capture_status: { enabled: true, excluded: 41, lastApp: null, lastError: null, lastStoredAtMs: NOW - 4 * 60_000, ocrRuns: 120, paused: null, running: true, stored: 1482, unchanged: 9120, ...more } });
    test(`popup memory paused ${mode}`, async ({ page }) => popup(page, CAP({ paused: "capture is paused for a set time", resumeAtUnixMs: NOW + 48 * 60_000 }), "memory-paused"));
    test(`popup memory off ${mode}`, async ({ page }) => popup(page, CAP({ enabled: false }), "memory-off"));
    test(`popup recording state ${mode}`, async ({ page }) => popup(page, livePatch(NOW, "recording"), "recording"));
    test(`popup recording paused ${mode}`, async ({ page }) => popup(page, livePatch(NOW, "paused"), "recording-paused"));
    test(`popup recording problem ${mode}`, async ({ page }) => popup(page, livePatch(NOW, "problem"), "recording-problem"));
    test(`popup no library ${mode}`, async ({ page }) => popup(page, { launch_state: { stage: { kind: "openedWithoutStore" }, busy: false, waitingForUser: false, generation: 1 }, ...CAP({ running: false }) }, "no-library"));
    // Needs attention with the sync row: the tray's status says sync is in error (the same fact as the tooltip's "Sync error").
    test(`popup attention ${mode}`, async ({ page }) =>
      popup(page, { tray_status: { captureEnabled: true, line: "Capturing · Sync error", recordingPaused: false, recordingStartedUnixMs: null, syncError: true, syncText: "" } }, "attention"));
    // An expired Google sign-in comes from the tray's own status (no read on open); screen-text reading that is failing from the capture status.
    test(`popup google sign-in expired ${mode}`, async ({ page }) => popup(page, { tray_status: { captureEnabled: true, line: "Capturing · Synced 10:20 AM", recordingPaused: false, recordingStartedUnixMs: null, syncError: false, syncText: "Synced 10:20 AM", googleSignInExpired: true } }, "google-sign-in"));
    test(`popup memory read problem ${mode}`, async ({ page }) => popup(page, CAP({ ocrProblem: "unavailable", ocrProblemAtUnixMs: NOW - 90_000, lastError: "OCR unavailable: 0x80070490" }), "memory-read-problem"));
    test(`popup memory read failed ${mode}`, async ({ page }) => popup(page, CAP({ ocrProblem: "failed", ocrProblemAtUnixMs: NOW - 60_000, lastError: "OCR: 0x80004005" }), "memory-read-failed"));
    // Every popup state at Windows text size 200% is shot below (`200 popup ...`), with a check that the pinned parts are in view.

    // ---- Windows "Text size" 200%: every small window, with a check that nothing a person must reach is clipped ----------
    // Measured on a real PC (2026-11-23): WebView2 multiplies devicePixelRatio by the text scale; the root font stays 16 px and the CSS
    // viewport shrinks. It is not CSS zoom and not the root size. Two shapes are drawn, both at deviceScaleFactor 2, both named
    // `window-<name>-text200[-clamped]-<mode>.png`:
    //   text200          the design size in CSS px: what a window has once the shell multiplies its logical size by the text scale
    //                    (text_scale.rs), the normal case;
    //   text200-clamped  half the design size in CSS px: the shape of a window the shell could not grow (it bit the work area), the
    //                    harshest case; the UI's compact layout is the fallback for it.
    // Privacy: with auto-start on a recording begins by itself, so Record, Don't record (Not now) and the countdown must be in view at any
    // text size. The check looks at each action button, the countdown and the pinned popup parts: inside the window, and the
    // topmost thing at its centre (so a scroll container hiding it does not count as visible).
    const PINNED = ".w-win-actions button, .w-win-countdown, .w-win-countdown button, .w-pop-pinned button, .w-pop-pinned .w-mem-state, .w-pop-pinned .w-rec, .w-pop-foot button, .w-hk-input, .w-win-frame .w-win-name, .w-win-pinned, .mb-state .mb-word, .mb-time, .mb-buttons button";
    // `windowScrolls` is the popup's named fallback (the pinned parts and the body's minimum are taller than the window): the
    // whole window may scroll, but at the top the header, the recording row and Stop must be in view. Every other shot must
    // not scroll the window at all, so a normal case cannot slide into the fallback.
    const assertReachable = async (page: Page, name: string, windowScrolls = false) => {
      const bad = await page.evaluate(
        ({ sel, windowScrolls }) => {
          const out: string[] = [];
          const vw = document.documentElement.clientWidth;
          const vh = window.innerHeight;
          const seen = (el: Element) => {
            const r = el.getBoundingClientRect();
            const inside = r.left >= -0.5 && r.top >= -0.5 && r.right <= vw + 0.5 && r.bottom <= vh + 0.5;
            const top = document.elementFromPoint(Math.min(Math.max(r.left + r.width / 2, 0), vw - 1), Math.min(Math.max(r.top + r.height / 2, 0), vh - 1));
            return inside && !!top && (el.contains(top) || top.contains(el));
          };
          const label = (el: Element) => ((el.textContent || el.className) as string).trim().slice(0, 30);
          // Nothing wider than the viewport: the page does not scroll sideways and no window part is wider than its window.
          if (document.documentElement.scrollWidth > vw + 1) out.push(`the page is wider than the window (${document.documentElement.scrollWidth} > ${vw})`);
          for (const el of Array.from(document.querySelectorAll<HTMLElement>(".w-win, .w-win-scroll"))) if (el.scrollWidth > el.clientWidth + 1) out.push(`${el.className.split(" ")[0]} is wider than the window (${el.scrollWidth} > ${el.clientWidth})`);
          const main = document.querySelector(".w-win");
          const scrolls = !!main && main.scrollHeight > main.clientHeight + 1;
          if (scrolls && !windowScrolls && document.querySelector(".w-win-frame, .w-pop")) out.push(`the whole window scrolls (${main!.scrollHeight} > ${main!.clientHeight})`);
          // At the top: the pinned parts are in view (in the fallback, only the header, the recording row and Stop must be).
          // A frame window in the fallback (the clamped shape, half the design space, where the pinned parts cannot all fit): the product
          // name is in view at the top, and every other pinned part is reached by scrolling the window to it.
          const frame = !!document.querySelector(".w-win-frame");
          const pinned = windowScrolls ? (frame ? ".w-win-frame .w-win-name" : ".w-win-head, .w-pop-pinned .w-rec, .w-pop-pinned .w-recrow button") : sel;
          for (const el of Array.from(document.querySelectorAll<HTMLElement>(pinned))) {
            const r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) continue;
            if (!seen(el)) out.push(`${label(el)} [${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}] in ${vw}x${vh}`);
          }
          if (windowScrolls && frame) {
            for (const el of Array.from(document.querySelectorAll<HTMLElement>(sel))) {
              if (el.getBoundingClientRect().height === 0) continue;
              el.scrollIntoView({ block: "nearest" });
              if (!seen(el)) out.push(`cannot scroll to ${label(el)}`);
            }
          }
          // Every section of the popup, and its footer, can be reached by scrolling (the body first, then the window).
          for (const el of Array.from(document.querySelectorAll<HTMLElement>(".w-pop .w-menu-section, .w-pop .w-pop-body .w-mem, .w-pop-foot"))) {
            if (el.getBoundingClientRect().height === 0) continue;
            // Brought to the top of the view (the body first, then the window): its first line must then be the topmost thing at
            // its place, not clipped by the body. A section taller than the body is read by scrolling on from there.
            el.scrollIntoView({ block: "start" });
            const r = el.getBoundingClientRect();
            const probe = document.elementFromPoint(Math.min(Math.max(r.left + r.width / 2, 0), vw - 1), Math.min(Math.max(r.top + 4, 0), vh - 1));
            if (!probe || !(el.contains(probe) || probe.contains(el)) || r.top < -0.5 || r.top > vh) out.push(`cannot scroll to ${label(el)}`);
          }
          for (const el of Array.from(document.querySelectorAll<HTMLElement>(".w-win, .w-win-scroll"))) el.scrollTop = 0;
          return out;
        },
        { sel: PINNED, windowScrolls },
      );
      if (bad.length) throw new Error(`${name} at 200%: not reachable without scrolling: ${bad.join("; ")}`);
    };
    /** One small window drawn in one of the two text-200% shapes (see above), at deviceScaleFactor 2, in its own context. */
    const text200 = async (browser: Browser, label: string, patch: Record<string, unknown>, name: string, clamped: boolean, wait: number, windowScrolls: boolean) => {
      const design = WIN[label];
      const viewport = clamped ? { width: Math.round(design.width / 2), height: Math.round(design.height / 2) } : design;
      const context = await browser.newContext({ deviceScaleFactor: 2, viewport, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
      const page = await context.newPage();
      const shape = clamped ? "text200-clamped" : "text200";
      try {
        await open(page, mode, label, { ...patch });
        await page.waitForTimeout(wait);
        await page.screenshot({ path: `${OUT}window-${name}-${shape}-${mode}.png` });
        const dpr = await page.evaluate(() => window.devicePixelRatio);
        if (dpr !== 2) throw new Error(`${name} ${shape}: devicePixelRatio is ${dpr}, not 2`);
        await assertReachable(page, `${name} ${shape}`, windowScrolls);
        // The product name is drawn, not only spoken, in every frame window (they have no title bar), and the meeting alert's
        // sentence is pinned outside the scroller. `assertReachable` skips a zero-size element, so presence and size are checked here.
        // The compact layout (the mark and name inline with the title) is chosen from the CSS space the window has: on in the clamped
        // shape, off at the design size.
        const missing = await page.evaluate((clamped) => {
          const out: string[] = [];
          // The in-meeting window has its own layout: the recording state, Pause or Resume and Stop are always drawn, whatever its size.
          const bar = document.querySelector(".mb");
          if (bar) {
            const shown = (el: Element | null) => !!el && el.getBoundingClientRect().width > 4 && el.getBoundingClientRect().height > 4;
            if (!shown(bar.querySelector(".mb-word"))) out.push("the recording state word is not visible");
            if (!shown(bar.querySelector(".mb-time"))) out.push("the recorded time is not visible");
            const labels = Array.from(bar.querySelectorAll(".mb-buttons button")).map((b) => (b.textContent || "").trim());
            if (labels.length !== 2 || !(labels[0] === "Pause" || labels[0] === "Resume") || labels[1] !== "Stop") out.push(`the buttons read ${JSON.stringify(labels)}`);
            for (const b of Array.from(bar.querySelectorAll(".mb-buttons button"))) if (!shown(b)) out.push("a button is not visible");
            if (bar.classList.contains("mb--compact") !== clamped) out.push(clamped ? "the compact layout is not on although the window has half its design space" : "the compact layout is on although the window has its design space");
            return out;
          }
          if (!document.querySelector(".w-win-frame")) return out;
          const vis = (el: Element | null) => !!el && el.getBoundingClientRect().width > 4 && el.getBoundingClientRect().height > 4 && getComputedStyle(el).visibility !== "hidden";
          const name = document.querySelector(".w-win-frame .w-win-name");
          if (!vis(name) || name!.textContent !== "Whimbrel") out.push("the product name is not visible");
          if (name && name.closest(".w-visually-hidden")) out.push("the product name is visually hidden");
          const pinned = document.querySelector(".w-win-pinned");
          if (pinned && (!vis(pinned) || pinned.closest(".w-win-scroll"))) out.push("the pinned sentence is hidden or inside the scroller");
          const inline = !!document.querySelector(".w-win-top--inline");
          if (inline !== clamped) out.push(clamped ? "the compact layout is not on although the window has half its design space" : "the compact layout is on although the window has its design space");
          return out;
        }, clamped);
        if (missing.length) throw new Error(`${name} ${shape}: ${missing.join("; ")}`);
        if (label === "alert") {
          const sentence = await page.evaluate(() => document.querySelector(".w-win-pinned")?.textContent ?? "");
          if (sentence !== "Record it? Audio stays on this PC.") throw new Error(`${name} ${shape}: the pinned sentence reads "${sentence}"`);
        }
      } finally {
        await context.close();
      }
    };
    /** Both shapes. At the design size nothing scrolls the window and every pinned part is in view. Clamped (half the space, which no
     *  real screen leaves a window), the whole window may scroll: the product name is in view at the top and every pinned part is
     *  reached by scrolling to it (the popup keeps its stricter named fallback: header, recording row and Stop in view). */
    const at200 = async (browser: Browser, label: string, patch: Record<string, unknown>, name: string, wait = 300) => {
      await text200(browser, label, patch, name, false, wait, false);
      await text200(browser, label, patch, name, true, wait, true);
    };
    const ALERT = { appId: "zoom", attendees: ["Sam Okafor", "Priya Shah"], id: "det-1", promptedUnixMs: NOW, rootPid: 4242, source: "Zoom", startUnixMs: NOW, title: "Northwind security review" };
    const COUNT = { alert_status: { autoStart: true, countdownSeconds: 8, meeting: ALERT } };
    test(`200 alert ${mode}`, ({ browser }) => at200(browser, "alert", {}, "alert"));
    test(`200 alert countdown ${mode}`, ({ browser }) => at200(browser, "alert", COUNT, "alert-countdown"));
    test(`200 alert long title ${mode}`, ({ browser }) => at200(browser, "alert", { alert_status: { autoStart: true, countdownSeconds: 8, meeting: { ...ALERT, title: "Quarterly planning and budget review with the whole leadership team and partners" } } }, "alert-long-title"));
    test(`200 question ${mode}`, ({ browser }) => at200(browser, "question-alert", {}, "question"));
    test(`200 budget ${mode}`, ({ browser }) => at200(browser, "budget-alert", {}, "budget"));
    test(`200 disk ${mode}`, ({ browser }) => at200(browser, "disk-alert", {}, "disk"));
    // The in-meeting window grows with the text size (meeting_bar.rs): the design size in CSS px, and half of it when the work area capped
    // it. Its state, the time, Pause or Resume and Stop are in view in both shapes; the window itself never scrolls.
    for (const kind of ["recording", "paused"] as LiveKind[]) {
      const name = kind === "recording" ? "in-meeting" : "in-meeting-paused";
      test(`200 in-meeting ${kind} ${mode}`, async ({ browser }) => {
        await text200(browser, "meeting-bar", livePatch(NOW, kind), name, false, 300, false);
        await text200(browser, "meeting-bar", livePatch(NOW, kind), name, true, 300, false);
      });
    }
    test(`200 still normal ${mode}`, ({ browser }) => at200(browser, "still-here", livePatch(NOW, "recording"), "still-normal"));
    test(`200 still paused ${mode}`, ({ browser }) => at200(browser, "still-here", livePatch(NOW, "paused"), "still-paused"));
    const CAP200 = (more: Record<string, unknown>) => ({ capture_status: { enabled: true, excluded: 41, lastApp: null, lastError: null, lastStoredAtMs: NOW - 4 * 60_000, ocrRuns: 120, paused: null, running: true, stored: 1482, unchanged: 9120, ...more } });
    const POPUPS: [string, Record<string, unknown>][] = [
      ["idle", {}],
      ["memory-paused", CAP200({ paused: "capture is paused for a set time", resumeAtUnixMs: NOW + 48 * 60_000 })],
      ["memory-paused-tomorrow", CAP200({ paused: "capture is paused for a set time", resumeAtUnixMs: NOW + 17.6 * 3_600_000 })],
      ["memory-paused-by-app", CAP200({ paused: "the screen is locked" })],
      ["memory-off", CAP200({ enabled: false })],
      // The two ways memory is really stopped (capture/service.rs): UI Automation would not start, or there is no service at all.
      ["memory-stopped", CAP200({ running: false, lastError: "UI Automation: 0x80070005" })],
      ["memory-stopped-unknown", CAP200({ running: false, lastError: "capture did not start; see About for the startup error" })],
      ["recording", livePatch(NOW, "recording")],
      ["recording-paused", livePatch(NOW, "paused")],
      ["recording-problem", livePatch(NOW, "problem")],
      ["recording-paused-problem", (() => { const p = livePatch(NOW, "paused"); return { ...p, meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: HEADSET } }; })()],
      // A stream that stopped delivering before the pause is not a problem while paused: the row says paused, nothing more.
      ["recording-paused-stalled", (() => { const p = livePatch(NOW, "paused"); return { ...p, meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: MIC_STALLED } }; })()],
      ["no-library", { launch_state: { stage: { kind: "openedWithoutStore" }, busy: false, waitingForUser: false, generation: 1 }, ...CAP200({ running: false }) }],
      ["google-sign-in", { tray_status: { captureEnabled: true, line: "Capturing · Synced 10:20 AM", recordingPaused: false, recordingStartedUnixMs: null, syncError: false, syncText: "Synced 10:20 AM", googleSignInExpired: true } }],
      ["memory-read-problem", CAP200({ ocrProblem: "unavailable", ocrProblemAtUnixMs: NOW - 90_000, lastError: "OCR unavailable: 0x80070490" })],
      ["sync-error", { tray_status: { captureEnabled: true, line: "Capturing · Sync error", recordingPaused: false, recordingStartedUnixMs: null, syncError: true, syncText: "" } }],
    ];
    for (const [name, patch] of POPUPS) test(`200 popup ${name} ${mode}`, ({ browser }) => at200(browser, "popup", patch, `popup-${name}`));
    // The worst case at 200%: a paused recording with a problem, memory paused until tomorrow (Resume now and Turn off), sync in
    // error and a next meeting. The pinned parts and the body's minimum are taller than the window, so the whole window scrolls:
    // the header, the recording row and Stop are in view at the top, and every section is reached by scrolling.
    test(`200 popup worst case ${mode}`, ({ browser }) => {
      const p = livePatch(NOW, "paused");
      const patch = {
        ...p,
        meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: HEADSET },
        ...CAP200({ paused: "capture is paused for a set time", resumeAtUnixMs: NOW + 17.6 * 3_600_000 }),
        tray_status: { captureEnabled: true, line: "Capturing · Sync error", recordingPaused: true, recordingStartedUnixMs: NOW - 12 * 60_000, syncError: true, syncText: "" },
      };
      return at200(browser, "popup", patch, "popup-worst-case", 300);
    });
    // Both status reads fail at 200%: Stop and Try again are still in view.
    test(`200 popup unreadable ${mode}`, ({ browser }) => at200(browser, "popup", { meeting_recording_status: "__fail__:no", tray_status: "__fail__:no" }, "popup-unreadable"));
    // The real clamped popup at text 200% on a PC (2026-11-23): 320x448 CSS px at deviceScaleFactor 2, the compact layout. The reason of
    // a recording problem is directly under "Recording problem", above the Memory section's box, never below the Memory controls.
    for (const [kind, patch] of [
      ["recording-problem", livePatch(NOW, "problem")],
      ["recording-paused-problem", (() => { const p = livePatch(NOW, "paused"); return { ...p, meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: HEADSET } }; })()],
    ] as [string, Record<string, unknown>][]) {
      for (const memory of ["on", "off"]) {
        test(`200 popup real clamped ${kind} memory ${memory} ${mode}`, async ({ browser }) => {
          const context = await browser.newContext({ deviceScaleFactor: 2, viewport: { width: 320, height: 448 }, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
          const page = await context.newPage();
          try {
            await open(page, mode, "popup", { ...patch, ...(memory === "off" ? CAP200({ enabled: false }) : {}) });
            await page.waitForTimeout(400);
            await page.screenshot({ path: `${OUT}window-popup-${kind}-memory-${memory}-text200-real-${mode}.png` });
            const r = await page.evaluate(() => {
              const box = (el: Element | null) => (el ? el.getBoundingClientRect() : null);
              const reason = document.querySelector(".w-pop-aside-reason, .w-recrow-problem-reason");
              const heads = Array.from(document.querySelectorAll(".w-menu-label"));
              const memoryHead = heads.find((h) => h.textContent === "Memory") ?? null;
              const memorySection = memoryHead ? memoryHead.closest("section") : null;
              const label = reason ? (reason.closest("section")?.querySelector("h2")?.textContent ?? reason.closest(".w-recrow")?.querySelector("strong")?.textContent ?? null) : null;
              const rb = box(reason);
              const mb = box(memorySection);
              const row = box(document.querySelector(".w-recrow"));
              const stops = Array.from(document.querySelectorAll("button")).filter((b) => b.textContent?.trim() === "Stop");
              // Whatever the reason sits in, no Memory control may come before it in the document.
              const before = reason && memorySection ? !!(memorySection.compareDocumentPosition(reason) & Node.DOCUMENT_POSITION_FOLLOWING) : false;
              return { dpr: window.devicePixelRatio, w: window.innerWidth, h: window.innerHeight, hasReason: !!reason, text: reason?.textContent ?? "", label, rb: rb && { top: rb.top, bottom: rb.bottom }, mb: mb && { top: mb.top }, row: row && { bottom: row.bottom }, stops: stops.length, memoryBeforeReason: before };
            });
            const fail: string[] = [];
            if (r.dpr !== 2 || r.w !== 320 || r.h !== 448) fail.push(`shape ${r.w}x${r.h} at ${r.dpr}`);
            if (!r.hasReason || !r.text) fail.push("the reason is not drawn");
            if (r.label !== "Recording problem") fail.push(`the reason sits under the label ${JSON.stringify(r.label)}, not "Recording problem"`);
            if (r.rb && r.mb && !(r.rb.bottom <= r.mb.top + 0.5)) fail.push(`the reason's box (bottom ${r.rb.bottom}) is not above the Memory section (top ${r.mb.top})`);
            if (r.memoryBeforeReason) fail.push("the Memory section comes before the reason in the document");
            if (r.stops !== 1) fail.push(`${r.stops} Stop buttons`);
            if (fail.length) throw new Error(`${kind} memory ${memory}: ${fail.join("; ")}`);
          } finally {
            await context.close();
          }
        });
      }
    }
    // A focused scrolling body scrolls with the arrow keys (the keys must not move focus off it).
    test(`200 popup scroll keys ${mode}`, async ({ browser }) => {
      // In the clamped shape (half the design size in CSS px, deviceScaleFactor 2) the popup's body scrolls.
      const context = await browser.newContext({ deviceScaleFactor: 2, viewport: { width: WIN.popup.width / 2, height: WIN.popup.height / 2 }, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
      const page = await context.newPage();
      try {
        await open(page, mode, "popup", { ...livePatch(NOW, "paused") });
        await page.waitForTimeout(500);
        // The body is focusable only while it has more below it (`tabIndex` goes at the end), so it is found by its class.
        const box = page.locator(".w-pop-body .w-win-scroll").first();
        await box.focus();
        for (const [key, check] of [["ArrowDown", "gt"], ["PageDown", "gt"], ["End", "end"], ["ArrowUp", "lt"], ["Home", "top"]] as const) {
          const before = await box.evaluate((el) => el.scrollTop);
          await page.keyboard.press(key);
          await page.waitForTimeout(500);
          const after = await box.evaluate((el) => ({ top: el.scrollTop, max: el.scrollHeight - el.clientHeight, focused: document.activeElement === el }));
          if (!after.focused) throw new Error(`${key} moved focus off the scrolling body`);
          const ok = check === "gt" ? after.top > before : check === "lt" ? after.top < before : check === "end" ? after.top >= after.max - 1 : after.top === 0;
          if (!ok) throw new Error(`${key}: scrollTop went ${before} -> ${after.top} (max ${after.max})`);
        }
      } finally {
        await context.close();
      }
    });
    test(`200 hotkey ready ${mode}`, ({ browser }) => at200(browser, "hotkey-panel", {}, "hotkey-ready"));
    test(`200 hotkey blocked ${mode}`, ({ browser }) =>
      at200(browser, "hotkey-panel", { hotkey_panel_read: { requestId: 1, window: { kind: "read", appName: "Mail", title: "Re: Q3 rollout", isSelection: true, truncated: false }, destination: { kind: "local" }, canAnswer: false, cannotAnswer: { kind: "local_model_missing" } } }, "hotkey-blocked"));
    test(`200 hotkey reading ${mode}`, ({ browser }) => at200(browser, "hotkey-panel", { hotkey_panel_read: "__hang__" }, "hotkey-reading"));

    // ---- the popup when a read is late or fails (100% text) -------------------------------------------------------------------
    // The detailed recording status never arrives, but the tray knows a recording runs: the row shows from the tray's status.
    test(`popup recording from tray ${mode}`, ({ page }) =>
      popup(page, { meeting_recording_status: "__hang__", tray_status: { captureEnabled: true, line: "Capturing · Synced 10:20 AM", recordingPaused: false, recordingStartedUnixMs: NOW - 14 * 60_000, syncError: false, syncText: "Synced 10:20 AM" } }, "recording-from-tray"));
    test(`popup recording unreadable ${mode}`, ({ page }) => popup(page, { meeting_recording_status: "__fail__:no", tray_status: "__fail__:no" }, "recording-unreadable"));
    test(`popup memory stopped ${mode}`, ({ page }) => popup(page, CAP({ running: false, lastError: "UI Automation: 0x80070005" }), "memory-stopped"));
    test(`popup memory stopped unknown ${mode}`, ({ page }) => popup(page, CAP({ running: false, lastError: "capture did not start; see About for the startup error" }), "memory-stopped-unknown"));
    test(`popup memory paused by app ${mode}`, ({ page }) => popup(page, CAP({ paused: "the screen is locked" }), "memory-paused-by-app"));
    test(`popup recording paused problem ${mode}`, ({ page }) => {
      const p = livePatch(NOW, "paused");
      return popup(page, { ...p, meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: HEADSET } }, "recording-paused-problem");
    });
    test(`popup recording paused stalled ${mode}`, ({ page }) => {
      const p = livePatch(NOW, "paused");
      return popup(page, { ...p, meeting_recording_status: { ...(p.meeting_recording_status as object), degraded: MIC_STALLED } }, "recording-paused-stalled");
    });

    test(`alert countdown ${mode}`, async ({ page }) => {
      await page.setViewportSize(WIN.alert);
      await open(page, mode, "alert", { alert_status: { autoStart: true, countdownSeconds: 8, meeting: { appId: "zoom", attendees: ["Sam Okafor", "Priya Shah"], id: "det-1", promptedUnixMs: NOW, rootPid: 4242, source: "Zoom", startUnixMs: NOW, title: "Northwind security review" } } });
      await page.screenshot({ path: `${OUT}window-alert-countdown-${mode}.png` });
    });
    test(`still here normal ${mode}`, async ({ page }) => {
      await page.setViewportSize(WIN["still-here"]);
      await open(page, mode, "still-here", live("recording"));
      await page.screenshot({ path: `${OUT}window-still-normal-${mode}.png` });
    });

    // The hotkey panel: reading, blocked, and an ask typed and answered (window_ask is scripted like Find's ask).
    const hotkey = async (page: Page, patch: Record<string, unknown>, name: string, ask?: { script: { at: number; event: Record<string, unknown> }[]; wait: number }) => {
      await page.setViewportSize(WIN["hotkey-panel"]);
      await open(page, mode, "hotkey-panel", { ...patch, ...(ask ? { __ask_script: ask.script } : {}) });
      if (ask) {
        await page.getByRole("textbox", { name: "Ask about this window" }).fill("What does Sam need from me?");
        await page.keyboard.press("Enter");
        await page.waitForTimeout(ask.wait);
      }
      await page.screenshot({ path: `${OUT}window-hotkey-panel-${name}-${mode}.png` });
    };
    const HOTKEY_ANSWER = "Sam wants the usage tier numbers before Thursday. Reply with the draft rates, and say the annual discount is still open.";
    const deltas = (upTo: number) => HOTKEY_ANSWER.split(" ").slice(0, upTo).map((w, i) => ({ at: 80 + i * 20, event: { kind: "delta", text: (i === 0 ? "" : " ") + w } }));
    test(`hotkey panel reading ${mode}`, async ({ page }) => hotkey(page, { hotkey_panel_read: "__hang__" }, "reading"));
    test(`hotkey panel ready ${mode}`, async ({ page }) => hotkey(page, {}, "ready"));
    test(`hotkey panel blocked ${mode}`, async ({ page }) =>
      hotkey(page, { hotkey_panel_read: { requestId: 1, window: { kind: "read", appName: "Mail", title: "Re: Q3 rollout", isSelection: true, truncated: false }, destination: { kind: "local" }, canAnswer: false, cannotAnswer: { kind: "local_model_missing" } } }, "blocked"));
    test(`hotkey panel answering ${mode}`, async ({ page }) => hotkey(page, {}, "answering", { script: [...deltas(9), { at: 60_000, event: { kind: "completed", answer: HOTKEY_ANSWER } }], wait: 700 }));
    test(`hotkey panel answered ${mode}`, async ({ page }) =>
      hotkey(page, {}, "answered", { script: [...deltas(22), { at: 80 + 22 * 20 + 40, event: { kind: "completed", answer: HOTKEY_ANSWER } }], wait: 1400 }));
    test(`hotkey panel failed ${mode}`, async ({ page }) =>
      hotkey(page, {}, "failed", { script: [{ at: 60, event: { kind: "failed", error: { kind: "provider_failure", reason: { kind: "local_model_missing" } }, partial: false } }], wait: 700 }));

    // ===== The finished note (screen 4): the Northwind renewal call, made up. The flag is on; the Meetings tab opens it. ====
    const note = (kind: NoteKind = "withSources", more: Record<string, unknown> = {}) => ({ ...notePatch(NOW, kind), ...more });
    const openNote = async (page: Page, kind: NoteKind = "withSources", size = LARGE, skin: Skin = THREAD, more: Record<string, unknown> = {}) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", note(kind, more), 1200, skin);
      await tab(page, "meetings");
      await settle(page, 1400);
    };
    const zoom200 = async (page: Page) => {
      await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
      await page.waitForTimeout(400);
    };
    for (const size of SIZES)
      test(`note sources ${mode} ${size.name}`, async ({ page }) => {
        await openNote(page, "withSources", size);
        // Source 3 selected: its card is lit in the margin, or under its line when the margin is folded.
        await page.getByRole("button", { name: /^Source 3,/ }).click();
        await settle(page, 300);
        await shot(page, `note-sources-${mode}-${size.name}`, size === LARGE);
      });
    test(`note no sources ${mode}`, async ({ page }) => {
      await openNote(page, "noSources");
      await shot(page, `note-no-sources-${mode}-large`, true);
    });
    test(`note zoom 200 ${mode}`, async ({ page }) => {
      await openNote(page, "withSources", SMALL);
      await page.getByRole("button", { name: /^Source 3,/ }).click();
      await zoom200(page);
      await shot(page, `note-zoom200-${mode}-small`, true);
    });
    // The live screen and the finished note at Windows text size 200% on a PC (main window 550x360 CSS px, deviceScaleFactor 2; the root font
    // stays 16 px). The header's actions (Edit, Transcript, More; Mark this moment, Pause, Stop) must not be clipped or scroll the page
    // sideways. Each action is visible, or inside the opened More menu. Pause/Resume and Stop of the live screen are never in a menu.
    const HEADER_PROBE = (kind: "live" | "note") => {
      const vw = document.documentElement.clientWidth;
      const header = kind === "note" ? document.querySelector<HTMLElement>(".th-fin-bar") : document.querySelector<HTMLElement>(".th-note-head");
      const bar = kind === "live" ? document.querySelector<HTMLElement>(".w-shell-pinned") : null;
      const wide: string[] = [];
      for (const root of [header, bar]) {
        if (!root) continue;
        for (const el of [root, ...Array.from(root.querySelectorAll<HTMLElement>("*"))]) {
          const r = el.getBoundingClientRect();
          if (r.width < 1 || r.height < 1) continue;
          if (r.width > vw + 0.5 || r.right > vw + 0.5 || r.left < -0.5) wide.push(`${el.className || el.tagName}:${Math.round(r.left)}-${Math.round(r.right)} of ${vw}`);
          if (el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1) wide.push(`${el.className || el.tagName} clips (scrollWidth ${el.scrollWidth} > ${el.clientWidth})`);
        }
      }
      const buttons = Array.from((header ?? document.body).querySelectorAll<HTMLElement>("button, a")).map((b) => (b.getAttribute("aria-label") || b.textContent || "").trim());
      const visible = (b: HTMLElement) => { const r = b.getBoundingClientRect(); return r.width > 4 && r.height > 4 && r.left >= -0.5 && r.right <= vw + 0.5; };
      const names = Array.from((header ?? document.body).querySelectorAll<HTMLElement>("button")).filter(visible).map((b) => (b.getAttribute("aria-label") || b.textContent || "").trim());
      return { dpr: window.devicePixelRatio, vw, vh: window.innerHeight, pageWide: document.documentElement.scrollWidth > vw + 1, wide, buttons, names };
    };
    const probeHeader = async (page: Page, kind: "live" | "note") => page.evaluate(HEADER_PROBE, kind);
    for (const [id, which, width] of [["live", "live", 550], ["live-paused", "live", 550], ["note", "note", 550], ["note-read-only", "note", 550], ["note-narrow", "note", 380], ["note-transcript", "note", 550], ["live-narrow", "live", 380], ["note-retro", "note", 550], ["live-retro", "live", 550]] as [string, "live" | "note", number][])
      test(`text 200 header ${id} ${mode}`, async ({ browser }) => {
        test.skip(id.endsWith("retro") && mode === "light", "the retro-terminal skin is a dark skin");
        const context = await browser.newContext({ deviceScaleFactor: 2, viewport: { width, height: 360 }, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
        const page = await context.newPage();
        try {
          const patch = id.startsWith("live-p") ? live("paused") : which === "live" ? live("recording") : id === "note-read-only" ? note("readOnly") : note("withSources");
          await open(page, mode, "main", patch, 1500, id.endsWith("retro") ? ({ id: "retro-terminal", palette: "phosphor" } as Skin) : THREAD);
          if (which === "note") {
            await tab(page, "meetings");
            await settle(page, 1400);
          }
          if (id === "note-transcript") {
            await page.getByRole("button", { name: /^Transcript/ }).click();
            await settle(page, 800);
          }
          await page.screenshot({ path: `${OUT}${id}-text200-header-${mode}.png` });
          const r = await probeHeader(page, which);
          const fail: string[] = [];
          if (r.dpr !== 2 || r.vw > width + 10) fail.push(`shape ${r.vw}x${r.vh} at ${r.dpr}`);
          if (r.pageWide) fail.push("the page scrolls sideways");
          if (r.wide.length) fail.push(`wider than the viewport or clipped: ${r.wide.slice(0, 4).join(" | ")}`);
          if (which === "note") {
            // Every action is visible, or inside the opened More menu.
            const want = ["Note", "Transcript", "More"];
            for (const w of want) if (!r.names.some((n) => n.startsWith(w))) fail.push(`${w} is not visible`);
            const more = page.getByRole("button", { name: "More", exact: true });
            await more.click();
            await settle(page, 300);
            await page.screenshot({ path: `${OUT}${id}-text200-header-more-${mode}.png` });
            const menu = await page.evaluate(() => {
              const m = document.querySelector<HTMLElement>(".th-fin-menu");
              const r = m ? m.getBoundingClientRect() : null;
              return { items: Array.from(document.querySelectorAll(".th-fin-menu [role^=menuitem]")).map((i) => (i.textContent || "").trim()), inside: !!r && r.left >= -0.5 && r.right <= document.documentElement.clientWidth + 0.5 };
            });
            if (!menu.inside) fail.push("the opened More menu is clipped by the window");
            const shown = (label: string) => r.names.some((n) => n.startsWith(label)) || menu.items.some((n) => n.startsWith(label));
            const edits = id === "note";
            for (const need of edits ? ["Edit", "Transcript", "More"] : ["Transcript", "More"]) if (!shown(need)) fail.push(`${need} is neither visible nor in the More menu`);
            r.names.push(...menu.items.map((n) => `[menu] ${n}`));
          } else {
            for (const need of ["Mark this moment"]) if (!r.names.some((n) => n.startsWith(need))) fail.push(`${need} is not visible`);
            const pinned = await page.evaluate(() => Array.from(document.querySelectorAll(".w-shell-pinned button")).map((b) => (b.textContent || "").trim()));
            const stop = id === "live-paused" ? ["Resume", "Stop"] : ["Pause", "Stop"];
            const all = await page.evaluate(() => Array.from(document.querySelectorAll("button")).filter((b) => { const q = b.getBoundingClientRect(); return q.width > 4 && !b.closest("[role=menu]"); }).map((b) => (b.textContent || "").trim()));
            for (const s of stop) if (!all.some((n) => n.startsWith(s)) && !pinned.some((n) => n.startsWith(s))) fail.push(`${s} is not drawn`);
          }
          console.log(`HEADER ${id} ${mode}: ${JSON.stringify(r.names)}`);
          if (fail.length) throw new Error(`${id} header at 200%: ${fail.join("; ")}`);
        } finally {
          await context.close();
        }
      });
    for (const [name, kind] of [["processing", "processing"], ["read-only", "readOnly"], ["no-summary", "noSummary"], ["failed", "failed"]] as [string, NoteKind][])
      test(`note ${name} ${mode}`, async ({ page }) => {
        await openNote(page, kind);
        await shot(page, `note-${name}-${mode}-large`, true);
      });
    test(`note edit ${mode}`, async ({ page }) => {
      await openNote(page);
      await page.getByRole("button", { name: "Edit", exact: true }).click();
      await settle(page, 400);
      await shot(page, `note-edit-${mode}-large`, true);
    });
    test(`note edit failed ${mode}`, async ({ page }) => {
      // No summary was written: Edit still opens the editor, and typing the first words keeps focus in it.
      await openNote(page, "failed");
      await page.getByRole("button", { name: "Edit", exact: true }).click();
      const box = page.getByRole("textbox", { name: "Summary", exact: true });
      await box.click();
      await page.keyboard.type("Northwind will renew for two years.");
      await settle(page, 400);
      if (!(await box.evaluate((el) => el === document.activeElement))) throw new Error("the summary editor lost focus while typing");
      await shot(page, `note-edit-failed-${mode}-large`, true);
    });
    test(`note transcript processing ${mode}`, async ({ page }) => {
      // A run in progress shows its stage above the transcript view too.
      await openNote(page, "processing");
      await page.getByRole("button", { name: /^Transcript/ }).click();
      await settle(page, 700);
      await shot(page, `note-transcript-processing-${mode}-large`, true);
    });
    test(`note menus ${mode}`, async ({ page }) => {
      await openNote(page);
      await page.getByRole("button", { name: "More", exact: true }).click();
      await settle(page, 300);
      await shot(page, `note-more-menu-${mode}-large`);
    });
    for (const size of SIZES)
      test(`note transcript ${mode} ${size.name}`, async ({ page }) => {
        await openNote(page, "withSources", size);
        // Opened from source 3: the line at 18:25 is highlighted and the lanes show the speakers.
        await page.getByRole("button", { name: /^Source 3,/ }).click();
        await page.getByRole("button", { name: "Open in transcript at 18:25" }).first().click({ force: true });
        await settle(page, 900);
        await shot(page, `note-transcript-${mode}-${size.name}`, size === LARGE);
      });
    test(`note transcript old meeting ${mode}`, async ({ page }) => {
      await openNote(page, "noSources");
      await page.getByRole("button", { name: /^Transcript/ }).click();
      await settle(page, 700);
      await shot(page, `note-transcript-no-ends-${mode}-large`, true);
    });
    test(`note from library ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", note(), 1200, THREAD);
      await tab(page, "library");
      await settle(page, 700);
      // Library opens on Find; the meeting is a row of Recent.
      await page.getByRole("radio", { name: "Recent", exact: true }).click();
      await settle(page, 600);
      await page.getByRole("button", { name: /^Open Northwind renewal check-in/ }).first().click();
      await settle(page, 1400);
      await shot(page, `note-from-library-${mode}-large`, true);
    });
    test(`note skin whimbrel ${mode}`, async ({ page }) => {
      await openNote(page, "withSources", LARGE, WHIMBREL);
      await page.getByRole("button", { name: /^Source 3,/ }).click();
      await shot(page, `note-skin-whimbrel-${mode}-large`, true);
    });

    // ===== Find, and the Ask with numbered sources (screen 5): the Library tab opens on it. The flag is on. ====
    const QUESTION = "What did Sam say about pricing last week?";
    const find = (kind: FindKind = "default", script?: AskScript, more: Record<string, unknown> = {}) => ({ ...findPatch(NOW, kind), ...(script ? { __ask_script: askScript(script) } : {}), ...more });
    const openFind = async (page: Page, patch: Record<string, unknown>, size = LARGE) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", patch, 1200, THREAD);
      await tab(page, "library");
      await settle(page, 900);
    };
    const field = (page: Page) => page.getByRole("searchbox", { name: "Search your work memory" });
    const searchFor = async (page: Page, text: string) => {
      await field(page).fill(text);
      await page.keyboard.press("Enter");
      await settle(page, 900);
    };
    const askFor = async (page: Page, text: string, wait = 3200) => {
      await field(page).fill(text);
      await page.keyboard.press("Control+Enter");
      await settle(page, wait);
    };
    const chip = async (page: Page, chipName: string, choice: string) => {
      await page.getByRole("button", { name: chipName, exact: true }).click();
      await page.getByRole("menuitemradio", { name: choice }).click();
      await settle(page, 400);
    };
    for (const size of SIZES) {
      test(`find empty ${mode} ${size.name}`, async ({ page }) => {
        await openFind(page, find(), size);
        await shot(page, `find-empty-${mode}-${size.name}`, size === LARGE);
      });
      test(`find results ${mode} ${size.name}`, async ({ page }) => {
        await openFind(page, find(), size);
        await searchFor(page, "pricing");
        await shot(page, `find-results-${mode}-${size.name}`, size === LARGE);
      });
      test(`find answered ${mode} ${size.name}`, async ({ page }) => {
        await openFind(page, find("local", "answer"), size);
        await searchFor(page, QUESTION);
        await askFor(page, QUESTION);
        await page.getByRole("button", { name: /^Source 2,/ }).click();
        await settle(page, 500);
        await shot(page, `find-answered-${mode}-${size.name}`, size === LARGE);
      });
    }
    test(`find best matches ${mode}`, async ({ page }) => {
      await openFind(page, find());
      await searchFor(page, "pricing");
      await page.getByRole("radio", { name: "Best matches first" }).click();
      await settle(page, 500);
      await shot(page, `find-best-matches-${mode}-large`, true);
    });
    test(`find streaming ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "stream"));
      await searchFor(page, QUESTION);
      await askFor(page, QUESTION, 900);
      await shot(page, `find-streaming-${mode}-large`);
    });
    test(`find queued ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "stream"));
      await askFor(page, QUESTION, 700);
      await askFor(page, "And what did Priya say?", 600);
      await shot(page, `find-queued-${mode}-large`);
    });
    test(`find failed ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "failed"));
      await searchFor(page, "pricing");
      await askFor(page, QUESTION, 900);
      await shot(page, `find-failed-${mode}-large`);
    });
    test(`find set up ${mode}`, async ({ page }) => {
      await openFind(page, find("setup"));
      await searchFor(page, QUESTION);
      await shot(page, `find-setup-needed-${mode}-large`, true);
    });
    test(`find no results ${mode}`, async ({ page }) => {
      await openFind(page, find());
      await chip(page, "Apps", "Excel");
      await chip(page, "Dates", "Last 7 days");
      await searchFor(page, "zzzz");
      await shot(page, `find-no-results-${mode}-large`);
    });
    test(`find person ${mode}`, async ({ page }) => {
      await openFind(page, find());
      await page.getByRole("button", { name: "People", exact: true }).click();
      await page.getByRole("menuitemradio", { name: "Sam Okafor" }).click();
      await searchFor(page, "pricing");
      await shot(page, `find-person-${mode}-large`, true);
    });
    test(`find no search model ${mode}`, async ({ page }) => {
      await openFind(page, find("noSearchModel"));
      await searchFor(page, "pricing");
      await shot(page, `find-no-search-model-${mode}-large`);
    });
    test(`find new conversation ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "answer"));
      await askFor(page, QUESTION);
      await chip(page, "Apps", "Excel");
      await settle(page, 500);
      await shot(page, `find-new-conversation-${mode}-large`);
    });
    for (const [name, id] of [["legacy", 2], ["source-gone", 3]] as const)
      test(`find saved ${name} ${mode}`, async ({ page }) => {
        await openFind(page, find());
        await page.getByRole("button", { name: "Conversations", exact: true }).click();
        await page.locator(".th-conv-title").nth(id - 1).click();
        await settle(page, 900);
        await shot(page, `find-${name}-${mode}-large`, true);
      });
    test(`find saved with panel ${mode}`, async ({ page }) => {
      await openFind(page, find());
      await page.getByRole("button", { name: "Conversations", exact: true }).click();
      await page.locator(".th-conv-title").first().click();
      await settle(page, 900);
      await shot(page, `find-conversations-${mode}-large`, true);
    });
    for (const [name, label] of [["recent", "Recent"], ["episodes", "Episodes"], ["digest", "Digest"]] as const)
      test(`find view ${name} ${mode}`, async ({ page }) => {
        await openFind(page, find());
        await page.getByRole("radio", { name: label, exact: true }).click();
        await settle(page, 700);
        if (name === "digest") {
          await page.getByRole("button", { name: "Generate today's digest" }).click();
          await settle(page, 700);
        }
        await shot(page, `find-${name}-${mode}-large`, true);
      });
    test(`find from home ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", find(), 1200, THREAD);
      await settle(page, 600);
      await page.getByRole("button", { name: /^Open memory/ }).nth(1).click();
      await settle(page, 1100);
      await shot(page, `find-from-home-${mode}-large`);
    });
    test(`find answer with code and marks ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "rich"));
      await askFor(page, QUESTION);
      await shot(page, `find-answer-rich-${mode}-large`, true);
    });
    // Conversation 1 kept Excel and Nov 9 to Nov 15 (fixtures.ts): opening it puts those chips back in place of the ones that were on, and
    // the follow-up is asked under them in the same conversation.
    test(`find reopened conversation with a follow-up ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "answer"));
      await chip(page, "Dates", "Last 7 days");
      await page.getByRole("button", { name: "Conversations", exact: true }).click();
      await page.locator(".th-conv-title").first().click();
      await settle(page, 900);
      await shot(page, `find-reopened-${mode}-large`, true);
      await page.locator(".th-cv-composer input").fill("And what did Priya say?");
      await page.keyboard.press("Enter");
      await settle(page, 3400);
      await shot(page, `find-reopened-follow-up-${mode}-large`, true);
    });
    // A conversation asked under a People filter: its Dates and Apps come back, the person does not, and the page says so.
    test(`find reopened people not kept ${mode}`, async ({ page }) => {
      const kept = { start_unix_ms: Date.UTC(2026, 10, 9), end_unix_ms: Date.UTC(2026, 10, 16), app_bundle_id: "excel.exe", people_left_out: true };
      await openFind(page, find("local", undefined, { chat_conversation_find_filter: kept }));
      await page.getByRole("button", { name: "Conversations", exact: true }).click();
      await page.locator(".th-conv-title").first().click();
      await settle(page, 900);
      await shot(page, `find-reopened-people-not-kept-${mode}-large`, true);
    });
    test(`home with search and ask ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", find("local"), 1200, THREAD);
      await settle(page, 600);
      await field(page).fill("What did Sam say about pricing?");
      await settle(page, 900);
      await shot(page, `home-search-ask-${mode}-large`);
    });
    test(`find zoom 200 ${mode}`, async ({ page }) => {
      await openFind(page, find("local", "answer"), SMALL);
      await askFor(page, QUESTION);
      await zoom200(page);
      await shot(page, `find-zoom200-${mode}-small`, true);
    });

    // ===== Conversations (screen 6): Ask on Today, the Conversations section, a topic's page. The flag is on. ====
    const conv = (kind: ConvKind = "default", script?: AskScript, more: Record<string, unknown> = {}) => ({ ...find("local", script), ...convPatch(NOW, kind), ...more });
    const openToday = async (page: Page, patch: Record<string, unknown>, size = LARGE) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", patch, 1200, THREAD);
      await settle(page, 600);
    };
    const openConv = async (page: Page, patch: Record<string, unknown>, size = LARGE) => {
      await openToday(page, patch, size);
      await tab(page, "conversations");
      await settle(page, 800);
    };
    const openRow = async (page: Page, id: number) => {
      await page.locator(`[data-conv-row="${id}"]`).click();
      await settle(page, 900);
    };
    for (const size of SIZES) {
      test(`conv today answered ${mode} ${size.name}`, async ({ page }) => {
        await openToday(page, conv("default", "answer"), size);
        await askFor(page, QUESTION);
        // From the top: the question and the start of the answer are in frame.
        await shot(page, `conv-today-answered-${mode}-${size.name}`, size === LARGE);
        // Then a source selected, which scrolls the answer area to its card.
        await page.getByRole("button", { name: /^Source 2,/ }).click();
        await settle(page, 400);
        await shot(page, `conv-today-answered-source-${mode}-${size.name}`, size === LARGE);
      });
      test(`conv list ${mode} ${size.name}`, async ({ page }) => {
        await openConv(page, conv(), size);
        await shot(page, `conv-list-${mode}-${size.name}`, size === LARGE);
      });
      test(`conv page topic ${mode} ${size.name}`, async ({ page }) => {
        await openConv(page, conv(), size);
        await openRow(page, 4);
        await shot(page, `conv-page-topic-${mode}-${size.name}`, size === LARGE);
      });
    }
    test(`conv today streaming ${mode}`, async ({ page }) => {
      await openToday(page, conv("default", "stream"));
      await askFor(page, QUESTION, 900);
      await shot(page, `conv-today-streaming-${mode}-large`);
    });
    test(`conv today error ${mode}`, async ({ page }) => {
      await openToday(page, conv("default", "failed"));
      await askFor(page, QUESTION, 900);
      await shot(page, `conv-today-error-${mode}-large`);
    });
    test(`conv today continue ${mode}`, async ({ page }) => {
      await openToday(page, conv("default", "answer"));
      await askFor(page, QUESTION);
      await page.getByRole("button", { name: "Continue as a conversation" }).click();
      await settle(page, 1200);
      await shot(page, `conv-today-continued-${mode}-large`, true);
    });
    test(`conv today zoom 200 ${mode}`, async ({ page }) => {
      await openToday(page, conv("default", "answer"), SMALL);
      await askFor(page, QUESTION);
      await zoom200(page);
      await shot(page, `conv-today-zoom200-${mode}-small`, true);
    });
    test(`conv list empty ${mode}`, async ({ page }) => {
      await openConv(page, conv("empty"));
      await shot(page, `conv-list-empty-${mode}-large`);
    });
    test(`conv page everything ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await openRow(page, 1);
      await shot(page, `conv-page-everything-${mode}-large`, true);
    });
    test(`conv page topic menu ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await openRow(page, 4);
      await page.getByRole("button", { name: /^Topic: Northwind/ }).click();
      await settle(page, 400);
      await shot(page, `conv-page-topic-menu-${mode}-large`, true);
    });
    test(`conv page topic menu all ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await openRow(page, 4);
      await page.getByRole("button", { name: /^Topic: Northwind/ }).click();
      await page.getByRole("menuitem", { name: "Choose another topic…" }).click();
      await settle(page, 400);
      await shot(page, `conv-page-topic-menu-all-${mode}-large`, true);
    });
    test(`conv page more menu ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await openRow(page, 4);
      await page.getByRole("button", { name: /More options/ }).click();
      await settle(page, 400);
      await shot(page, `conv-page-more-menu-${mode}-large`);
    });
    test(`conv page topic follow-up ${mode}`, async ({ page }) => {
      await openConv(page, conv("default", "answer"));
      await openRow(page, 4);
      await page.locator(".th-cv-composer input").fill("What about the Acme agreement?");
      await page.keyboard.press("Enter");
      await settle(page, 1100);
      await shot(page, `conv-page-topic-follow-up-${mode}-large`, true);
    });
    test(`conv page meeting ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await openRow(page, 5);
      await shot(page, `conv-page-meeting-${mode}-large`, true);
    });
    test(`conv page new ${mode}`, async ({ page }) => {
      await openConv(page, conv());
      await page.getByRole("button", { name: "New conversation" }).click();
      await settle(page, 600);
      await shot(page, `conv-page-new-${mode}-large`);
    });
    test(`conv page zoom 200 ${mode}`, async ({ page }) => {
      await openConv(page, conv(), SMALL);
      await openRow(page, 4);
      await zoom200(page);
      await shot(page, `conv-page-zoom200-${mode}-small`, true);
    });
    test(`conv topic page ${mode}`, async ({ page }) => {
      await openToday(page, conv());
      // The Topics tab opens on its first topic by itself (Northwind).
      await tab(page, "topics");
      await settle(page, 1500);
      await shot(page, `conv-topic-page-${mode}-large`, true);
    });
    test(`conv topic page small ${mode}`, async ({ page }) => {
      await openToday(page, conv(), SMALL);
      await tab(page, "topics");
      await settle(page, 1500);
      await shot(page, `conv-topic-page-${mode}-small`, true);
    });
    test(`conv topic ask ${mode}`, async ({ page }) => {
      await openToday(page, conv("default", "answer"));
      await tab(page, "topics");
      await settle(page, 1500);
      await page.getByRole("textbox", { name: "Ask about Northwind" }).fill("What is still open with Northwind?");
      await page.getByRole("button", { name: "Ask", exact: true }).last().click();
      await settle(page, 1100);
      await shot(page, `conv-topic-ask-${mode}-large`);
    });
  }

  // ===== Settings, redesigned (screen 8). The Thread skin is drawn; every picture is `settings-*`. ==========
  if (AFTER) {
    const THREAD: Skin = { id: "thread", palette: "thread" };
    const WIDE = { width: 2560, height: 1100 };
    // The real-PC shape at Windows text 150%: the window is 734 x 481 CSS px at twice the pixel density.
    const PC150 = { width: 734, height: 481 };
    const MEMORY_BANNED = /memory database|activity capture|ai & routing|capture & privacy|never delete|keep full quality forever/i;
    const MADE_UP_KEY = "sk-example-not-a-key";

    async function goSection(page: Page, section: string) {
      const button = page.locator(`#${section}-nav`);
      if (await button.isVisible()) await button.click();
      else await page.getByRole("combobox", { name: "Settings section" }).selectOption(section);
      await settle(page, 900);
    }
    async function openSettings(page: Page, mode: Mode, section: string, patch: Record<string, unknown> = {}, size: { width: number; height: number } = LARGE) {
      await page.setViewportSize(size);
      await open(page, mode, "main", { ...patch }, 1100, THREAD);
      await tab(page, "settings");
      await settle(page, 600);
      await goSection(page, section);
    }
    /** What a shot must hold, asserted rather than eyeballed: nothing scrolls sideways, one h1, no retired name, every
     *  search anchor of the section is there (conditional ones are listed, not failed), and no key is drawn back. */
    async function checks(page: Page, section: string, conditional: string[] = []) {
      const found = await page.evaluate(
        ({ section, anchors }) => {
          const root = document.querySelector(".rs-frame") as HTMLElement;
          const wide = [document.documentElement, document.body, document.querySelector(".content")].filter(Boolean).some((e) => (e as HTMLElement).scrollWidth > (e as HTMLElement).clientWidth + 1);
          const html = document.body.innerHTML;
          return {
            wide,
            h1: document.querySelectorAll("h1").length,
            text: root?.innerText ?? "",
            missing: anchors.filter((id) => !document.querySelector(`[data-settings-anchor="${id}"]`)),
            secret: html.includes("a1b2") || html.includes("sk-ant-…"),
            section,
          };
        },
        { section, anchors: SETTINGS_SEARCH_ENTRIES.filter((e) => e.section === section).map((e) => e.id) },
      );
      expect(found.wide, "horizontal overflow").toBe(false);
      expect(found.h1, "one h1").toBe(1);
      expect(found.text).not.toMatch(MEMORY_BANNED);
      expect(found.secret, "a saved key is drawn").toBe(false);
      const real = found.missing.filter((id) => !conditional.includes(id));
      expect(real, "missing anchors").toEqual([]);
      if (found.missing.length) console.log(`[anchors ${section}] not drawn in this state: ${found.missing.join(", ")}`);
    }
    // Anchors that only exist when their state does: the Manage row (a paid plan), the devices list (sync ready), Files kept aside.
    const CONDITIONAL = ["plan.manage", "capture.foreignFiles", "devices.list"];

    const SECTION_IDS = ["status", "general", "profile", "plan", "capture", "meetings", "ai", "topics", "devices", "connectors"];
    for (const section of SECTION_IDS)
      test(`settings redesigned ${section} ${mode}`, async ({ page }) => {
        await openSettings(page, mode, section);
        await checks(page, section, CONDITIONAL);
        await shot(page, `settings-${section}-${mode}-large`, true);
      });
    for (const section of ["capture", "ai"])
      test(`settings redesigned ${section} small ${mode}`, async ({ page }) => {
        await openSettings(page, mode, section, {}, SMALL);
        await checks(page, section, CONDITIONAL);
        await shot(page, `settings-${section}-${mode}-small`, true);
      });
    for (const section of ["capture", "ai"])
      test(`settings redesigned ${section} pc150 ${mode}`, async ({ browser }) => {
        const context = await browser.newContext({ viewport: PC150, deviceScaleFactor: 2, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
        const page = await context.newPage();
        await openSettings(page, mode, section, {}, PC150);
        await checks(page, section, CONDITIONAL);
        await shot(page, `settings-${section}-${mode}-pc150`, true);
        await context.close();
      });
    test(`settings redesigned forced colors ${mode}`, async ({ page }) => {
      for (const section of ["capture", "ai"]) {
        await openSettings(page, mode, section);
        await page.emulateMedia({ forcedColors: "active" });
        await settle(page, 400);
        await shot(page, `settings-${section}-forced-${mode}-large`, true);
      }
    });
    test(`settings redesigned wide ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "capture", {}, WIDE);
      await checks(page, "capture", CONDITIONAL);
      await shot(page, `settings-wide-${mode}-2560`);
    });

    // --- states --------------------------------------------------------------------------------------------------------------------------------
    const sync = answers(NOW).sync_status as Record<string, unknown>;
    const plan = (over: Record<string, unknown>) => ({ ...sync, plan: { billing: true, interval: null, plan: "free", planTier: "free", plusAvailable: true, renews: false, ...over } });
    const states: [string, Record<string, unknown>][] = [
      ["signed-out", { sync_status: { ...sync, state: "notSignedIn", plan: null }, sync_refresh_plan: { ...sync, state: "notSignedIn", plan: null } }],
      ["free", { sync_status: plan({}), sync_refresh_plan: plan({}) }],
      ["plus", { sync_status: plan({ plan: "cloud", planTier: "plus", interval: "month", renews: true, untilUnixMs: NOW + 18 * 86_400_000 }), sync_refresh_plan: plan({ plan: "cloud", planTier: "plus", interval: "month", renews: true, untilUnixMs: NOW + 18 * 86_400_000 }), whimbrel_ai_status: { balanceMicros: 6_400_000, low: false, signedIn: true } }],
    ];
    for (const [name, patch] of states)
      test(`settings redesigned plan ${name} ${mode}`, async ({ page }) => {
        await openSettings(page, mode, "plan", patch);
        await checks(page, "plan", CONDITIONAL);
        await shot(page, `settings-plan-${name}-${mode}-large`, true);
      });
    test(`settings redesigned devices ready ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "devices", { sync_refresh_plan: sync });
      await checks(page, "devices", CONDITIONAL);
      await shot(page, `settings-devices-ready-${mode}-large`, true);
    });
    test(`settings redesigned topics empty tab ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", { topics_page_list: [] }, 1100, THREAD);
      await tab(page, "topics");
      await settle(page, 700);
      await shot(page, `settings-topics-empty-tab-${mode}-large`);
    });
    const aiBase = answers(NOW).ai_settings as Record<string, unknown>;
    test(`settings redesigned keys saved ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "ai", { ai_settings: { ...aiBase, keys: [{ masked: "sk-example-not-a-key", provider: "anthropic", syncEnabled: true }] } });
      const html = await page.evaluate(() => document.body.innerHTML);
      expect(html.includes(MADE_UP_KEY), "the saved key is drawn").toBe(false);
      await shot(page, `settings-ai-key-saved-${mode}-large`, true);
    });
    test(`settings redesigned keys replaced ${mode}`, async ({ page }) => {
      const TYPED = "sk-example-typed-in-replace-not-a-key";
      const keys = [{ masked: "sk-example-not-a-key", provider: "anthropic", syncEnabled: true }];
      await openSettings(page, mode, "ai", { ai_settings: { ...aiBase, keys }, set_api_key: keys });
      await page.getByRole("button", { name: "Replace Anthropic API key" }).click();
      await page.getByLabel("Anthropic API key").fill(TYPED);
      await page.getByRole("button", { name: "Save", exact: true }).first().click();
      await settle(page, 700);
      // After Replace, Save: the typed key is in no box and nowhere in the page, and the row is back to "saved".
      const after = await page.evaluate(() => ({ html: document.body.innerHTML, boxes: [...document.querySelectorAll<HTMLInputElement>('input[type="password"], input[type="text"]')].map((i) => i.value) }));
      expect(after.html.includes(TYPED), "the key typed in Replace is still in the page").toBe(false);
      expect(after.boxes.some((v) => v.includes("sk-")), "a box still holds a key").toBe(false);
      await expect(page.getByRole("button", { name: "Replace Anthropic API key" })).toBeVisible();
    });
    test(`settings redesigned keys empty ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "ai", { ai_settings: { ...aiBase, keys: [] } });
      await shot(page, `settings-ai-key-empty-${mode}-large`, true);
    });

    // --- Library encryption: every state reachable with fixtures says "your library", never the database ------------------------------------
    const ENC = { badge: "notEncrypted", cleanupRunning: false, fastSearch: { available: true, enabled: false }, notes: [] as unknown[], pendingNextLaunch: false, plainBackupPresent: false, showsStoreTools: true };
    const NOTE_KINDS = [
      { kind: "keyStoreUnavailable" },
      { kind: "failedBefore" },
      { kind: "migrationSkipped", stateUnreadable: true },
      { kind: "migrationSkipped", skip: { reason: "open_elsewhere" } },
      { kind: "migrationSkipped", skip: { reason: "recording" } },
      { kind: "migrationSkipped", skip: { reason: "low_disk" } },
      { kind: "migrationFailed" },
      { kind: "encryptionAvailable" },
      { kind: "plainBackupKept" },
      { kind: "restoredCopy", movedAside: 1, warning: "stateNotMovedAside" },
      { kind: "pendingKeyNotPromoted" },
      { kind: "pendingKeyNotDeleted" },
      { kind: "recoveredKeyNotStored", failure: "noPreviousSlot" },
      { kind: "recoveredKeyNotStored", failure: "other" },
      { kind: "recoveredKeyStoreUnverified" },
      { kind: "memoryDatabaseUnavailable" },
    ];
    const encryptionText = (page: Page) => page.evaluate(() => (document.querySelector("#capture\\.encryption") as HTMLElement | null)?.innerText ?? "");
    const NO_DATABASE = /database/i;
    const ENCRYPTION_STATES: [string, Record<string, unknown>][] = [
      ["encrypted with every note", { encryption_card: { ...ENC, badge: "encrypted", plainBackupPresent: true, notes: NOTE_KINDS.map((note) => ({ note, action: null })) } }],
      ["needs attention", { encryption_card: { ...ENC, badge: "needsAttention", notes: [{ note: { kind: "memoryDatabaseUnavailable" }, action: null }, { note: { kind: "keyStoreUnavailable" }, action: null }] } }],
      ["encrypt scheduled for the next launch", { encryption_card: { ...ENC, pendingNextLaunch: true } }],
      ["not encrypted", { encryption_card: ENC }],
    ];
    for (const [name, patch] of ENCRYPTION_STATES)
      test(`settings redesigned encryption ${name} ${mode}`, async ({ page }) => {
        await openSettings(page, mode, "capture", patch);
        expect(await encryptionText(page), name).not.toMatch(NO_DATABASE);
      });
    for (const refusal of ["keychainUnavailable", "twoKeysFound", "recording", "lowDisk", "openElsewhere", "failedBefore", "other"])
      test(`settings redesigned encryption refused ${refusal} ${mode}`, async ({ page }) => {
        await openSettings(page, mode, "capture", { encryption_card: ENC, encryption_precheck_for_setup: refusal });
        await page.getByRole("button", { name: "Encrypt your library…" }).click();
        await settle(page, 500);
        const said = await encryptionText(page);
        expect(said, refusal).not.toMatch(NO_DATABASE);
        expect(said.length).toBeGreaterThan(40);
        if (refusal === "lowDisk") await shot(page, `settings-encryption-refused-${mode}-large`);
      });
    test(`settings redesigned encryption wizard ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "capture", { encryption_card: ENC, encryption_precheck_for_setup: null, setup_reuses_key: false, ...SETUP, setup_save_file: { kind: "saved", path: "C:\\Users\\Alex\\Documents\\Whimbrel recovery key.txt", sameDiskAsDatabase: true } });
      await page.getByRole("button", { name: "Encrypt your library…" }).click();
      await settle(page, 500);
      // Every screen of the wizard: the passphrase step, then the words, the file (same disk warning), the test, and the end.
      expect(await encryptionText(page), "wizard, step 1").not.toMatch(NO_DATABASE);
      await page.getByLabel("Passphrase", { exact: true }).first().fill(PHRASE);
      await page.getByLabel("Type it again").fill(PHRASE);
      await page.getByRole("button", { name: "Create recovery key" }).click();
      await settle(page, 600);
      await page.getByRole("button", { name: /Save recovery file/ }).click();
      await settle(page, 500);
      const wizard = await encryptionText(page);
      expect(wizard, "wizard, steps 2 to 4").not.toMatch(NO_DATABASE);
      expect(wizard).toContain("your library");
      // The page "Print words and QR code…" prints is in the document too (visible in print media only): it names the library as well.
      const printed = (await page.locator(".print-page").textContent()) ?? "";
      expect(printed.length, "the print page is drawn").toBeGreaterThan(100);
      expect(printed, "print page").not.toMatch(NO_DATABASE);
      expect(printed).toContain("your library");
      await shot(page, `settings-encryption-wizard-${mode}-large`, true);
    });
    test(`settings redesigned retention confirm ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "capture");
      const box = page.getByRole("textbox", { name: /Screen text/ }).or(page.getByLabel(/Screen text/)).first();
      // The row being changed stays in view behind the question, so the picture shows what it is about.
      await page.locator("#capture\\.retention").evaluate((el) => el.scrollIntoView({ block: "start" }));
      await box.fill("30");
      await box.press("Enter");
      await settle(page, 500);
      await shot(page, `settings-retention-confirm-${mode}-large`);
    });
    test(`settings redesigned delete everything ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "capture");
      await page.getByRole("button", { name: /Delete everything/ }).first().click();
      await settle(page, 700);
      await shot(page, `settings-delete-everything-${mode}-large`);
    });
    test(`settings redesigned search ${mode}`, async ({ page }) => {
      await openSettings(page, mode, "general");
      await page.getByRole("searchbox", { name: "Search settings" }).fill("capture");
      await settle(page, 500);
      await shot(page, `settings-search-results-${mode}-large`);
      await page.getByRole("searchbox", { name: "Search settings" }).fill("zzzz");
      await settle(page, 400);
      await shot(page, `settings-search-none-${mode}-large`);
      // A result opens its section and marks the row.
      await page.getByRole("searchbox", { name: "Search settings" }).fill("local-only");
      await settle(page, 300);
      await page.getByRole("button", { name: /Local-only mode/ }).first().click();
      await settle(page, 900);
      await shot(page, `settings-search-jump-${mode}-large`);
    });
  }

  // ===== The restyle pass (step 10): Topics, Automations, the Meetings list, the capture and the episode, and the fixes from the real-PC
  // checks. The Thread skin is drawn. ===========
  if (AFTER) {
    const THREAD: Skin = { id: "thread", palette: "thread" };
    /** The real-PC shapes: the main window at Windows text 150% (734 x 481 CSS px) and at 200% (550 x 360), twice the pixel density. */
    const TEXT150 = { width: 734, height: 481 };
    const TEXT200 = { width: 550, height: 360 };
    const openAt = async (page: Page, patch: Record<string, unknown>, size = LARGE, wait = 1100) => {
      await page.setViewportSize(size);
      await open(page, mode, "main", { ...patch }, wait, THREAD);
    };
    /** What a restyled screen must hold at any size, asserted rather than eyeballed: nothing scrolls sideways, and nothing visible in
     *  the content area is wider than the window or cut off at its right edge. */
    const sound = async (page: Page, what: string) => {
      const found = await page.evaluate(() => {
        const vw = document.documentElement.clientWidth;
        const content = document.querySelector("#panel");
        const out: string[] = [];
        for (const el of Array.from(content?.querySelectorAll<HTMLElement>("*") ?? [])) {
          const r = el.getBoundingClientRect();
          if (r.width < 1 || r.height < 1) continue;
          if (r.right > vw + 0.5) out.push(`${el.className || el.tagName} ends at ${Math.round(r.right)} of ${vw}`);
          // Nor past the card, list or page it sits in.
          const box = el.parentElement?.closest<HTMLElement>(".card, .topics-page, .detail, .meeting-list, .w-state");
          if (box && r.right > box.getBoundingClientRect().right + 1.5 && getComputedStyle(el).position !== "absolute") out.push(`${el.className || el.tagName} ends at ${Math.round(r.right)}, past its ${box.className.split(" ")[0]} at ${Math.round(box.getBoundingClientRect().right)}`);
        }
        return { wide: document.documentElement.scrollWidth > vw + 1 || document.body.scrollWidth > vw + 1, out: out.slice(0, 6), vw };
      });
      expect(found.wide, `${what}: the page scrolls sideways`).toBe(false);
      expect(found.out, `${what}: cut off at the right edge`).toEqual([]);
    };
    const shapes = (name: string, prepare: (page: Page) => Promise<void>, patch: Record<string, unknown> = {}) => {
      test(`${name} text 150 ${mode}`, async ({ browser }) => {
        const context = await browser.newContext({ deviceScaleFactor: 2, viewport: TEXT150, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
        const page = await context.newPage();
        try {
          await open(page, mode, "main", { ...patch }, 1100, THREAD);
          await prepare(page);
          await sound(page, `${name} at 734x481`);
          await page.screenshot({ path: `${OUT}${name}-text150-${mode}.png` });
        } finally {
          await context.close();
        }
      });
      test(`${name} text 200 ${mode}`, async ({ browser }) => {
        const context = await browser.newContext({ deviceScaleFactor: 2, viewport: TEXT200, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
        const page = await context.newPage();
        try {
          await open(page, mode, "main", { ...patch }, 1100, THREAD);
          await prepare(page);
          await sound(page, `${name} at 550x360`);
          await page.screenshot({ path: `${OUT}${name}-text200-short-${mode}.png` });
        } finally {
          await context.close();
        }
      });
      test(`${name} forced colors ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        await open(page, mode, "main", { ...patch }, 1100, THREAD);
        await page.emulateMedia({ colorScheme: mode, forcedColors: "active" });
        await prepare(page);
        await shot(page, `${name}-forced-${mode}-large`);
      });
    };

    // --- Topics: the list and a topic page, an empty list, an error, another topic chosen ---------------------------------
    for (const size of SIZES)
      test(`topics restyle ${mode} ${size.name}`, async ({ page }) => {
        await openAt(page, {}, size);
        await tab(page, "topics");
        await settle(page, 900);
        await shot(page, `topics-${mode}-${size.name}`, size === LARGE);
      });
    test(`topics empty restyle ${mode}`, async ({ page }) => {
      await openAt(page, { topics_page_list: [] });
      await tab(page, "topics");
      await settle(page);
      await shot(page, `topics-empty-${mode}-large`);
    });
    test(`topics error ${mode}`, async ({ page }) => {
      await openAt(page, { topics_page_list: "__fail__:Could not read your topics. Try again in a moment." });
      await tab(page, "topics");
      await settle(page);
      await shot(page, `topics-error-${mode}-large`);
    });
    test(`topics loading ${mode}`, async ({ page }) => {
      await openAt(page, { topics_page_list: "__hang__" });
      await tab(page, "topics");
      await settle(page, 600);
      await shot(page, `topics-loading-${mode}-large`);
    });
    test(`topics selected ${mode}`, async ({ page }) => {
      await openAt(page, {});
      await tab(page, "topics");
      await settle(page, 700);
      await page.locator(".topic-row").nth(1).click();
      await settle(page, 700);
      await shot(page, `topics-selected-${mode}-large`);
      await page.locator(".topic-row").nth(1).focus();
      await page.keyboard.press("Tab");
      await settle(page, 200);
      await shot(page, `topics-focus-${mode}-large`);
    });
    shapes("topics", async (page) => {
      await tab(page, "topics");
      await settle(page, 900);
    });

    // --- Automations: approvals waiting, routines, history; nothing yet; an error; still loading --------------------------
    for (const size of SIZES)
      test(`automations restyle ${mode} ${size.name}`, async ({ page }) => {
        await openAt(page, {}, size);
        await tab(page, "automations");
        await settle(page, 900);
        await shot(page, `automations-${mode}-${size.name}`, size === LARGE);
      });
    test(`automations empty ${mode}`, async ({ page }) => {
      await openAt(page, { approvals_list: [], routines_list: [], routine_runs: [] });
      await tab(page, "automations");
      await settle(page, 900);
      await shot(page, `automations-empty-${mode}-large`);
    });
    test(`automations error ${mode}`, async ({ page }) => {
      await openAt(page, { routines_list: "__fail__:Could not read your routines. Try again in a moment." });
      await tab(page, "automations");
      await settle(page, 900);
      await shot(page, `automations-error-${mode}-large`);
    });
    test(`automations loading ${mode}`, async ({ page }) => {
      await openAt(page, { routines_list: "__hang__" });
      await tab(page, "automations");
      await settle(page, 700);
      await shot(page, `automations-loading-${mode}-large`);
    });
    test(`automations history open ${mode}`, async ({ page }) => {
      await openAt(page, {});
      await tab(page, "automations");
      await settle(page, 900);
      await page.locator(".run-head").first().click();
      await page.locator(".run-head").first().focus();
      await settle(page, 300);
      await shot(page, `automations-history-${mode}-large`, true);
    });
    shapes("automations", async (page) => {
      await tab(page, "automations");
      await settle(page, 900);
    });

    // The redesigned Meetings tab opens on the finished note, full width; Back shows the list.
    const listFirst = async (page: Page) => {
      const back = page.getByRole("button", { name: /Back/ }).first();
      if (await back.isVisible().catch(() => false)) {
        await back.click();
        await settle(page, 600);
      }
    };
    // --- The Meetings list (the finished note and the live screen are drawn by their own blocks) -------------------------------
    for (const size of SIZES)
      test(`meetings restyle ${mode} ${size.name}`, async ({ page }) => {
        await openAt(page, {}, size);
        await tab(page, "meetings");
        await settle(page, 1200);
        await listFirst(page);
        await shot(page, `meetings-${mode}-${size.name}`, size === LARGE);
      });
    test(`meetings empty ${mode}`, async ({ page }) => {
      await openAt(page, { meeting_list: [], meeting_chips: [] });
      await tab(page, "meetings");
      await settle(page, 900);
      await shot(page, `meetings-empty-${mode}-large`);
    });
    test(`meetings error ${mode}`, async ({ page }) => {
      await openAt(page, { meeting_list: "__fail__:Could not read your meetings. Try again in a moment." });
      await tab(page, "meetings");
      await settle(page, 900);
      await shot(page, `meetings-error-${mode}-large`);
    });
    test(`meetings selected ${mode}`, async ({ page }) => {
      await openAt(page, {});
      await tab(page, "meetings");
      await settle(page, 1200);
        await listFirst(page);
      // The open meeting (first) shows its bar; the keyboard focus ring is on the third row.
      await page.locator(".meeting-item__hit").nth(2).focus();
      await shot(page, `meetings-selected-${mode}-large`);
    });
    shapes("meetings", async (page) => {
      await tab(page, "meetings");
      await settle(page, 1200);
        await listFirst(page);
      // The finished note's toolbar at the real text shapes (734x481 and 550x360): every control keeps its words, and every control's box
      // holds its own icon, words and keycap, and no two boxes overlap. (The row of controls may wrap; what it may not do is poke out.)
      const bar = await page.evaluate(() => {
        const root = document.querySelector(".th-fin-bar-end");
        if (!root) return null;
        const buttons = Array.from(root.querySelectorAll<HTMLElement>("button"));
        const named = (b: HTMLElement) => (b.getAttribute("aria-label") || b.textContent || "").trim();
        const out: string[] = [];
        const boxes = buttons.map((b) => b.getBoundingClientRect());
        buttons.forEach((b, i) => {
          const r = boxes[i];
          for (const el of Array.from(b.querySelectorAll<HTMLElement>("*"))) {
            const c = el.getBoundingClientRect();
            if (c.width < 1 || c.height < 1) continue;
            if (c.left < r.left - 1 || c.right > r.right + 1 || c.top < r.top - 1 || c.bottom > r.bottom + 1)
              out.push(`${named(b)}: ${el.getAttribute("class") || el.tagName} sticks out of its box (child ${Math.round(c.left)}-${Math.round(c.right)}, box ${Math.round(r.left)}-${Math.round(r.right)})`);
          }
          if (b.scrollWidth > b.clientWidth + 1) out.push(`${named(b)}: its content is wider than its box`);
          boxes.forEach((o, j) => {
            if (j > i && Math.min(r.right, o.right) - Math.max(r.left, o.left) > 1 && Math.min(r.bottom, o.bottom) - Math.max(r.top, o.top) > 1) out.push(`${named(b)} overlaps ${named(buttons[j])}`);
          });
        });
        return { names: buttons.map(named), out };
      });
      expect(bar, "the finished note has a toolbar").not.toBeNull();
      for (const name of ["Copy", "More ways to copy", "Send recap", "Edit", "Note", "Transcript", "More"])
        expect(bar!.names.some((n) => n === name || n.startsWith(`${name} `) || n.startsWith(name) && name === "Transcript"), `the ${name} control is still there`).toBe(true);
      expect(bar!.out, "the toolbar controls hold their contents and do not overlap").toEqual([]);
      // The Import disclosure takes the Thread target, 32px here (the summary rule is `.rt.meeting-list summary`, not a descendant selector).
      const summary = await page.locator(".meeting-list summary").first().boundingBox();
      expect(summary, "the Import summary").not.toBeNull();
      expect(summary!.height, "the Import summary has the Thread target (32px, --w-target)").toBeGreaterThanOrEqual(31.5);
    });

    // --- The capture and the episode, as the Library's Recent view opens them ----------------------------------------------------
    for (const [kind, key] of [["capture", "capture-101"], ["episode", "episode-201"]] as const) {
      const openDetail = async (page: Page) => {
        await tab(page, "library");
        await settle(page, 700);
        await page.getByRole("radio", { name: "Recent", exact: true }).click();
        await settle(page, 700);
        await page.locator(`[data-open-key="${key}"]`).click();
        await settle(page, 900);
      };
      for (const size of SIZES)
        test(`library ${kind} restyle ${mode} ${size.name}`, async ({ page }) => {
          await openAt(page, {}, size);
          await openDetail(page);
          await shot(page, `library-${kind}-${mode}-${size.name}`, size === LARGE);
        });
      shapes(`library-${kind}`, openDetail);
    }
    test(`library capture error ${mode}`, async ({ page }) => {
      await openAt(page, { library_capture: "__fail__:This item could not be read." });
      await tab(page, "library");
      await settle(page, 700);
      await page.getByRole("radio", { name: "Recent", exact: true }).click();
      await settle(page, 700);
      await page.locator('[data-open-key="capture-101"]').click();
      await settle(page, 700);
      await shot(page, `library-capture-error-${mode}-large`);
    });

    // --- Fix 4: the recap shows formatted text, not raw Markdown ------------------------------------------------------------------
    test(`home recap markdown ${mode}`, async ({ page }) => {
      const overview = answers(NOW).today_overview as Record<string, unknown>;
      const recap = {
        ranAtUnixMs: NOW - 3 * 60 * 60_000,
        summary: "Yesterday you spent most of the day on **Kite Practice** and the **Airship Club Project**.\n\n- Booked the field for *Saturday*\n- Sent the parts list to the club\n\nRead [the field rules](https://example.com/rules) before you go.",
      };
      await openAt(page, { today_overview: { ...overview, briefRecap: recap } });
      await settle(page, 600);
      const text = await page.locator('[data-section="recap"]').innerText();
      expect(text).not.toContain("**");
      expect(text).toContain("Kite Practice");
      await page.locator('[data-section="recap"]').scrollIntoViewIfNeeded();
      await shot(page, `home-recap-markdown-${mode}-large`);
    });

    // --- Fix 5: the finished note with no transcript and no sources folds its margin away ----------------------------------------
    test(`note no transcript ${mode}`, async ({ page }) => {
      await openAt(page, notePatch(NOW, "noTranscript"), LARGE, 1200);
      await tab(page, "meetings");
      await settle(page, 1400);
      const margin = await page.evaluate(() => {
        const pageEl = document.querySelector<HTMLElement>(".th-fin-page");
        const row = document.querySelector<HTMLElement>(".th-fin-row");
        const r = pageEl?.getBoundingClientRect();
        return { has: pageEl?.dataset.hasMargin ?? null, columns: row ? getComputedStyle(row).gridTemplateColumns.split(" ").length : 0, width: r ? Math.round(r.width) : 0, left: r ? Math.round(r.left) : 0, vw: document.documentElement.clientWidth };
      });
      expect(margin.has, "has-margin").toBe("false");
      expect(margin.columns, "one column").toBe(1);
      await shot(page, `note-no-transcript-${mode}-large`, true);
    });
    test(`note no transcript small ${mode}`, async ({ page }) => {
      await openAt(page, notePatch(NOW, "noTranscript"), SMALL, 1200);
      await tab(page, "meetings");
      await settle(page, 1400);
      await shot(page, `note-no-transcript-${mode}-small`, false);
    });

    // --- Fix 6: the top bar at Windows text 150% and 200%: the recording state and the Memory state stay, full size, on the first row ------
    const TOP_BAR = () => {
      const vw = document.documentElement.clientWidth;
      const vh = window.innerHeight;
      const box = (sel: string) => {
        const el = document.querySelector<HTMLElement>(sel);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { top: Math.round(r.top), left: Math.round(r.left), right: Math.round(r.right), bottom: Math.round(r.bottom), width: Math.round(r.width), height: Math.round(r.height) };
      };
      const clipped = (sel: string) => {
        const el = document.querySelector<HTMLElement>(sel);
        if (!el) return true;
        const r = el.getBoundingClientRect();
        const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        return r.width < 4 || r.height < 4 || r.left < -0.5 || r.right > vw + 0.5 || r.top < -0.5 || r.bottom > vh + 0.5 || !top || !(el.contains(top) || top.contains(el)) || el.scrollWidth > el.clientWidth + 1;
      };
      const capture = document.querySelector(".w-shell-pinned .w-capture-button") ? ".w-shell-pinned .w-capture-button" : ".w-shell-capture .w-capture-button";
      // The shell's bar on every section; on the live screen (Home gives its place to it) the screen draws the same bar itself.
      const rec = document.querySelector(".w-shell-pinned .w-rec") ? ".w-shell-pinned .w-rec" : document.querySelector(".w-shell-recording .w-rec") ? ".w-shell-recording .w-rec" : ".th-rec .w-rec";
      return {
        dpr: window.devicePixelRatio,
        vw,
        vh,
        wide: document.documentElement.scrollWidth > vw + 1,
        rec: { box: box(rec), clipped: clipped(rec) },
        memory: { box: box(capture), clipped: clipped(capture), font: getComputedStyle(document.querySelector(capture) as Element).fontSize },
        title: box(".w-shell-title"),
        toggle: box(".w-shell-search-toggle"),
        field: box(".w-shell-search"),
        fieldShown: !!document.querySelector<HTMLElement>(".w-shell-search")?.offsetParent,
      };
    };
    for (const [shape, size] of [["text150", TEXT150], ["text200", TEXT200]] as const)
      for (const kind of ["idle", "recording"] as const)
        test(`top bar ${shape} ${kind} ${mode}`, async ({ browser }) => {
          const context = await browser.newContext({ deviceScaleFactor: 2, viewport: size, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" });
          const page = await context.newPage();
          try {
            await open(page, mode, "main", { ...(kind === "recording" ? livePatch(NOW, "recording") : {}) }, 1400, THREAD);
            const at = await page.evaluate(TOP_BAR);
            await page.screenshot({ path: `${OUT}top-bar-${shape}-${kind}-${mode}.png` });
            console.log(`[top bar ${shape} ${kind}] ${JSON.stringify(at)}`);
            expect(at.dpr).toBe(2);
            expect(at.wide, "scrolls sideways").toBe(false);
            expect(at.memory.clipped, "the Memory state is clipped").toBe(false);
            expect(at.memory.box!.height, "the Memory control keeps its size").toBeGreaterThanOrEqual(30);
            if (kind === "recording") {
              expect(at.rec.clipped, "the recording state is clipped").toBe(false);
              expect(at.rec.box!.height, "the recording state keeps its size").toBeGreaterThanOrEqual(30);
            }
            if (shape === "text150") {
              // One row: the title, the search icon and the Memory state share a top edge band, and the field is not drawn under them.
              expect(at.fieldShown, "the field is collapsed to an icon").toBe(false);
              expect(at.toggle, "the search icon").not.toBeNull();
              expect(Math.abs(at.toggle!.top - at.memory.box!.top), "the icon and the Memory state are on one row").toBeLessThan(12);
            }
            if (shape === "text150") {
              // Ctrl+K opens the field and focuses it; the icon says it is expanded; Escape puts it away again.
              await page.keyboard.press("Control+k");
              await page.waitForTimeout(300);
              expect(await page.evaluate(() => document.activeElement?.getAttribute("aria-label"))).toBe("Search your work memory");
              expect(await page.evaluate(() => (document.activeElement as HTMLInputElement | null)?.tagName)).toBe("INPUT");
              expect(await page.evaluate(() => document.querySelector(".w-shell-search-toggle button")?.getAttribute("aria-expanded")), "the icon says the field is open").toBe("true");
              await page.screenshot({ path: `${OUT}top-bar-${shape}-${kind}-search-${mode}.png` });
              await page.keyboard.press("Escape");
              await page.waitForTimeout(200);
              expect(await page.evaluate(() => !!document.querySelector<HTMLElement>(".w-shell-search")?.offsetParent), "Escape puts the empty field away").toBe(false);
              expect(await page.evaluate(() => document.querySelector(".w-shell-search-toggle button")?.getAttribute("aria-expanded")), "the icon says the field is closed").toBe("false");
              expect(await page.evaluate(() => document.activeElement === document.querySelector(".w-shell-search-toggle button")), "focus returns to the search icon").toBe(true);
            }
          } finally {
            await context.close();
          }
        });

    // --- Fix 7: a recording that is starting shows "Starting recording" and offers Stop ----------------------------------------------
    test(`shell starting ${mode}`, async ({ page }) => {
      await openAt(page, livePatch(NOW, "starting"), LARGE, 1300);
      await tab(page, "automations");
      await settle(page, 700);
      await expect(page.locator(".w-shell-recording").getByText("Starting recording")).toBeVisible();
      const stop = page.locator(".w-shell-recording").getByRole("button", { name: "Stop" });
      await expect(stop).toBeVisible();
      await expect(stop).toBeEnabled();
      await shot(page, `shell-starting-${mode}-large`);
    });
  }

  // ===== The switch is on for everyone (docs/ui-redesign/screens/switch-removal.md, A1 and A2). =================================
  // A device with nothing stored is moved to Thread before the first paint; the system's more-contrast setting draws High Contrast
  // whatever skin is picked, and the stored choice stays. These are the only pictures that start from anything but a picked skin.
  {
    const THREAD_PICKED: Skin = { id: "thread", palette: "thread" };
    const FRESH: Skin = { id: null, palette: null, moveDone: false };
    const highContrast = async (page: Page) => {
      await page.emulateMedia({ contrast: "more" });
      await expect(async () => {
        expect(await page.evaluate(() => matchMedia("(prefers-contrast: more)").matches)).toBe(true);
      }).toPass();
    };
    test(`home fresh device ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", {}, 1100, FRESH);
      // Nothing stored: the page moved the device to Thread, and says so in what it stored.
      await expect(page.locator("html")).toHaveAttribute("data-skin", "thread");
      expect(await page.evaluate(() => [localStorage.getItem("skinId"), localStorage.getItem("paletteId"), localStorage.getItem("skinThreadMoveDone")])).toEqual(["thread", "thread", "1"]);
      await shot(page, `home-fresh-device-${mode}-large`);
    });
    test(`home classic picked ${mode}`, async ({ page }) => {
      await page.setViewportSize(LARGE);
      await open(page, mode, "main", {}, 1100, WHIMBREL);
      await expect(page.locator("html")).toHaveAttribute("data-skin", "whimbrel");
      await shot(page, `home-classic-${mode}-large`);
    });
    for (const [name, skin] of [["thread", THREAD_PICKED], ["classic", WHIMBREL]] as const)
      test(`home high contrast ${name} ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        await highContrast(page);
        await open(page, mode, "main", {}, 1100, skin);
        await expect(page.locator("html")).toHaveAttribute("data-palette", "high-contrast");
        // The stored choice is untouched.
        expect(await page.evaluate(() => localStorage.getItem("skinId"))).toBe(skin.id);
        await shot(page, `home-high-contrast-${name}-${mode}-large`);
      });
    for (const id of ["topics", "settings"])
      test(`${id} high contrast ${mode}`, async ({ page }) => {
        await page.setViewportSize(LARGE);
        await highContrast(page);
        await open(page, mode, "main", {}, 1100, THREAD_PICKED);
        await tab(page, id);
        await settle(page, 900);
        await expect(page.locator("html")).toHaveAttribute("data-palette", "high-contrast");
        await shot(page, `${id}-high-contrast-${mode}-large`);
      });
  }
}
