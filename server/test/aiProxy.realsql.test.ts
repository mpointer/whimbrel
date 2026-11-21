import { afterEach, beforeEach, expect, it, vi } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as accounts from "../src/accounts";
import { AI_PRICES, handleAiMessages } from "../src/aiProxy";
import { creditPurchase, grantAllowance, placeHold } from "../src/aiCredit";
import type { SqlExecutor } from "../src/accounts";
import type { Env } from "../src/index";
import { realSqlExecutor } from "./testHelpers/realSqlExecutor";

// The proxy's money decisions against REAL SQLite: the spendable read, the
// 429-versus-402 choice, the shrink and the race re-check run the SQL that
// ships, so changing a decision read to prepaid-only fails these.

const NOW = Date.UTC(2026, 10, 19, 9, 0, 0);
const END = NOW + 30 * 24 * 60 * 60 * 1000;
const MODEL = "claude-haiku-4-5";

let open: (SqlExecutor & { close(): void })[] = [];
beforeEach(() => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: "g-1", audience: "test-client-id" });
});
afterEach(() => {
  for (const db of open) db.close();
  open = [];
  vi.restoreAllMocks();
});

const kv = { get: async () => null, put: async () => undefined };
const env = {
  GOOGLE_CLIENT_ID: "test-client-id",
  ALLOWED_GOOGLE_EMAILS: "owner@example.com",
  OPEN_SIGN_IN: "true",
  ANTHROPIC_API_KEY: "sk-ant-whimbrel",
  ENROLLMENT_HANDOFF: kv,
} as unknown as Env;

/** An execution context that remembers what the handler hands to waitUntil. */
function context() {
  const pending: Promise<unknown>[] = [];
  const ctx = { waitUntil: (p: Promise<unknown>) => void pending.push(p), passThroughOnException() {} };
  return { ctx: ctx as never, settled: () => Promise.all(pending) };
}

async function account(prepaid: number, allowance: number) {
  const db = realSqlExecutor();
  open.push(db);
  const a = await accounts.getOrCreateAccount(db, "owner@example.com", "g-1", 1, "db", "https://db.test");
  await accounts.enrollDevice(db, a.id, "mac-key", "approved", 2, "mac");
  if (prepaid > 0) await creditPurchase(db, a.id, prepaid, "pi_seed", NOW);
  if (allowance > 0) expect(await grantAllowance(db, { accountId: a.id, grantKey: "in_seed", micros: allowance, periodEndUnixMs: END }, NOW)).toBe(true);
  return { db, id: a.id };
}

const request = (max_tokens = 1024) =>
  new Request("https://example.com/ai/v1/messages", {
    method: "POST",
    headers: { authorization: "Bearer t", "x-whimbrel-device": "mac-key", "content-type": "application/json" },
    body: JSON.stringify({ model: MODEL, max_tokens, stream: true, messages: [{ role: "user", content: "hi" }] }),
  });

const SSE =
  `event: message_start\ndata: {"type":"message_start","message":{"id":"m","model":"${MODEL}","usage":{"input_tokens":1000,"output_tokens":1}}}\n\n` +
  `event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":500}}\n\n` +
  `event: message_stop\ndata: {"type":"message_stop"}\n\n`;

