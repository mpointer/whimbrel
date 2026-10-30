import { it, expect, vi, beforeEach, type MockInstance } from "vitest";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import { runInactivity } from "../src/inactivity";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.UTC(2026, 9, 29, 9, 0, 0); // October 29, 2026 09:00 UTC

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    GOOGLE_CLIENT_ID: "c",
    TURSO_DATABASE_URL: "u",
    TURSO_DATABASE_AUTH_TOKEN: "t",
    TURSO_PLATFORM_API_TOKEN: "p",
    TURSO_ORG_SLUG: "org",
    TURSO_DB_GROUP: "g",
    ALLOWED_GOOGLE_EMAILS: "",
    RESEND_API_KEY: "re_test",
    ENROLLMENT_HANDOFF: undefined as never,
    ...overrides,
  };
}

/** A Resend stand-in that records each email body it was asked to send. */
function resend(ok = true) {
  const sent: { to: string[]; subject: string; text: string; from: string; reply_to?: string }[] = [];
  const fetcher = vi.fn(async (_url: string | URL | Request, init?: RequestInit) => {
    sent.push(JSON.parse(String(init?.body)));
    return new Response("{}", { status: ok ? 200 : 500 });
  }) as unknown as typeof fetch;
  return { sent, fetcher };
}

let del: MockInstance<typeof turso.deleteAccountDatabase>;
beforeEach(() => {
  vi.restoreAllMocks();
  del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
});

/** An account whose only device was last seen `daysAgo` days before NOW. */
async function idle(db: FakeSqlExecutor, daysAgo: number, email = "a@example.com", sub = "sub-a") {
  const account = await accounts.getOrCreateAccount(db, email, sub, 1, `db-${sub}`, `https://db-${sub}`);
  const device = await accounts.enrollDevice(db, account.id, `key-${sub}`, "approved", 1, "mac");
  await accounts.touchLastSeen(db, account.id, device.id, NOW - daysAgo * DAY);
  return { account, device };
}

it("cron warns once after 180 days and records warned_at", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 181);
  await idle(db, 179, "b@example.com", "sub-b");
  const { sent, fetcher } = resend();
  expect(await runInactivity(db, makeEnv(), NOW, fetcher)).toEqual({ warned: 1, deleted: 0, expiredSwaps: 0 });
  expect(sent).toEqual([
    {
      from: "Whimbrel <whimbrel@futuremeanswered.com>",
      to: ["a@example.com"],
      reply_to: "hello@whimbrelapp.com",
      subject: "Your Whimbrel sync data will be deleted on November 12, 2026",
      text:
        "None of your devices has connected to Whimbrel sync for six months. Open Whimbrel on any of your devices " +
        "before November 12, 2026 to keep your synced data. Nothing on your devices themselves will be deleted.",
    },
  ]);
  expect(db.rawAccount(account.id)?.warnedAtUnixMs).toBe(NOW);
});

it("cron does not warn twice in one day", async () => {
  const db = new FakeSqlExecutor();
  await idle(db, 200);
  const { sent, fetcher } = resend();
  await runInactivity(db, makeEnv(), NOW, fetcher);
  expect((await runInactivity(db, makeEnv(), NOW + 60_000, fetcher)).warned).toBe(0);
  expect(sent).toHaveLength(1);
});

it("a failed email leaves warned_at empty", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 200);
  const { fetcher } = resend(false);
  expect((await runInactivity(db, makeEnv(), NOW, fetcher)).warned).toBe(0);
  expect(db.rawAccount(account.id)?.warnedAtUnixMs ?? null).toBeNull();
  // Without a key nothing is sent, so nothing can ever be deleted either.
  expect((await runInactivity(db, makeEnv({ RESEND_API_KEY: undefined }), NOW + 30 * DAY, fetcher)).deleted).toBe(0);
  expect(await accounts.findAccountByGoogleSub(db, "sub-a")).not.toBeNull();
});

