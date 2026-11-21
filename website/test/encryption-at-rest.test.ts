import { describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";

const UPDATES = "https://updates.whimbrelapp.com";

async function page(path: string): Promise<{ status: number; text: string }> {
  const ctx = createExecutionContext();
  const e: Env = { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: UPDATES };
  const res = await handle(new Request(`https://whimbrel.test${path}`), e, ctx, (async () => new Response("no", { status: 404 })) as typeof fetch);
  await waitOnExecutionContext(ctx);
  return { status: res.status, text: await res.text() };
}

function words(html: string): string {
  return html
    .replace(/<script[\s\S]*?<\/script>/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&#?\w+;/g, " ")
    .replace(/[‘’]/g, "'")
    .replace(/[“”]/g, '"')
    .replace(/\s+/g, " ")
    .toLowerCase();
}

function everything(html: string): string {
  const attributes = [...html.matchAll(/\b(?:content|alt|title|aria-label)="([^"]*)"/g)].map((m) => m[1]);
  const scripts = [...html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1].replace(/\\u0027/g, "'"));
  return words([html.replace(/<script[\s\S]*?<\/script>/g, " "), ...attributes, ...scripts].join(" . "));
}

describe("encryption page", () => {
  it("exists, is indexable, and is in the sitemap", async () => {
    const { status, text } = await page("/features/encryption-at-rest/");
    expect(status).toBe(200);
    expect(text).not.toMatch(/<meta name="robots" content="noindex/);
    const map = await page("/sitemap.xml");
    expect(map.text).toContain("/features/encryption-at-rest/");
  });

  it("is linked from the homepage, the footer and the confidential-work page", async () => {
    for (const path of ["/", "/features/meeting-notes/", "/for/confidential-work/"]) {
      const { text } = await page(path);
      expect(text, path).toContain('href="/features/encryption-at-rest/"');
    }
    const home = await page("/");
    expect(home.text).toContain('href="/for/client-controls/"');
    const cc = await page("/for/confidential-work/");
    expect(cc.text).toContain('href="/for/client-controls/"');
  });

  it("carries the cleared claims", async () => {
    const t = words((await page("/features/encryption-at-rest/")).text);
    for (const s of [
      "the memory database can be encrypted on your computer",
      "keychain on the mac and in credential manager on windows",
      "a recovery file with a passphrase, plus 24 words",
      "keep them somewhere other than this computer",
      "if the key is lost (a new computer, a reset mac keychain, or a new windows profile)",
      "only the memory database is encrypted.",
      "audio and other files are not encrypted",
      "phones are not covered",
      "on the mac it first offers export everything",
      "the 24 words are the key itself",
      "the database cannot be opened, by you or by whimbrel",
      "whimbrel keeps a token count, not what you asked",
      "time machine, migration assistant",
      "roaming windows profile",
      "if the key is lost (a new computer, a reset mac keychain, or a new windows profile)",
      "delete everything also removes the database keys",
      "faster search (uses more memory)",
      "check encryption speed",
      "recordings, transcripts and history are stored on your computer, not on our servers",
            "does not protect against someone using your unlocked computer",
    ]) {
      expect(t, `required: ${s}`).toContain(s);
    }
  });

  it("makes none of the banned claims, anywhere on the page", async () => {
    const all = everything((await page("/features/encryption-at-rest/")).text);
    for (const phrase of [
      "nothing leaves",
      "never leaves",
      "never leave",
      "we never receive",
      "encrypted at rest",
      "safe from malware",
      "nobody can read",
      "unbreakable",
      "audio is encrypted",
      "recordings are encrypted",
      "phones are encrypted",
      "phone is encrypted",
      "protects against someone using",
      "all your data is encrypted",
      "stays in your",
      "stays on this",
      "library is encrypted",
      "everything is encrypted",
      "files are encrypted",
      "which offers export everything first",
      "part of the free app",
      "your library",
      "your database is encrypted",
    ]) {
      expect(all, `forbidden phrase: ${phrase}`).not.toContain(phrase);
    }
    // No competitor claims (the footer's compare links are shared chrome, so check the page itself).
    const main = words((await page("/features/encryption-at-rest/")).text.match(/<main[\s\S]*<\/main>/)?.[0] ?? "");
    expect(main.length).toBeGreaterThan(0);
    expect(main).not.toMatch(/\b(otter|fireflies|granola|fathom|read\.ai|rewind|littlebird)\b/);
    // No performance numbers: the only figures allowed are the 24 words and the 16 GB / 1 GB memory settings.
    expect(all).not.toMatch(/\d+\s?(%|ms|x faster|times)/);
  });

  it("the homepage, confidential-work and compare pages make no banned encryption or client-controls claim", async () => {
    for (const path of ["/", "/for/confidential-work/", "/compare/fellow-alternative/", "/compare/voicenotes-alternative/"]) {
      const all = everything((await page(path)).text);
      for (const phrase of [
        "your database is encrypted",
        "encrypted at rest yet",
        "database is not encrypted",
        "relies on your computer's disk encryption",
        "stays in your",
        "library is encrypted",
        "everything is encrypted",
        "files are encrypted",
        "phones are encrypted",
        "nothing leaves",
        "never leaves",
        "we never receive",
        "no trace",
        "completely erased",
        "permanently erased",
        "everything is deleted",
      ]) {
        expect(all, `${path}: forbidden phrase: ${phrase}`).not.toContain(phrase);
      }
    }
    const home = everything((await page("/")).text);
    expect(home).toContain("delete, keep for a set time, export, or exclude them from hosted ai models");
    const cc = everything((await page("/for/confidential-work/")).text);
    expect(cc).toContain("filevault on a mac or bitlocker on windows; it also covers recordings");
  });
});
