export interface TursoTokenConfig {
  platformApiToken: string;
  organizationSlug: string;
  databaseName: string;
}

export class TursoApiError extends Error {}

/** Mints a scoped, time-limited Turso database auth token via Turso's
 * Platform API. `expiration` uses Turso's duration format (e.g. "1d",
 * "2w1d30m"). Tokens are always full-access — read-only scoping isn't
 * needed since a device that can't write can't sync its own captures. */
export async function mintDeviceToken(
  config: TursoTokenConfig,
  expiration: string,
  fetchImpl: typeof fetch = fetch,
): Promise<string> {
  const url =
    `https://api.turso.tech/v1/organizations/${encodeURIComponent(config.organizationSlug)}` +
    `/databases/${encodeURIComponent(config.databaseName)}/auth/tokens` +
    `?expiration=${encodeURIComponent(expiration)}&authorization=full-access`;

  const response = await fetchImpl(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${config.platformApiToken}`,
    },
  });

  if (!response.ok) {
    const body = await response.text();
    throw new TursoApiError(`Turso token mint failed (${response.status}): ${body}`);
  }

  const data = (await response.json()) as { jwt: string };
  return data.jwt;
}

/** Invalidates every currently-issued auth token for the given database in
 * one call — Turso's Platform API has no way to target a single token or
 * device (confirmed against Turso's own docs during design; see
 * docs/superpowers/specs/2026-07-31-device-revocation-and-naming-design.md).
 * This is the only lever it exposes for actually cutting off a device's
 * already-held access, at the cost of every other currently-enrolled
 * device needing to silently re-authenticate on its next sync attempt. */
export async function rotateAllTokens(
  config: TursoTokenConfig,
  fetchImpl: typeof fetch = fetch,
): Promise<void> {
  const url =
    `https://api.turso.tech/v1/organizations/${encodeURIComponent(config.organizationSlug)}` +
    `/databases/${encodeURIComponent(config.databaseName)}/auth/rotate`;

  const response = await fetchImpl(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${config.platformApiToken}`,
    },
  });

  if (!response.ok) {
    const body = await response.text();
    throw new TursoApiError(`Turso token rotation failed (${response.status}): ${body}`);
  }
}

export interface TursoDatabaseConfig {
  platformApiToken: string;
  organizationSlug: string;
  group: string;
}

export interface CreatedDatabase {
  name: string;
  url: string;
}

/** Provisions a new Turso database for an account, in the given pre-existing
 * group (Turso's mechanism for cheaply sharing resources across many
 * per-tenant databases). `group` must already exist — creating a group is a
 * one-time manual provisioning step
 * (`turso group create <name>`), not something this function does. */
export async function createAccountDatabase(
  config: TursoDatabaseConfig,
  databaseName: string,
  fetchImpl: typeof fetch = fetch,
): Promise<CreatedDatabase> {
  const url = `https://api.turso.tech/v1/organizations/${encodeURIComponent(config.organizationSlug)}/databases`;

  const response = await fetchImpl(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${config.platformApiToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ name: databaseName, group: config.group }),
  });

  if (!response.ok) {
    const body = await response.text();
    throw new TursoApiError(`Turso database creation failed (${response.status}): ${body}`);
  }

  const data = (await response.json()) as { database: { Name: string; Hostname: string } };
  return { name: data.database.Name, url: `https://${data.database.Hostname}` };
}