it("cron deletes 14 days after a successful warning", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 181);
  const { fetcher } = resend();
  await runInactivity(db, makeEnv(), NOW, fetcher);
  expect((await runInactivity(db, makeEnv(), NOW + 13 * DAY, fetcher)).deleted).toBe(0);
  expect((await runInactivity(db, makeEnv(), NOW + 14 * DAY + 1, fetcher)).deleted).toBe(1);
  expect(del).toHaveBeenCalledWith(expect.anything(), "db-sub-a");
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("contact after the warning cancels deletion", async () => {
  const db = new FakeSqlExecutor();
  const { account, device } = await idle(db, 181);
  const { fetcher } = resend();
  await runInactivity(db, makeEnv(), NOW, fetcher);
  await accounts.touchLastSeen(db, account.id, device.id, NOW + DAY);
  expect((await runInactivity(db, makeEnv(), NOW + 20 * DAY, fetcher)).deleted).toBe(0);
  expect(del).not.toHaveBeenCalled();
});

it("unlimited accounts are never warned", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 400, "Mike@Example.com");
  db.markWarned(account.id, NOW - 30 * DAY); // even a stray warning never deletes it
  const { sent, fetcher } = resend();
  const result = await runInactivity(db, makeEnv({ UNLIMITED_EMAILS: "mike@example.com" }), NOW, fetcher);
  expect(result).toEqual({ warned: 0, deleted: 0, expiredSwaps: 0 });
  expect(sent).toHaveLength(0);
  expect(db.rawAccount(account.id)).toBeDefined();
});

it("null last_seen is backfilled, not deleted", async () => {
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "old@example.com", "sub-old", 1, "n", "u");
  const { sent, fetcher } = resend();
  expect(await runInactivity(db, makeEnv(), NOW, fetcher)).toEqual({ warned: 0, deleted: 0, expiredSwaps: 0 });
  expect(db.rawAccount(account.id)?.lastSeenUnixMs).toBe(NOW);
  expect(sent).toHaveLength(0);
});

it("abandoned swap devices expire after 7 days", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await idle(db, 1);
  const old = await accounts.enrollDevice(db, account.id, "old-swap", "pending", NOW - 8 * DAY, "mac");
  await accounts.setReplacesDevice(db, old.id, mac.id);
  const recent = await accounts.enrollDevice(db, account.id, "recent-swap", "pending", NOW - 2 * DAY, "mac");
  await accounts.setReplacesDevice(db, recent.id, mac.id);
  const plain = await accounts.enrollDevice(db, account.id, "plain", "pending", NOW - 30 * DAY, "ios");
  const { fetcher } = resend();
  expect((await runInactivity(db, makeEnv(), NOW, fetcher)).expiredSwaps).toBe(1);
  expect(db.rawDevice(old.id)).toBeUndefined();
  expect(db.rawDevice(recent.id)).toBeDefined();
  expect(db.rawDevice(plain.id)).toBeDefined();
  expect(db.rawDevice(mac.id)?.status).toBe("approved");
});

it("an expired swap that a live swap still points at is kept", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await idle(db, 1);
  const a = await accounts.enrollDevice(db, account.id, "a", "pending", NOW - 20 * DAY, "mac");
  await accounts.setReplacesDevice(db, a.id, mac.id);
  const b = await accounts.enrollDevice(db, account.id, "b", "pending", NOW - 10 * DAY, "mac");
  await accounts.setReplacesDevice(db, b.id, a.id);
  const c = await accounts.enrollDevice(db, account.id, "c", "pending", NOW - DAY, "mac");
  await accounts.setReplacesDevice(db, c.id, b.id);
  const { fetcher } = resend();
  expect((await runInactivity(db, makeEnv(), NOW, fetcher)).expiredSwaps).toBe(0);
  expect([a, b, c].map((d) => db.rawDevice(d.id) !== undefined)).toEqual([true, true, true]);
});