function upstream() {
  const calls: string[] = [];
  const fetcher = (async (_url: string, init: RequestInit) => {
    calls.push(String(init.body));
    return new Response(SSE, { status: 200, headers: { "content-type": "text/event-stream" } });
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

const bodyBytes = (max_tokens: number) =>
  new TextEncoder().encode(JSON.stringify({ model: MODEL, max_tokens, stream: true, messages: [{ role: "user", content: "hi" }] })).byteLength;
function heldFor(bytes: number, maxTokens: number): number {
  const p = AI_PRICES[MODEL];
  const raw = Math.ceil(((bytes * Math.max(p.input, p.cacheWrite) + maxTokens * p.output) * 13) / (10 * 1_000_000));
  return Math.ceil(raw / 100) * 100;
}
const holds = async (db: SqlExecutor) => db.execute("SELECT micros, from_allowance_micros AS fromAllowance FROM ai_hold", []);

it("a_plus_user_with_zero_prepaid_and_allowance_left_is_served", async () => {
  const { db, id } = await account(0, 6_000_000);
  const { fetcher, calls } = upstream();
  const { ctx, settled } = context();
  const res = await handleAiMessages(request(), env, ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  expect(calls).toHaveLength(1);
  await res.text();
  await settled();
  const [row] = await db.execute("SELECT ai_credit_micros AS prepaid, ai_allowance_micros AS allowance, ai_open_holds AS n FROM account WHERE id = ?", [id]);
  expect(row.prepaid).toBe(0);
  expect(row.allowance).toBe(6_000_000 - 4_600);
  expect(row.n).toBe(0);
});

it("at_the_four_hold_cap_a_plus_user_gets_429_not_402", async () => {
  const { db, id } = await account(0, 6_000_000);
  for (let i = 0; i < 4; i++) expect(await placeHold(db, id, 100, NOW)).not.toBeNull();
  const realBatch = db.batch.bind(db);
  let batches = 0;
  db.batch = ((stmts: Parameters<typeof db.batch>[0]) => (batches++, realBatch(stmts))) as typeof db.batch;
  const { fetcher, calls } = upstream();
  const res = await handleAiMessages(request(), env, context().ctx, db, fetcher, NOW);
  expect(res.status).toBe(429);
  expect(await res.json()).toEqual({ error: "too_many_in_flight" });
  expect(calls).toHaveLength(0);
  // Refused at once: one hold attempt, no shrink and second attempt.
  expect(batches).toBe(1);
});

it("allowance_and_prepaid_both_zero_answers_402_credit_exhausted", async () => {
  // One account with nothing, one whose allowance has ended by the request time.
  for (const [a, at] of [[await account(0, 0), NOW], [await account(0, 6_000_000), END + 1_000]] as const) {
    const { fetcher, calls } = upstream();
    const res = await handleAiMessages(request(), env, context().ctx, a.db, fetcher, at);
    expect(res.status).toBe(402);
    expect(await res.json()).toEqual({ error: "credit_exhausted" });
    expect(calls).toHaveLength(0);
    expect(await holds(a.db)).toHaveLength(0);
  }
});

it("the_shrink_uses_allowance_plus_prepaid", async () => {
  // Enough in total for about 2000 output tokens; the prepaid part alone is below the 512-token floor.
  const total = heldFor(bodyBytes(8192), 2000);
  const prepaid = 1_000;
  expect(prepaid).toBeLessThan(heldFor(bodyBytes(512), 512));
  const { db } = await account(prepaid, total - prepaid);
  const { fetcher, calls } = upstream();
  const { ctx, settled } = context();
  const res = await handleAiMessages(request(8192), env, ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  const forwarded = JSON.parse(calls[0]).max_tokens;
  expect(forwarded).toBeGreaterThanOrEqual(2000);
  expect(forwarded).toBeLessThan(2100);
  const [hold] = await holds(db);
  expect(hold.micros).toBe(heldFor(new TextEncoder().encode(calls[0]).byteLength, forwarded));
  expect(hold.micros).toBeLessThanOrEqual(total);
  expect(hold.fromAllowance).toBe(total - prepaid);
  await res.text();
  await settled();
});

it("the_credit_header_stays_prepaid_only", async () => {
  const { db } = await account(250_000, 6_000_000);
  const { fetcher } = upstream();
  const { ctx, settled } = context();
  const res = await handleAiMessages(request(), env, ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  // The hold came out of the allowance, so prepaid is untouched and the header is exactly it.
  expect(res.headers.get("x-whimbrel-credit")).toBe("250000");
  await res.text();
  await settled();
});

it("a_race_that_takes_the_last_slot_after_the_shrink_is_429_for_an_allowance_only_user", async () => {
  const total = heldFor(bodyBytes(8192), 2000);
  const { db, id } = await account(0, total);
  for (let i = 0; i < 3; i++) expect(await placeHold(db, id, 100, NOW)).not.toBeNull();
  // Right after the proxy's spendable read, another request takes the 4th slot (and some credit arrives,
  // so the shrunk hold still fits and only the cap is in the way).
  const real = db.execute.bind(db);
  let raced = false;
  db.execute = (async (sql: string, params: unknown[]) => {
    const rows = await real(sql, params);
    if (!raced && sql.includes("AS spendable")) {
      raced = true;
      expect(await placeHold(db, id, 100, NOW)).not.toBeNull();
      await creditPurchase(db, id, 5_000, "pi_race", NOW);
    }
    return rows;
  }) as typeof db.execute;
  const { fetcher, calls } = upstream();
  const res = await handleAiMessages(request(8192), env, context().ctx, db, fetcher, NOW);
  expect(raced).toBe(true);
  expect(res.status).toBe(429);
  expect(await res.json()).toEqual({ error: "too_many_in_flight" });
  expect(calls).toHaveLength(0);
});

it("a_race_that_drains_the_money_after_the_shrink_is_402_credit_exhausted", async () => {
  const total = heldFor(bodyBytes(8192), 2000);
  const { db, id } = await account(0, total);
  // Right after the proxy's spendable read, a competing request holds everything that was left.
  const real = db.execute.bind(db);
  let raced = false;
  db.execute = (async (sql: string, params: unknown[]) => {
    const rows = await real(sql, params);
    if (!raced && sql.includes("AS spendable")) {
      raced = true;
      expect(await placeHold(db, id, total, NOW)).not.toBeNull();
    }
    return rows;
  }) as typeof db.execute;
  const { fetcher, calls } = upstream();
  const res = await handleAiMessages(request(8192), env, context().ctx, db, fetcher, NOW);
  expect(raced).toBe(true);
  expect(res.status).toBe(402);
  expect(await res.json()).toEqual({ error: "credit_exhausted" });
  expect(calls).toHaveLength(0);
});
