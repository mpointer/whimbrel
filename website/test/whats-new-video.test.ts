import { describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";
import site from "../site/config.json";
// The caption source: demo/whats-new.html, the page the video is recorded from.
import captionSource from "../demo/whats-new.html?raw";

const UPDATES = "https://updates.whimbrelapp.com";

async function get(path: string): Promise<Response> {
  const ctx = createExecutionContext();
  const e: Env = { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: UPDATES };
  const res = await handle(new Request(`https://whimbrel.test${path}`), e, ctx, (async () => new Response("no", { status: 404 })) as typeof fetch);
  await waitOnExecutionContext(ctx);
  return res;
}
const page = async (path: string) => (await get(path)).text();

/** Page text plus the data the page carries (JSON-LD, attributes), lower case, curly quotes folded. */
function everything(html: string): string {
  return html
    .replace(/<[^>]+>/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&#?\w+;/g, " ")
    .replace(/\\u0027/g, "'")
    .replace(/[‘’]/g, "'")
    .toLowerCase();
}

/** What the video says: the text of the elements that appear on screen, without the script and the style. */
function captions(source: string): string {
  return everything(source.replace(/<!--[\s\S]*?-->/g, " ").replace(/<style[\s\S]*?<\/style>/g, " ").replace(/<script[\s\S]*?<\/script>/g, " "));
}

const PAGES = ["/features/encryption-at-rest/", "/for/client-controls/"];
const VIDEO = site.whatsNewVideo;

describe("What's new in 0.9.80 video", () => {
  it("is declared in the site config next to the untouched November 11 tour", () => {
    expect(VIDEO.title).toBe("What's new in Whimbrel 0.9.80");
    expect(VIDEO.uploadDate).toBe("2026-11-19T00:00:00-05:00");
    expect(VIDEO.mp4).toBe("/media/whimbrel-whats-new.mp4");
    expect(VIDEO.webm).toBe("/media/whimbrel-whats-new.webm");
    expect(VIDEO.poster).toBe("/media/whimbrel-whats-new-poster.jpg");
    expect(VIDEO.duration).toMatch(/^PT\d+S$/);
    expect(site.demoVideo).toEqual({
      title: "Whimbrel product tour",
      description:
        "A one-minute walkthrough of Whimbrel with Wi-Fi off: a meeting recorded without a bot and transcribed on the computer, its summary, a page for the client, and a cited answer from a local model.",
      mp4: "/media/whimbrel-tour.mp4",
      webm: "/media/whimbrel-tour.webm",
      poster: "/media/whimbrel-tour-poster.jpg",
      uploadDate: "2026-11-11T00:00:00-05:00",
      duration: "PT66S",
    });
  });

  it.each(PAGES)("is embedded on %s, with its own VideoObject and not the tour", async (path) => {
    const html = await page(path);
    expect(html).toContain(`<source src="${VIDEO.webm}" type="video/webm">`);
    expect(html).toContain(`<source src="${VIDEO.mp4}" type="video/mp4">`);
    expect(html).toContain(`poster="${VIDEO.poster}"`);
    expect(html).toContain('"@type":"VideoObject"');
    expect(html).toContain(`"embedUrl":"https://whimbrelapp.com${path}#whats-new"`);
    expect(html).not.toContain(site.demoVideo.mp4);
    // Below the intro, before the questions.
    expect(html.indexOf('id="whats-new"')).toBeGreaterThan(html.indexOf("page-hero"));
    expect(html.indexOf('id="whats-new"')).toBeLessThan(html.indexOf('class="faq"'));
  });

  it("is on no other English page, and the home page still embeds only the November 11 tour", async () => {
    const home = await page("/");
    expect(home).toContain(`<source src="${site.demoVideo.mp4}" type="video/mp4">`);
    expect(home).toContain('"embedUrl":"https://whimbrelapp.com/#demo"');
    expect(home).not.toContain("whimbrel-whats-new");
    for (const path of ["/features/", "/for/confidential-work/", "/download/", "/changelog/", "/pricing/"]) {
      const res = await get(path);
      if (res.status === 200) expect(await res.text(), path).not.toContain("whimbrel-whats-new");
    }
  });

  it("keeps the November 11 tour files in place, and ships the new media under 20 MB each", async () => {
    for (const file of ["/media/whimbrel-tour.mp4", "/media/whimbrel-tour.webm", "/media/whimbrel-tour-poster.jpg"]) {
      expect((await get(file)).status, file).toBe(200);
    }
    for (const file of [VIDEO.mp4, VIDEO.webm, VIDEO.poster]) {
      const res = await get(file);
      expect(res.status, file).toBe(200);
      const size = (await res.arrayBuffer()).byteLength;
      expect(size, file).toBeGreaterThan(10_000);
      expect(size, file).toBeLessThan(20 * 1024 * 1024);
    }
  });

  it("says nothing on the page or in the captions that the cleared pages do not say", async () => {
    const onPage = everything(`${VIDEO.title} ${VIDEO.description}`);
    const spoken = captions(captionSource);
    expect(spoken).toContain("every client page has four controls: delete, keep, export, exclude from hosted models.");
    expect(spoken).toContain("only the memory database is encrypted.");
    expect(spoken).toContain("audio and other files are not.");
    expect(spoken).toContain("phones are unchanged.");
    for (const [where, text] of [["page text", onPage], ["captions", spoken]] as const) {
      for (const phrase of [
        "nothing leaves",
        "never leaves",
        "never leave",
        "we never receive",
        "stays on your computer",
        "no trace",
        "completely erased",
        "unlocked computer is safe",
        "export everything",
        "performance",
        "faster",
        "plus",
        "cloud",
        "$",
        "/month",
        "per month",
        "free trial",
      ]) {
        expect(text, `${where}: ${phrase}`).not.toContain(phrase);
      }
    }
    // The encryption claim never reaches audio, other files or phones.
    for (const m of spoken.matchAll(/[^.]*\b(audio|phones?|other files)\b[^.]*\./g)) {
      expect(m[0], "limits are stated as limits").toMatch(/\b(not|unchanged)\b/);
    }
  });
});
