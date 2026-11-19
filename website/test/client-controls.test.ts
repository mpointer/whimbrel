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

/** The visible words of the page: tags dropped, entities and curly quotes folded, case and spacing flattened. */
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

/**
 * Every word the page puts out, chrome included: the visible text, the <title>, the meta description and the other
 * meta and attribute text, and the JSON-LD (where the FAQ data goes). Lower case, curly quotes folded.
 */
function everything(html: string): string {
  const attributes = [...html.matchAll(/\b(?:content|alt|title|aria-label)="([^"]*)"/g)].map((m) => m[1]);
  const scripts = [...html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1].replace(/\\u0027/g, "'"));
  return words([html.replace(/<script[\s\S]*?<\/script>/g, " "), ...attributes, ...scripts].join(" . "));
}

describe("client controls page", () => {
  it("is kept out of search until the results matrix is measured", async () => {
    const { text } = await page("/for/client-controls/");
    expect(text).toMatch(/<meta name="robots" content="noindex/);
  });

  it("makes no forbidden claim anywhere on the page: title, description, FAQ data and chrome included", async () => {
    const { text } = await page("/for/client-controls/");
    const all = everything(text);
    expect(all).toContain("client controls: delete, keep, export, exclude");
    expect(all).toContain("does the delete reach my other devices?");
    for (const phrase of [
      "all your devices",
      "every device",
      "no trace",
      "stays on your computer",
      // The shared tagline may say it only for Local-only mode (checked below).
      // No retention removes routine results or people's names: never promise it.
      "own retention",
      "never leaves",
      "never leave",
      "nothing leaves",
      "we never receive",
      "everything",
      "time machine",
      "completely erased",
      "permanently erased",
    ]) {
      expect(all, `forbidden phrase anywhere on the page: ${phrase}`).not.toContain(phrase);
    }
    for (const m of all.matchAll(/[^.]*\bstays? on your computer\b[^.]*/g)) {
      expect(m[0], "stay on your computer only for Local-only mode").toContain("in local-only mode");
    }
    for (const m of all.matchAll(/[^.]*\bbackups?\b[^.]*\./g)) {
      expect(m[0], "backups only mentioned as not covered").toMatch(/\b(not|cannot|can't|does not|do not)\b/);
    }
  });

  it("says what stays on other devices, the matcher's limit, the held-back setting, and that only the Mac saves a receipt", async () => {
    const { text } = await page("/for/client-controls/");
    const all = everything(text);
    for (const sentence of [
      // B2: applying the delete does not remove these on other devices.
      "routine results, profile suggestions, people's names and meeting briefs that named the client can stay",
      "the delete does not remove them there",
      "some derived copies stay on your other devices; see below",
      "if new email or a new meeting later matches",
      // Major 5: whole-word matching.
      "a name inside another word, or in text written without spaces (some inflected forms, some scripts), may not match",
      // Major 22: a device that has not updated holds the setting back.
      "your other devices could still send the client's material to a hosted model",
      // Windows has no Save receipt.
      "on a mac you can save it as a file",
    ]) {
      expect(all, `required: ${sentence}`).toContain(sentence);
    }
    expect(all).not.toMatch(/(?<!on a mac )you can save it/);
  });


  it("client_controls_page_makes_no_forbidden_claims", async () => {
    const { status, text } = await page("/for/client-controls/");
    expect(status).toBe(200);
    // The page itself; the header and footer are site chrome shared with every page.
    const main = text.match(/<main[\s\S]*<\/main>/)?.[0] ?? "";
    expect(main.length).toBeGreaterThan(0);
    const t = words(main);

    // Claims the product cannot back.
    for (const phrase of [
      "nothing leaves",
      "never leaves",
      "never leave",
      "we never receive",
      "everything is deleted",
      "everything is removed",
      "time machine",
      "backups are deleted",
      "backups are removed",
      "cloud copies are deleted",
      "mailbox originals are deleted",
      "completely erased",
      "permanently erased",
      // The control's old name, which over-claimed: it is "Delete this client" in the app.
      "delete everything about this client",
    ]) {
      expect(t, `forbidden phrase: ${phrase}`).not.toContain(phrase);
    }
    // Any mention of backups must be to say they are not covered.
    for (const m of t.matchAll(/[^.]*\bbackups?\b[^.]*\./g)) {
      expect(m[0], "backups only mentioned as not covered").toMatch(/\b(not|cannot|can't|does not|do not)\b/);
    }
    // No competitor claims without a source.
    expect(t).not.toMatch(/\b(otter|fireflies|granola|fathom|read\.ai)\b/);

    // The wording the spec allows, verbatim.
    for (const sentence of [
      "in one step on current devices, with a receipt of what was removed and what you still need to check",
      "from the moment whimbrel files it under the client, and new material is checked locally first",
      "material that names or involves the client",
      // The control's name in the app.
      "delete this client",
    ]) {
      expect(t, `required: ${sentence}`).toContain(sentence);
    }
  });
});
