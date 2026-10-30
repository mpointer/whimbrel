import { it, expect, beforeEach } from "vitest";
import worker from "../src/index";
import type { Env } from "../src/index";
import { env } from "cloudflare:test";

// Fake Analytics Engine binding: records every data point the Worker writes.
let points: { blobs?: string[]; indexes?: string[] }[] = [];
let failWrites = false;

function makeEnv(): Env {
  return {
    UPDATES_BUCKET: env.UPDATES_BUCKET,
    CLIENTS: {
      writeDataPoint(p: { blobs?: string[]; indexes?: string[] }) {
        if (failWrites) throw new Error("analytics down");
        points.push(p);
      },
    },
  };
}

const CLIENT = "0123456789abcdef0123456789abcdef";
const headers = (over: Record<string, string> = {}) => ({
  "X-Whimbrel-Client": CLIENT,
  "X-Whimbrel-Platform": "mac",
  "X-Whimbrel-Version": "0.9.35",
  ...over,
});

beforeEach(async () => {
  points = [];
  failWrites = false;
  // Miniflare's R2 emulation persists objects across tests within a run unless
  // explicitly cleared — start each test from a known-empty bucket.
  const list = await env.UPDATES_BUCKET.list();
  for (const obj of list.objects) {
    await env.UPDATES_BUCKET.delete(obj.key);
  }
});

it("returns 404 for version.json when nothing has been published yet", async () => {
  const response = await worker.fetch(new Request("https://example.com/version.json"), makeEnv());
  expect(response.status).toBe(404);
});

it("serves version.json once published", async () => {
  await env.UPDATES_BUCKET.put("version.json", JSON.stringify({ version: "0.3.0", url: "https://example.com/Whimbrel-0.3.0.zip" }));
  const response = await worker.fetch(new Request("https://example.com/version.json"), makeEnv());
  expect(response.status).toBe(200);
  expect(response.headers.get("content-type")).toBe("application/json");
  const body = await response.json();
  expect(body).toEqual({ version: "0.3.0", url: "https://example.com/Whimbrel-0.3.0.zip" });
});

it("returns 404 for a zip that hasn't been uploaded", async () => {
  const response = await worker.fetch(new Request("https://example.com/Whimbrel-0.3.0.zip"), makeEnv());
  expect(response.status).toBe(404);
});

it("streams an uploaded zip with the right content-type", async () => {
  const bytes = new TextEncoder().encode("fake zip bytes");
  await env.UPDATES_BUCKET.put("Whimbrel-0.3.0.zip", bytes);
  const response = await worker.fetch(new Request("https://example.com/Whimbrel-0.3.0.zip"), makeEnv());
  expect(response.status).toBe(200);
  expect(response.headers.get("content-type")).toBe("application/zip");
  const received = new Uint8Array(await response.arrayBuffer());
  expect(received).toEqual(bytes);
});

it("returns 404 for an unrouted path", async () => {
  const response = await worker.fetch(new Request("https://example.com/nope"), makeEnv());
  expect(response.status).toBe(404);
});

it("returns 405 for a non-GET request to a known route", async () => {
  const response = await worker.fetch(new Request("https://example.com/version.json", { method: "POST" }), makeEnv());
  expect(response.status).toBe(405);
});

it("serves the Windows manifest and installer once published", async () => {
  await env.UPDATES_BUCKET.put("windows/latest.json", JSON.stringify({ version: "0.9.8" }));
  await env.UPDATES_BUCKET.put("windows/Whimbrel_0.9.8_x64-setup.exe", new TextEncoder().encode("MZ"));
  const manifest = await worker.fetch(new Request("https://example.com/windows/latest.json"), makeEnv());
  expect(manifest.status).toBe(200);
  expect(manifest.headers.get("content-type")).toBe("application/json");
  expect(await manifest.json()).toEqual({ version: "0.9.8" });
  const installer = await worker.fetch(new Request("https://example.com/windows/Whimbrel_0.9.8_x64-setup.exe"), makeEnv());
  expect(installer.status).toBe(200);
  expect(new TextDecoder().decode(await installer.arrayBuffer())).toBe("MZ");
});

it("serves nothing else under windows/", async () => {
  await env.UPDATES_BUCKET.put("windows/other.txt", "x");
  const response = await worker.fetch(new Request("https://example.com/windows/other.txt"), makeEnv());
  expect(response.status).toBe(404);
});

it("a manifest request with a client id writes one data point", async () => {
  await env.UPDATES_BUCKET.put("version.json", "{}");
  await env.UPDATES_BUCKET.put("windows/latest.json", "{}");
  await worker.fetch(new Request("https://example.com/version.json", { headers: headers() }), makeEnv());
  expect(points).toEqual([{ blobs: [CLIENT, "mac", "0.9.35"], indexes: [CLIENT] }]);
  await worker.fetch(new Request("https://example.com/windows/latest.json", { headers: headers({ "X-Whimbrel-Platform": "windows" }) }), makeEnv());
  expect(points).toHaveLength(2);
  expect(points[1].blobs![1]).toBe("windows");
});

it("a malformed client id is served but not counted", async () => {
  await env.UPDATES_BUCKET.put("version.json", "{}");
  for (const bad of <Record<string, string>[]>[
    { "X-Whimbrel-Client": "ABCDEF0123456789abcdef0123456789" },
    { "X-Whimbrel-Client": "abc" },
    { "X-Whimbrel-Platform": "ios" },
    { "X-Whimbrel-Version": "1.2" },
  ]) {
    const response = await worker.fetch(new Request("https://example.com/version.json", { headers: headers(bad) }), makeEnv());
    expect(response.status).toBe(200);
  }
  expect(points).toEqual([]);
});

it("binaries are not counted", async () => {
  await env.UPDATES_BUCKET.put("Whimbrel-0.3.0.zip", "z");
  await env.UPDATES_BUCKET.put("windows/Whimbrel_0.9.8_x64-setup.exe", "MZ");
  await worker.fetch(new Request("https://example.com/Whimbrel-0.3.0.zip", { headers: headers() }), makeEnv());
  await worker.fetch(new Request("https://example.com/windows/Whimbrel_0.9.8_x64-setup.exe", { headers: headers() }), makeEnv());
  expect(points).toEqual([]);
});

it("no header is served unchanged", async () => {
  await env.UPDATES_BUCKET.put("version.json", '{"version":"0.3.0"}');
  const response = await worker.fetch(new Request("https://example.com/version.json"), makeEnv());
  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ version: "0.3.0" });
  expect(points).toEqual([]);
});

it("a failing data point write does not affect the response", async () => {
  await env.UPDATES_BUCKET.put("version.json", "{}");
  failWrites = true;
  const response = await worker.fetch(new Request("https://example.com/version.json", { headers: headers() }), makeEnv());
  expect(response.status).toBe(200);
});