it("cron returns a stuck resetting device to pending", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await idle(db, 1);
  const stuck = await accounts.enrollDevice(db, account.id, "stuck", "pending", NOW - DAY, "mac");
  await accounts.setReplacesDevice(db, stuck.id, mac.id);
  await accounts.claimSwapForReset(db, stuck.id, NOW - 11 * 60 * 1000);
  const fresh = await accounts.enrollDevice(db, account.id, "fresh", "pending", NOW - DAY, "mac");
  await accounts.setReplacesDevice(db, fresh.id, mac.id);
  await accounts.claimSwapForReset(db, fresh.id, NOW - 60 * 1000);
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(db.rawDevice(stuck.id)?.status).toBe("pending");
  expect(db.rawDevice(stuck.id)?.resetting_since_unix_ms).toBeNull();
  expect(db.rawDevice(fresh.id)?.status).toBe("resetting");
});

it("cron deletes retired databases after 30 days", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 1);
  await accounts.retireDatabase(db, account.id, "old-31", NOW - 31 * DAY);
  await accounts.retireDatabase(db, account.id, "old-29", NOW - 29 * DAY);
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(del.mock.calls.map((c) => c[1])).toEqual(["old-31"]);
  expect(db.rawRetired().map((r) => r.name)).toEqual(["old-29"]);
});

it("a failed retired-database delete keeps its row for the next run", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 1);
  await accounts.retireDatabase(db, account.id, "old-31", NOW - 31 * DAY);
  del.mockRejectedValueOnce(new Error("turso down"));
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(db.rawRetired().map((r) => r.name)).toEqual(["old-31"]);
});

it("cron never deletes an account's current database even if listed as retired", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 1);
  await accounts.retireDatabase(db, account.id, "db-sub-a", NOW - 40 * DAY);
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(del).not.toHaveBeenCalled();
  expect(db.rawRetired()).toHaveLength(1);
});

it("cron sends the reset notice once and clears it", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 1);
  const resetAt = Date.UTC(2026, 9, 28, 14, 5);
  await accounts.setAccountDatabase(db, account.id, "new-db", "https://new-db", resetAt);
  const failing = resend(false);
  await runInactivity(db, makeEnv(), NOW, failing.fetcher);
  expect(db.rawAccount(account.id)?.resetNoticePendingUnixMs).toBe(resetAt);

  const { sent, fetcher } = resend();
  await runInactivity(db, makeEnv(), NOW, fetcher);
  await runInactivity(db, makeEnv(), NOW + DAY, fetcher);
  expect(sent).toHaveLength(1);
  expect(sent[0]).toMatchObject({
    from: "Whimbrel <whimbrel@futuremeanswered.com>",
    to: ["a@example.com"],
    reply_to: "hello@whimbrelapp.com",
    subject: "Your Whimbrel sync was reset",
  });
  expect(sent[0].text).toContain("October 28, 2026 at 14:05 UTC");
  expect(sent[0].text).toContain("If this wasn't you, reply to this email.");
  expect(db.rawAccount(account.id)?.resetNoticePendingUnixMs ?? null).toBeNull();
});

// ---- fix round 1 ----
async function warnedAt(db: FakeSqlExecutor) {
  const made = await idle(db, 181);
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(db.rawAccount(made.account.id)?.warnedAtUnixMs).toBe(NOW);
  return made;
}

it("a contact after the cron reads the account cancels deletion", async () => {
  const db = new FakeSqlExecutor();
  const { account, device } = await warnedAt(db);
  // The device reconnects between the cron's read and its claim.
  db.after("SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms FROM account WHERE warned_at", () =>
    accounts.touchLastSeen(db, account.id, device.id, NOW + 15 * DAY),
  );
  expect((await runInactivity(db, makeEnv(), NOW + 15 * DAY, resend().fetcher)).deleted).toBe(0);
  expect(del).not.toHaveBeenCalled();
  expect(db.rawAccount(account.id)?.deletingSinceUnixMs ?? null).toBeNull();
});

