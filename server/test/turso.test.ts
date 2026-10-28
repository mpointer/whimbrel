import { it, expect, vi } from "vitest";
import { mintDeviceToken, TursoApiError, createAccountDatabase, rotateAllTokens } from "../src/turso";

const CONFIG = {
  platformApiToken: "test-platform-token",
  organizationSlug: "test-org",
  databaseName: "test-db",
};

it("mints a token and returns the jwt from a successful response", async () => {
  const fetchImpl = vi.fn(async (url: string | URL, init?: RequestInit) => {
    expect(String(url)).toBe(
      "https://api.turso.tech/v1/organizations/test-org/databases/test-db/auth/tokens?expiration=1d&authorization=full-access",
    );
    expect(init?.method).toBe("POST");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer test-platform-token");
    return new Response(JSON.stringify({ jwt: "minted-jwt-value" }), { status: 200 });
  });

  const token = await mintDeviceToken(CONFIG, "1d", fetchImpl as unknown as typeof fetch);

  expect(token).toBe("minted-jwt-value");
  expect(fetchImpl).toHaveBeenCalledOnce();
});

it("throws TursoApiError with the response body on a 400", async () => {
  const fetchImpl = vi.fn(async () => new Response("invalid expiration format", { status: 400 }));

  await expect(mintDeviceToken(CONFIG, "not-a-duration", fetchImpl as unknown as typeof fetch)).rejects.toThrow(
    TursoApiError,
  );
});

it("throws TursoApiError on a 404 (database not found)", async () => {
  const fetchImpl = vi.fn(
    async () => new Response(JSON.stringify({ error: "could not find database" }), { status: 404 }),
  );

  await expect(mintDeviceToken(CONFIG, "1d", fetchImpl as unknown as typeof fetch)).rejects.toThrow(TursoApiError);
});

const DATABASE_CONFIG = {
  platformApiToken: "test-platform-token",
  organizationSlug: "test-org",
  group: "test-group",
};

it("creates a database and derives its connection url from the returned hostname", async () => {
  const fetchImpl = vi.fn(async (url: string | URL, init?: RequestInit) => {
    expect(String(url)).toBe("https://api.turso.tech/v1/organizations/test-org/databases");
    expect(init?.method).toBe("POST");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer test-platform-token");
    expect((init?.headers as Record<string, string>)["Content-Type"]).toBe("application/json");
    expect(JSON.parse(init?.body as string)).toEqual({ name: "kestrel-sync-abc123", group: "test-group" });
    return new Response(
      JSON.stringify({ database: { Name: "kestrel-sync-abc123", Hostname: "kestrel-sync-abc123-test-org.turso.io" } }),
      { status: 200 },
    );
  });

  const result = await createAccountDatabase(DATABASE_CONFIG, "kestrel-sync-abc123", fetchImpl as unknown as typeof fetch);

  expect(result).toEqual({
    name: "kestrel-sync-abc123",
    url: "https://kestrel-sync-abc123-test-org.turso.io",
  });
});

it("throws TursoApiError with the response body on a non-2xx response", async () => {
  const fetchImpl = vi.fn(async () => new Response("database name already exists", { status: 409 }));

  await expect(
    createAccountDatabase(DATABASE_CONFIG, "kestrel-sync-abc123", fetchImpl as unknown as typeof fetch),
  ).rejects.toThrow(TursoApiError);
});

it("rotates all tokens for a database", async () => {
  const fetchImpl = vi.fn(async (url: string | URL, init?: RequestInit) => {
    expect(String(url)).toBe("https://api.turso.tech/v1/organizations/test-org/databases/test-db/auth/rotate");
    expect(init?.method).toBe("POST");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer test-platform-token");
    return new Response(null, { status: 200 });
  });

  await rotateAllTokens(CONFIG, fetchImpl as unknown as typeof fetch);

  expect(fetchImpl).toHaveBeenCalledOnce();
});

it("throws TursoApiError with the response body on a non-2xx response from rotate", async () => {
  const fetchImpl = vi.fn(async () => new Response("database not found", { status: 404 }));

  await expect(rotateAllTokens(CONFIG, fetchImpl as unknown as typeof fetch)).rejects.toThrow(TursoApiError);
});
