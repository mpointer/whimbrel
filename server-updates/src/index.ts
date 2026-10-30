import type { AnalyticsEngineDataset, R2Bucket } from "@cloudflare/workers-types";

export interface Env {
  UPDATES_BUCKET: R2Bucket;
  /** Weekly unique desktop clients; see scripts/weekly-clients.sql. */
  CLIENTS: AnalyticsEngineDataset;
}

/** Serves the update manifest and release zip straight out of R2 — no
 * database, no KV, matching this Worker's deliberately minimal scope (see
 * docs/superpowers/specs/2026-08-06-auto-update-downloads-design.md).
 * release.sh writes both objects directly via `wrangler r2 object put`;
 * this Worker only ever reads. */
/** Published objects, by request path → R2 key and content-type. The Mac reads
 * `version.json` and `Whimbrel-<v>.zip`; Windows (M3.5) reads `windows/latest.json`
 * (tauri-plugin-updater's signed manifest) and the NSIS installer it points at. */
const ROUTES: { path: RegExp; contentType: string }[] = [
  { path: /^\/version\.json$/, contentType: "application/json" },
  { path: /^\/Whimbrel-[\w.-]+\.zip$/, contentType: "application/zip" },
  { path: /^\/windows\/latest\.json$/, contentType: "application/json" },
  { path: /^\/windows\/Whimbrel_[\w.-]+_x64-setup\.exe$/, contentType: "application/octet-stream" },
];

const MANIFESTS = new Set(["/version.json", "/windows/latest.json"]);

/** Counts one manifest poll from a current build. Old builds send no headers and
 * a bad header is ignored, so neither is counted; analytics never touches the response. */
function countClient(request: Request, env: Env): void {
  const client = request.headers.get("X-Whimbrel-Client");
  const platform = request.headers.get("X-Whimbrel-Platform");
  const version = request.headers.get("X-Whimbrel-Version");
  if (!client || !platform || !version) return;
  if (!/^[0-9a-f]{32}$/.test(client) || !/^(mac|windows)$/.test(platform) || !/^\d+\.\d+\.\d+$/.test(version)) return;
  try {
    env.CLIENTS.writeDataPoint({ blobs: [client, platform, version], indexes: [client] });
  } catch {
    // Analytics is best-effort.
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const route = ROUTES.find((r) => r.path.test(url.pathname));
    if (!route) {
      return new Response("Not Found", { status: 404 });
    }
    if (request.method !== "GET") {
      return new Response("Method Not Allowed", { status: 405 });
    }
    if (MANIFESTS.has(url.pathname)) countClient(request, env);
    const object = await env.UPDATES_BUCKET.get(url.pathname.slice(1)); // strip leading "/"
    if (!object) {
      return new Response("Not Found", { status: 404 });
    }
    return new Response(object.body, { headers: { "content-type": route.contentType } });
  },
};
