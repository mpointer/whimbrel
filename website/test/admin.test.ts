import { beforeEach, describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";
import { leadsCsv, type LeadRow } from "../src/admin";

const TEAM = "whimbrel.cloudflareaccess.com";
const AUD = "aud-tag-123";
const NOW = Math.floor(Date.now() / 1000);

function b64url(bytes: Uint8Array | string): string {
  const raw = typeof bytes === "string" ? bytes : String.fromCharCode(...bytes);
  return btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function keyPair() {
  return (await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
}

async function token(privateKey: CryptoKey, claims: Record<string, unknown>, kid = "k1"): Promise<string> {
  const head = b64url(JSON.stringify({ alg: "RS256", kid, typ: "JWT" }));
  const body = b64url(JSON.stringify(claims));
  const sig = new Uint8Array(await crypto.subtle.sign("RSASSA-PKCS1-v1_5", privateKey, new TextEncoder().encode(`${head}.${body}`)));
  return `${head}.${body}.${b64url(sig)}`;
}

let signer: CryptoKeyPair;
let certs: Response;

function fetcher(): typeof fetch {
  return (async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    return url === `https://${TEAM}/cdn-cgi/access/certs` ? certs.clone() : new Response("nope", { status: 404 });
  }) as typeof fetch;
}

const configured: Partial<Env> = { ADMIN_ACCESS_TEAM: TEAM, ADMIN_ACCESS_AUD: AUD, ADMIN_EMAILS: "mike@example.com" };
const good = { aud: [AUD], iss: `https://${TEAM}`, exp: NOW + 600, email: "Mike@Example.com" };

async function get(path: string, opts: { jwt?: string; env?: Partial<Env>; method?: string } = {}) {
  const headers = opts.jwt ? { "cf-access-jwt-assertion": opts.jwt } : undefined;
  const ctx = createExecutionContext();
  const res = await handle(new Request(`https://whimbrel.test${path}`, { method: opts.method ?? "GET", headers }),
    { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: "https://updates.whimbrelapp.com", ...opts.env } as Env, ctx, fetcher());
  await waitOnExecutionContext(ctx);
  return res;
}

async function seed() {
  const old = new Date((NOW - 20 * 86400) * 1000).toISOString(), recent = new Date((NOW - 2 * 86400) * 1000).toISOString();
  await env.DB.batch([
    env.DB.prepare("DELETE FROM beta_signups"), env.DB.prepare("DELETE FROM downloads"), env.DB.prepare("DELETE FROM leads"),
    env.DB.prepare(`INSERT INTO leads (id, email, name, company, platform, created_at, updated_at, utm_source) VALUES
      ('a', 'ada@example.com', 'Ada', 'Analytical', 'mac', ?1, ?1, 'producthunt'),
      ('b', 'bob@example.com', '=HYPERLINK(1)', NULL, 'windows', ?1, ?1, NULL),
      ('c', 'old@example.com', 'Old Lead', NULL, 'mac', ?2, ?2, NULL)`).bind(recent, old),
    env.DB.prepare(`INSERT INTO downloads (lead_id, platform, version, created_at) VALUES ('a', 'mac', '0.9.70', ?1)`).bind(recent),
    env.DB.prepare(`INSERT INTO beta_signups (id, lead_id, email, phones, computer, status, created_at) VALUES
      ('s1', 'a', 'ada@example.com', 'iphone', 'mac', 'approved', ?1)`).bind(recent),
  ]);
}

describe("/admin/leads", () => {
  beforeEach(async () => {
    signer = await keyPair();
    const jwk = (await crypto.subtle.exportKey("jwk", signer.publicKey)) as JsonWebKey;
    certs = Response.json({ keys: [{ ...jwk, kid: "k1" }] });
    await seed();
  });

  it("is 404 until Access is configured, and without a token", async () => {
    expect((await get("/admin/leads", { jwt: await token(signer.privateKey, good) })).status).toBe(404);
    expect((await get("/admin/leads", { env: configured })).status).toBe(404);
  });

  it("shows the last 7 days of leads with their downloads and beta request", async () => {
    const res = await get("/admin/leads", { env: configured, jwt: await token(signer.privateKey, good) });
    expect(res.status).toBe(200);
    expect(res.headers.get("cache-control")).toBe("no-store");
    const html = await res.text();
    expect(html).toContain("2 signed up in the last 7 days; 1 downloaded.");
    expect(html).toContain("mac 0.9.70");
    expect(html).toContain("approved (iphone)");
    expect(html).toContain("producthunt");
    expect(html).not.toContain("old@example.com");
    expect(html).toContain("=HYPERLINK(1)");
    expect(html).not.toContain("<script");
  });

  it("also allows the emails in the ADMIN_EMAILS_EXTRA secret", async () => {
    const jwt = await token(signer.privateKey, { ...good, email: "Second@Example.com" });
    expect((await get("/admin/leads", { env: configured, jwt })).status).toBe(404);
    expect((await get("/admin/leads", { env: { ...configured, ADMIN_EMAILS_EXTRA: "second@example.com" }, jwt })).status).toBe(200);
  });

  it("widens to all time", async () => {
    const html = await (await get("/admin/leads?days=all", { env: configured, jwt: await token(signer.privateKey, good) })).text();
    expect(html).toContain("old@example.com");
    expect(html).toContain("3 signed up in all time");
  });

  it("refuses a wrong audience, an expired token, another email, another key, and a non-GET", async () => {
    const other = await keyPair();
    for (const jwt of [
      await token(signer.privateKey, { ...good, aud: ["someone-else"] }),
      await token(signer.privateKey, { ...good, exp: NOW - 1 }),
      await token(signer.privateKey, { ...good, email: "eve@example.com" }),
      await token(signer.privateKey, { ...good, iss: "https://evil.cloudflareaccess.com" }),
      await token(other.privateKey, good),
      await token(signer.privateKey, good, "unknown-kid"),
      "not.a.jwt",
    ]) {
      expect((await get("/admin/leads", { env: configured, jwt })).status).toBe(404);
    }
    expect((await get("/admin/leads", { env: configured, jwt: await token(signer.privateKey, good), method: "POST" })).status).toBe(405);
  });

  it("exports CSV for the same window", async () => {
    const res = await get("/admin/leads.csv?days=30", { env: configured, jwt: await token(signer.privateKey, good) });
    expect(res.headers.get("content-type")).toContain("text/csv");
    expect(res.headers.get("content-disposition")).toContain("whimbrel-leads-30.csv");
    const csv = await res.text();
    expect(csv.split("\r\n")[0]).toMatch(/^Signed up,Name,Email/);
    expect(csv).toContain("old@example.com");
  });
});

describe("leadsCsv", () => {
  it("never lets a cell start a spreadsheet formula, and quotes commas", () => {
    const row = { email: "a@b.c", name: "=cmd|' /C calc'!A0", company: "Acme, Inc", downloads: 0, registrations: 1,
      marketing_opt_in: 0, platform: "mac", created_at: "2026-11-12T10:00:00Z" } as LeadRow;
    const line = leadsCsv([row]).split("\r\n")[1];
    expect(line).toContain("'=cmd");
    expect(line).toContain('"Acme, Inc"');
  });
});
