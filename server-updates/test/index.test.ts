import { it, expect, beforeEach } from "vitest";
import worker from "../src/index";
import type { Env } from "../src/index";
import { env } from "cloudflare:test";

function makeEnv(): Env {
  return { UPDATES_BUCKET: env.UPDATES_BUCKET };
}

beforeEach(async () => {
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
