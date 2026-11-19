import type { Platform } from "./leads";

/** What a download link asks for: a platform's default build, or the Windows on Arm (arm64)
 * installer, which lives in the same Windows manifest under `windows-aarch64`. */
export type DownloadTarget = Platform | "windows-arm";

export interface Release {
  version: string | null;
  url: string;
}

/** Where each platform's current release is described. These are the same manifests the
 * apps' own auto-updaters read, so the website can never offer a build the updater
 * doesn't know about. */
const MANIFEST_PATH: Record<DownloadTarget, string> = {
  mac: "/version.json", // { version, url, sha256, ... } written by apps/macos/scripts/release-publish.sh
  windows: "/windows/latest.json", // tauri-plugin-updater format, written by release-windows.yml
  "windows-arm": "/windows/latest.json", // the same manifest's windows-aarch64 entry
};

/** The platform a download is recorded under (the leads tables know only mac and windows). */
export function platformOf(target: DownloadTarget): Platform {
  return target === "windows-arm" ? "windows" : target;
}

function urlFromManifest(platform: DownloadTarget, manifest: unknown): { version: string | null; url: unknown } {
  const m = manifest as Record<string, unknown> | null;
  if (!m || typeof m !== "object") return { version: null, url: null };
  const version = typeof m.version === "string" ? m.version : null;
  if (platform === "mac") return { version, url: m.url };
  const platforms = m.platforms as Record<string, { url?: unknown }> | undefined;
  return { version, url: platforms?.[platform === "windows-arm" ? "windows-aarch64" : "windows-x86_64"]?.url };
}

/** Reads the platform's manifest from the updates Worker and returns the installer URL.
 * The URL must point back at the updates Worker: its own host, or one of `otherHosts`
 * (the same Worker's custom domain, updates.whimbrelapp.com). A manifest naming any other
 * host is treated as broken rather than redirected to, so this route can never become
 * an open redirect. Returns null when nothing is published or the manifest is malformed. */
export async function resolveRelease(
  platform: DownloadTarget,
  updatesUrl: string,
  fetcher: typeof fetch = fetch,
  otherHosts: readonly string[] = [],
): Promise<Release | null> {
  const base = new URL(updatesUrl);
  let response: Response;
  try {
    response = await fetcher(new URL(MANIFEST_PATH[platform], base).toString(), {
      headers: { "user-agent": "whimbrel-website" },
      cf: { cacheTtl: 300, cacheEverything: true },
    } as RequestInit);
  } catch {
    return null;
  }
  if (!response.ok) return null;

  let manifest: unknown;
  try {
    manifest = await response.json();
  } catch {
    return null;
  }
  const { version, url } = urlFromManifest(platform, manifest);
  if (typeof url !== "string") return null;

  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "https:" || (parsed.host !== base.host && !otherHosts.includes(parsed.host))) return null;
  return { version, url: parsed.toString() };
}
