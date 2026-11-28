import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import catalogText from "../l10n/Localizable.xcstrings?raw";
import { NoStoreBanner } from "./NoStoreBanner";
import { qrOfWords, QUIET_ZONE } from "./qr";
import { PrintPage, WordsGrid, WordsQr } from "./RecoveryWords";
import { SetupModel } from "./setupModel";
import { SetupView } from "./SetupView";

// The X4 launch and encryption sources (brief 5.5): the launch screens, the card and its model.
const sources = Object.fromEntries(
  Object.entries({
    ...import.meta.glob("./**/*.{ts,tsx}", { query: "?raw", import: "default", eager: true }),
    ...import.meta.glob("../settings/{EncryptionCard,encryptionCardModel}.{ts,tsx}", { query: "?raw", import: "default", eager: true }),
  })
    .filter(([path]) => !/\.test\.tsx?$/.test(path) && !path.endsWith(".d.ts"))
    .map(([path, text]) => [path, text as string]),
);

/** Every key those sources pass to the t function, as translate.py extract reads them. */
const keys = [
  ...new Set(
    Object.values(sources).flatMap((text) => [...text.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`) as string)),
  ),
];
const catalog = JSON.parse(catalogText) as { strings: Record<string, { shouldTranslate?: boolean; localizations?: Record<string, unknown> }> };

describe("the copy rules (port of EncryptionCopyTests)", () => {
  it("finds the sources and their keys", () => {
    expect(Object.keys(sources).some((p) => p.endsWith("SetupView.tsx"))).toBe(true);
    expect(Object.keys(sources).some((p) => p.endsWith("EncryptionCard.tsx"))).toBe(true);
    expect(keys.length).toBeGreaterThan(150);
  });

  it("no em or en dash in any key", () => {
    expect(keys.filter((k) => /[–—]/.test(k))).toEqual([]);
  });

  it("none of the banned phrases", () => {
    const banned = [/stays on this PC/i, /nothing leaves/i, /never leaves/i, /we never receive/i, /safe from malware/i, /(?<!database )\bencrypted at rest\b/i, /\bMac\b/, /Keychain/, /Finder/, /Time Machine/];
    expect(keys.filter((k) => banned.some((b) => b.test(k)))).toEqual([]);
  });

  it("the words-are-the-key notice is present", () => {
    expect(keys).toContain(
      "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.",
    );
  });

  it("every address is the app's own feedback address", () => {
    const addresses = keys.flatMap((k) => k.match(/[\w.+-]+@[\w-]+(?:\.[\w-]+)+/g) ?? []);
    expect(addresses.length).toBeGreaterThan(0);
    expect(new Set(addresses)).toEqual(new Set(["hello@whimbrelapp.com"]));
  });

  it("every key is in the catalog, translated into every language unless flagged shouldTranslate=false", () => {
    expect(keys.filter((k) => !(k in catalog.strings))).toEqual([]);
    const langs = ["de", "es", "fr", "fr-CA", "pt"];
    const missing = keys.filter((k) => catalog.strings[k].shouldTranslate !== false && !langs.every((l) => l in (catalog.strings[k].localizations ?? {})));
    expect(missing).toEqual([]);
  });
});

describe("secret hygiene (port of EncryptionSecretHygieneTests and the no-network scan)", () => {
  const forbidden = ["navigator.clipboard", "writeText", 'execCommand("copy")', "execCommand('copy')", "console.", "localStorage", "sessionStorage", "draggable", "fetch(", "XMLHttpRequest", "http://", "https://"];
  it.each(forbidden)("no %s in the launch screens or the card", (needle) => {
    expect(Object.entries(sources).filter(([, text]) => text.includes(needle)).map(([path]) => path)).toEqual([]);
  });
});

const WORDS = ["abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd", "abuse", "access", "accident"].flatMap((w) => [w, `${w}s`]);

describe("the words and the QR", () => {
  it("numbers the 24 words 1. to 24. with no way to copy them", () => {
    const html = renderToStaticMarkup(<WordsGrid words={WORDS} />);
    expect(html.match(/<li>/g)?.length).toBe(24);
    expect(html).toContain(">1.</span> abandon");
    expect(html).toContain(">24.</span> accidents");
    expect(html).not.toMatch(/<button|draggable|copy/i);
  });

  it("the QR is the words joined by single spaces, with a 4-module quiet zone, drawn as one path", () => {
    const qr = qrOfWords(WORDS);
    expect(qr.size).toBeGreaterThan(2 * QUIET_ZONE + 21);
    expect(qr.path).toMatch(/^M\d+ \d+h1v1h-1z/);
    expect(qrOfWords(WORDS)).toEqual(qr);
    expect(qrOfWords([...WORDS.slice(0, 23), "zoo"]).path).not.toBe(qr.path);
    const html = renderToStaticMarkup(<WordsQr words={WORDS} px={150} />);
    expect(html).toContain('width="150"');
    expect(html).toContain('aria-label="QR code of the 24 words"');
  });

  it("the printed page has the title, the notice, the words, a 200 px QR and the footer", () => {
    const html = renderToStaticMarkup(<PrintPage words={WORDS} />);
    for (const s of ["Whimbrel recovery words", "These words are the key itself.", ">24.</span> accidents", 'width="200"', "Keep this page somewhere safe and offline."]) {
      expect(html).toContain(s);
    }
  });
});

describe("the printed page takes optional words", () => {
  it("with words it prints those; without, the page is unchanged", () => {
    const html = renderToStaticMarkup(<PrintPage words={WORDS} copy={{ notice: "NOTICE LINE", footer: "FOOTER LINE" }} />);
    expect(html).toContain("NOTICE LINE");
    expect(html).toContain("FOOTER LINE");
    expect(html).not.toContain("memory database");
    const plain = renderToStaticMarkup(<PrintPage words={WORDS} />);
    expect(plain.match(/memory database/g)?.length).toBe(2);
  });
});

describe("the setup screen", () => {
  const model = () =>
    new SetupModel({
      begin: vi.fn(),
      saveFile: vi.fn(),
      verify: vi.fn(),
      startOver: vi.fn(),
      print: vi.fn(),
      reusesKey: false,
    });

  it("before Create: the passphrase step, the lost-both caption, Continue off, and no words", () => {
    const html = renderToStaticMarkup(<SetupView model={model()} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} />);
    for (const s of ["Create your recovery key", "1. Choose a passphrase", "0 of at least 12 characters", "Create recovery key", "nobody can unlock the memory database", "Quit Whimbrel"]) {
      expect(html).toContain(s);
    }
    expect(html).not.toContain("3. Write down your 24 recovery words");
    expect(html).toMatch(/<button class="mode mode--active" disabled="">Continue<\/button>/);
  });

  it("after Delete everything: the notice comes first, above the title; never without one", () => {
    const line = "Your previous data was deleted. This is a new, empty library.";
    const html = renderToStaticMarkup(<SetupView model={model()} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} notice={line} />);
    expect(html.indexOf(line)).toBeGreaterThan(-1);
    expect(html.indexOf(line)).toBeLessThan(html.indexOf("Create your recovery key"));
    expect(renderToStaticMarkup(<SetupView model={model()} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} />)).not.toContain(line);
  });

  it("embedded (the redesigned Settings card): the title is a real heading below the card's, and no section is a card inside the card", () => {
    const html = renderToStaticMarkup(
      <SetupView model={model()} continueTitle="Encrypt" onContinue={() => {}} onCancel={() => {}} embedded words={{ intro: "I", sameDisk: "S", wordsWarning: "W", footer: "F", printNotice: "N", printFooter: "P" }} />,
    );
    expect(html).toMatch(/<h4[^>]*>Create your recovery key<\/h4>/);
    expect(html).not.toContain("<h1");
    expect(html).not.toMatch(/class="card/);
    expect(html).toMatch(/<h5[^>]*>1\. Choose a passphrase<\/h5>/);
    // The launch screen and the old tab keep their own look.
    const launch = renderToStaticMarkup(<SetupView model={model()} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} />);
    expect(launch).toMatch(/<h1 class="display">Create your recovery key<\/h1>/);
    expect(launch).toMatch(/class="card stack-sm"/);
  });

  it("Settings: Encrypt the memory database and Cancel, no Quit", () => {
    const html = renderToStaticMarkup(<SetupView model={model()} continueTitle="Encrypt the memory database" onContinue={() => {}} onCancel={() => {}} />);
    expect(html).toContain("Encrypt the memory database");
    expect(html).toContain(">Cancel<");
    expect(html).not.toContain("Quit Whimbrel");
  });
});

describe("no-store mode", () => {
  it("a recording start is refused without the database; Stop still works", async () => {
    const { recordButtonDisabled } = await import("../meetings/recordingModel");
    expect(recordButtonDisabled("start", false, false)).toBe(false);
    expect(recordButtonDisabled("start", false, true)).toBe(true);
    expect(recordButtonDisabled("stop", false, true)).toBe(false);
  });

  it("the banner says recording is off and opens Settings", () => {
    const html = renderToStaticMarkup(<NoStoreBanner onOpenSettings={() => {}} />);
    expect(html).toContain("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.");
    expect(html).toContain("Open Settings");
  });
});
