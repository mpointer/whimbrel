import { describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";

const UPDATES = "https://updates.whimbrelapp.com";

async function page(path: string): Promise<string> {
  const ctx = createExecutionContext();
  const e: Env = { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: UPDATES };
  const res = await handle(new Request(`https://whimbrel.test${path}`), e, ctx, (async () => new Response("no", { status: 404 })) as typeof fetch);
  await waitOnExecutionContext(ctx);
  expect(res.status).toBe(200);
  return res.text();
}

function meta(html: string, attr: "property" | "name", key: string): string | undefined {
  const m = html.match(new RegExp(`<meta ${attr}="${key}" content="([^"]*)"`));
  return m?.[1];
}

const languages = [
  { path: "/", lang: "en", locale: "en_US", thanks: "/download/thanks/", copy: "Copy message" },
  { path: "/es/", lang: "es", locale: "es_LA", thanks: "/es/download/thanks/", copy: "Copiar mensaje" },
  { path: "/fr/", lang: "fr-CA", locale: "fr_CA", thanks: "/fr/download/thanks/", copy: "Copier le message" },
];

describe("link preview tags on the home page, in three languages", () => {
  for (const l of languages) {
    it(`${l.lang} has its own og and twitter tags`, async () => {
      const html = await page(l.path);
      expect(html).toContain(`<html lang="${l.lang}"`);
      const title = meta(html, "property", "og:title");
      const description = meta(html, "property", "og:description");
      expect(title && title.length).toBeGreaterThan(10);
      expect(description && description.length).toBeGreaterThanOrEqual(70);
      expect(meta(html, "property", "og:type")).toBe("website");
      expect(meta(html, "property", "og:site_name")).toBe("Whimbrel");
      expect(meta(html, "property", "og:locale")).toBe(l.locale);
      expect(meta(html, "property", "og:url")).toBe(`https://whimbrelapp.com${l.path}`);
      expect(meta(html, "property", "og:image")).toBe("https://whimbrelapp.com/assets/og-image.png");
      expect(meta(html, "property", "og:image:width")).toBe("1200");
      expect(meta(html, "property", "og:image:height")).toBe("630");
      expect(meta(html, "name", "twitter:card")).toBe("summary_large_image");
      expect(meta(html, "name", "twitter:title")).toBe(title);
      expect(meta(html, "name", "twitter:description")).toBe(description);
      expect(meta(html, "name", "twitter:image")).toBe(meta(html, "property", "og:image"));
    });
  }

  it("uses a different title and description in each language", async () => {
    const pages = await Promise.all(languages.map((l) => page(l.path)));
    const titles = pages.map((h) => meta(h, "property", "og:title"));
    const descriptions = pages.map((h) => meta(h, "property", "og:description"));
    expect(new Set(titles).size).toBe(3);
    expect(new Set(descriptions).size).toBe(3);
  });
});

describe("share block", () => {
  for (const l of languages) {
    for (const path of [l.path, l.thanks]) {
      it(`${path} has the message, the link and a copy button`, async () => {
        const html = await page(path);
        expect(html).toContain("data-share-message");
        expect(html).toContain(`data-share-button hidden>${l.copy}</button>`);
        const message = html.match(/data-share-message>([^<]*)</)?.[1] ?? "";
        expect(message).toContain("https://whimbrelapp.com");
        expect(message).not.toMatch(/[–—]/);
        // A plain link only: no code, no tracking parameter.
        expect(message).not.toMatch(/[?&=]/);
      });
    }
  }
});