it("a failed deletion is finished by the next run", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await warnedAt(db);
  del.mockRejectedValueOnce(new Error("turso down"));
  expect((await runInactivity(db, makeEnv(), NOW + 15 * DAY, resend().fetcher)).deleted).toBe(0);
  expect(db.rawAccount(account.id)?.deletingSinceUnixMs).toBe(NOW + 15 * DAY);
  // A claim younger than 10 minutes is left to the deletion that holds it.
  await runInactivity(db, makeEnv(), NOW + 15 * DAY + 5 * 60 * 1000, resend().fetcher);
  expect(db.rawAccount(account.id)).toBeDefined();
  // A finished stale claim counts as a deletion.
  expect((await runInactivity(db, makeEnv(), NOW + 16 * DAY, resend().fetcher)).deleted).toBe(1);
  expect(db.rawAccount(account.id)).toBeUndefined();
  expect(del).toHaveBeenLastCalledWith(expect.anything(), "db-sub-a");
});

it("reset recovery leaves a device re-claimed since it was listed", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await idle(db, 1);
  const me = await accounts.enrollDevice(db, account.id, "me", "pending", NOW - DAY, "mac");
  await accounts.setReplacesDevice(db, me.id, mac.id);
  await accounts.claimSwapForReset(db, me.id, NOW - 30 * 60 * 1000);
  db.after("SELECT id FROM device WHERE status = 'resetting'", async () => {
    await accounts.unclaimSwapForReset(db, me.id);
    await accounts.claimSwapForReset(db, me.id, NOW);
  });
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(db.rawDevice(me.id)).toMatchObject({ status: "resetting", resetting_since_unix_ms: NOW });
});

it("a retired database that becomes current mid-run is not deleted", async () => {
  const db = new FakeSqlExecutor();
  const { account } = await idle(db, 1);
  await accounts.retireDatabase(db, account.id, "old-31", NOW - 31 * DAY);
  db.after("SELECT name FROM retired_database", async () => {
    await accounts.setAccountDatabase(db, account.id, "old-31", "u", NOW);
  });
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(del).not.toHaveBeenCalled();
  expect(db.rawRetired()).toHaveLength(1);
});

it("an expired swap that a new swap points at mid-run is kept", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await idle(db, 1);
  const old = await accounts.enrollDevice(db, account.id, "old", "pending", NOW - 8 * DAY, "mac");
  await accounts.setReplacesDevice(db, old.id, mac.id);
  db.after("SELECT id, status, replaces_device_id", async () => {
    const fresh = await accounts.enrollDevice(db, account.id, "fresh", "pending", NOW, "mac");
    await accounts.setReplacesDevice(db, fresh.id, old.id);
  });
  await runInactivity(db, makeEnv(), NOW, resend().fetcher);
  expect(db.rawDevice(old.id)).toBeDefined();
});

it("a Resend failure logs only the status", async () => {
  const db = new FakeSqlExecutor();
  await idle(db, 200);
  const log = vi.spyOn(console, "error").mockImplementation(() => {});
  const fetcher = (async () => new Response("secret-body a@example.com", { status: 422 })) as unknown as typeof fetch;
  await runInactivity(db, makeEnv(), NOW, fetcher);
  const logged = log.mock.calls.flat().map(String).join(" ");
  expect(logged).toContain("422");
  expect(logged).not.toContain("secret-body");
});

// ---- fix round 2 ----
it("cron skips an account with a reset in progress", async () => {
  const db = new FakeSqlExecutor();
  const { account, device: mac } = await warnedAt(db);
  const me = await accounts.enrollDevice(db, account.id, "me", "pending", NOW, "mac");
  await accounts.setReplacesDevice(db, me.id, mac.id);
  // Claimed just before the cron runs, so reset recovery leaves it alone.
  await accounts.claimSwapForReset(db, me.id, NOW + 15 * DAY - 60_000);
  expect((await runInactivity(db, makeEnv(), NOW + 15 * DAY, resend().fetcher)).deleted).toBe(0);
  expect(db.rawAccount(account.id)?.deletingSinceUnixMs ?? null).toBeNull();
  expect(del).not.toHaveBeenCalled();
});
