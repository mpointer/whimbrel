import { it, expect, vi, beforeEach } from "vitest";
import { env, createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import * as googleAuth from "../src/googleAuth";
import * as accounts from "../src/accounts";
import { AI_PRICES, costMicros, handleAiMessages } from "../src/aiProxy";
import { aiBalance, creditPurchase, placeHold } from "../src/aiCredit";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";

const NOW = 1_800_000_000_000;
const MODEL = "claude-haiku-4-5";
const START = 5_000_000;

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    GOOGLE_CLIENT_ID: "test-client-id",
    TURSO_DATABASE_URL: "https://test-control-plane.example.turso.io",
    TURSO_DATABASE_AUTH_TOKEN: "test-database-token",
    TURSO_PLATFORM_API_TOKEN: "test-platform-token",
    TURSO_ORG_SLUG: "test-org",
    TURSO_DB_GROUP: "test-group",
    ALLOWED_GOOGLE_EMAILS: "owner@example.com",
    OPEN_SIGN_IN: "true",
    ANTHROPIC_API_KEY: "sk-ant-whimbrel",
    ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF,
    ...overrides,
  };
}

let sub = "";
beforeEach(() => {
  vi.restoreAllMocks();
  sub = `ai-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
});

async function seeded(status: "approved" | "pending" = "approved", credit = START) {
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", sub, 1, "db", "https://db");
  await accounts.enrollDevice(db, account.id, "mac-key", status, 2, "mac");
  if (credit > 0) await creditPurchase(db, account.id, credit, `pi_${crypto.randomUUID()}`, NOW);
  return { db, id: account.id };
}

const messages = (body: object = {}, headers: Record<string, string> = {}) =>
  new Request("https://example.com/ai/v1/messages", {
    method: "POST",
    headers: { authorization: "Bearer t", "x-whimbrel-device": "mac-key", "content-type": "application/json", ...headers },
    body: JSON.stringify({ model: MODEL, max_tokens: 1024, stream: true, messages: [{ role: "user", content: "hi" }], ...body }),
  });

const SSE = [
  `event: message_start\ndata: {"type":"message_start","message":{"id":"msg_1","model":"${MODEL}","usage":{"input_tokens":1000,"output_tokens":1}}}\n\n`,
  `event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}\n\n`,
  `event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}\n\n`,
  `event: content_block_stop\ndata: {"type":"content_block_stop","index":0}\n\n`,
  `event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":500}}\n\n`,
  `event: message_stop\ndata: {"type":"message_stop"}\n\n`,
];
const USAGE = { input_tokens: 1000, output_tokens: 500 };

function sseFetcher(chunks: string[] = SSE) {
  const calls: { url: string; init: RequestInit }[] = [];
  const fetcher = (async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    const enc = new TextEncoder();
    let i = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(c) {
        if (i < chunks.length) c.enqueue(enc.encode(chunks[i++]));
        else c.close();
      },
    });
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

function jsonFetcher(status: number, body: unknown) {
  const calls: { url: string; init: RequestInit }[] = [];
  const fetcher = (async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

it("costMicros_is_list_price_times_1_3_rounded_up_to_100", () => {
  // haiku-4-5: $1 in, $5 out per MTok. 1000 in + 500 out = $0.0035, x1.3 = 4550 micros, up to 4600.
  expect(costMicros(MODEL, USAGE)).toBe(4600);
  const p = AI_PRICES["claude-opus-5"];
  const raw = (2000 * p.input + 10 * p.output + 300 * p.cacheWrite + 4000 * p.cacheRead) / 1e6;
  expect(costMicros("claude-opus-5", { input_tokens: 2000, output_tokens: 10, cache_creation_input_tokens: 300, cache_read_input_tokens: 4000 })).toBe(
    Math.ceil((raw * 1.3) / 100) * 100,
  );
});

it("streams_through_and_settles_real_usage", async () => {
  const { db, id } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const ctx = createExecutionContext();
  const res = await handleAiMessages(
    messages({ max_tokens: 100_000 }, { "anthropic-version": "2099-01-01", "anthropic-beta": "fast-mode-2026-02-01" }),
    makeEnv(),
    ctx,
    db,
    fetcher,
    NOW,
  );
  expect(res.status).toBe(200);
  expect(res.headers.get("content-type")).toBe("text/event-stream");
  const held = Number(res.headers.get("x-whimbrel-credit"));
  expect(held).toBeLessThan(START);
  expect(held).toBe(await aiBalance(db, id));
  expect(await res.text()).toBe(SSE.join(""));
  await waitOnExecutionContext(ctx);
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, USAGE));
  expect(db.rawAiHolds()).toHaveLength(0);
  const usage = db.rawAiLedger().filter((r) => r.kind === "usage");
  expect(usage).toHaveLength(1);
  expect(usage[0]).toMatchObject({ model: MODEL, input_tokens: 1000, output_tokens: 500 });
  // Whimbrel's key goes upstream, the caller's token does not; max_tokens is capped.
  expect(calls).toHaveLength(1);
  expect(calls[0].url).toBe("https://api.anthropic.com/v1/messages");
  const sent = new Headers(calls[0].init.headers);
  expect(sent.get("x-api-key")).toBe("sk-ant-whimbrel");
  expect(sent.get("authorization")).toBeNull();
  expect(sent.get("anthropic-version")).toBe("2023-06-01");
  expect(sent.get("anthropic-beta")).toBeNull();
  expect([...sent.keys()].sort()).toEqual(["anthropic-version", "content-type", "x-api-key"]);
  expect(JSON.parse(String(calls[0].init.body))).toMatchObject({ model: MODEL, max_tokens: 8192, stream: true });
});

/** An upstream that sends `chunks`, then hangs until its request is aborted. */
function hangingFetcher(chunks: string[]) {
  const state: { signal?: AbortSignal; aborted: boolean } = { aborted: false };
  const fetcher = (async (_url: string, init: RequestInit) => {
    state.signal = init.signal ?? undefined;
    const enc = new TextEncoder();
    let i = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(c) {
        if (i < chunks.length) return c.enqueue(enc.encode(chunks[i++]));
        return new Promise<void>((resolve) =>
          state.signal?.addEventListener("abort", () => {
            state.aborted = true;
            resolve();
          }),
        );
      },
    });
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
  }) as unknown as typeof fetch;
  return { fetcher, state };
}

/** Starts a stream, reads every chunk `chunks` holds, then hangs up. */
async function hangUpAfter(chunks: string[]) {
  const seed = await seeded();
  const { fetcher, state } = hangingFetcher(chunks);
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages(), makeEnv(), ctx, seed.db, fetcher, NOW);
  const reader = res.body!.getReader();
  const dec = new TextDecoder();
  let got = "";
  while (got.length < chunks.join("").length) got += dec.decode((await reader.read()).value);
  expect(seed.db.rawAiHolds()).toHaveLength(1);
  await reader.cancel();
  return { ...seed, ctx, state };
}

it("disconnect_settles_at_once_and_frees_the_slot", async () => {
  // message_start, one text delta, then nothing until aborted.
  const { db, id, ctx, state } = await hangUpAfter([SSE[0], SSE[2]]);
  // Settled at once: no waiting on the context or the cron.
  expect(state.signal?.aborted).toBe(true);
  expect(state.aborted).toBe(true);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(0);
  // No message_delta yet: output is charged at max_tokens (1024).
  const usage = { input_tokens: 1000, output_tokens: 1024 };
  expect(db.rawAiLedger().find((r) => r.kind === "usage")).toMatchObject({ input_tokens: 1000, output_tokens: 1024 });
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, usage));
  await waitOnExecutionContext(ctx);
  expect(db.rawAiLedger().filter((r) => r.kind === "usage")).toHaveLength(1);
});

it("hang_up_before_any_delta_charges_max_tokens_output", async () => {
  const { db, id } = await hangUpAfter([SSE[0]]);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAiLedger().find((r) => r.kind === "usage")).toMatchObject({ input_tokens: 1000, output_tokens: 1024 });
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, { input_tokens: 1000, output_tokens: 1024 }));
});

it("hang_up_after_a_usage_delta_charges_that_output", async () => {
  const { db, id } = await hangUpAfter([SSE[0], SSE[2], SSE[4]]);
  expect(db.rawAiLedger().find((r) => r.kind === "usage")).toMatchObject({ input_tokens: 1000, output_tokens: 500 });
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, USAGE));
});

it("pending_device_is_refused", async () => {
  const { db, id } = await seeded("pending");
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages(), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(401);
  expect(calls).toHaveLength(0);
  expect(await aiBalance(db, id)).toBe(START);
});

it("bad_or_missing_credentials_are_401", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const ctx = createExecutionContext();
  const noAuth = new Request("https://example.com/ai/v1/messages", { method: "POST", headers: { "x-whimbrel-device": "mac-key" }, body: "{}" });
  expect((await handleAiMessages(noAuth, makeEnv(), ctx, db, fetcher, NOW)).status).toBe(401);
  expect((await handleAiMessages(messages({}, { "x-whimbrel-device": "other-key" }), makeEnv(), ctx, db, fetcher, NOW)).status).toBe(401);
  // Not on the allowlist while sign-in is closed.
  const closed = makeEnv({ OPEN_SIGN_IN: undefined, ALLOWED_GOOGLE_EMAILS: "someone@example.com" });
  expect((await handleAiMessages(messages(), closed, ctx, db, fetcher, NOW)).status).toBe(401);
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockRejectedValue(new Error("bad token"));
  expect((await handleAiMessages(messages(), makeEnv(), ctx, db, fetcher, NOW)).status).toBe(401);
  expect(calls).toHaveLength(0);
});

it("account_being_deleted_is_503", async () => {
  const { db, id } = await seeded();
  await accounts.claimForDeletion(db, id, NOW);
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages(), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(503);
  expect(calls).toHaveLength(0);
});

it("unknown_model_is_400", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages({ model: "gpt-4o" }), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(400);
  expect(calls).toHaveLength(0);
});

it("no_credit_returns_402_without_calling_upstream", async () => {
  const { db, id } = await seeded("approved", 0);
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages(), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(402);
  expect(await res.json()).toEqual({ error: "credit_exhausted" });
  expect(calls).toHaveLength(0);
  expect(await aiBalance(db, id)).toBe(0);
});

/** The proxy's hold for a body of `bytes` bytes and `maxTokens` output, rounded up to 100 micros as placeHold does. */
function heldFor(model: string, bytes: number, maxTokens: number): number {
  const p = AI_PRICES[model];
  const raw = Math.ceil(((bytes * Math.max(p.input, p.cacheWrite) + maxTokens * p.output) * 13) / (10 * 1_000_000));
  return Math.ceil(raw / 100) * 100;
}

const bodyBytes = (max_tokens: number) =>
  new TextEncoder().encode(JSON.stringify({ model: MODEL, max_tokens, stream: true, messages: [{ role: "user", content: "hi" }] })).byteLength;

it("short_credit_shrinks_max_tokens_to_what_the_balance_holds", async () => {
  // Ruling R16: enough for 2000 output tokens but not 8192, so the request goes ahead at about 2000.
  const balance = heldFor(MODEL, bodyBytes(8192), 2000);
  expect(balance).toBeLessThan(heldFor(MODEL, bodyBytes(8192), 8192));
  const { db, id } = await seeded("approved", balance);
  const { fetcher, calls } = sseFetcher();
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages({ max_tokens: 8192 }), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  expect(calls).toHaveLength(1);
  const sentBody = String(calls[0].init.body);
  const forwarded = JSON.parse(sentBody).max_tokens;
  expect(forwarded).toBeGreaterThanOrEqual(2000);
  expect(forwarded).toBeLessThan(2100);
  // The hold is sized on the very body that was forwarded, and fits the balance.
  const holds = db.rawAiHolds();
  expect(holds).toHaveLength(1);
  expect(holds[0].micros).toBe(heldFor(MODEL, new TextEncoder().encode(sentBody).byteLength, forwarded));
  expect(holds[0].micros).toBeLessThanOrEqual(balance);
  expect(heldFor(MODEL, bodyBytes(forwarded + 1), forwarded + 1)).toBeGreaterThan(balance);
  expect(await res.text()).toBe(SSE.join(""));
  await waitOnExecutionContext(ctx);
  expect(await aiBalance(db, id)).toBe(balance - costMicros(MODEL, USAGE));
  expect(db.rawAiHolds()).toHaveLength(0);
});

it("credit_below_the_512_token_floor_is_402", async () => {
  const balance = heldFor(MODEL, bodyBytes(8192), 512) - 100;
  const { db, id } = await seeded("approved", balance);
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages({ max_tokens: 8192 }), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(402);
  expect(await res.json()).toEqual({ error: "credit_exhausted" });
  expect(calls).toHaveLength(0);
  expect(await aiBalance(db, id)).toBe(balance);
  expect(db.rawAiHolds()).toHaveLength(0);
});

it("a_request_already_at_or_below_the_floor_is_402_when_short", async () => {
  const balance = heldFor(MODEL, bodyBytes(300), 300) - 100;
  const { db } = await seeded("approved", balance);
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages({ max_tokens: 300 }), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(402);
  expect(calls).toHaveLength(0);
});

it("unsupported_requests_are_400_without_a_hold_or_upstream_call", async () => {
  const { db, id } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const ctx = createExecutionContext();
  const user = (...content: object[]) => ({ messages: [{ role: "user", content }] });
  const cases: Record<string, object> = {
    base64Pdf: user({ type: "document", source: { type: "base64", media_type: "application/pdf", data: "JVBERi0x" } }, { type: "text", text: "hi" }),
    base64Image: user({ type: "image", source: { type: "base64", media_type: "image/png", data: "iVBORw0K" } }, { type: "text", text: "hi" }),
    urlImage: user({ type: "image", source: { type: "url", url: "https://example.com/big.png" } }),
    textDocument: user({ type: "document", source: { type: "text", media_type: "text/plain", data: "doc" } }),
    tools: { tools: [{ name: "lookup", input_schema: { type: "object" } }] },
    webSearch: { tools: [{ type: "web_search_20260209", name: "web_search", max_uses: 50 }] },
    toolChoice: { tool_choice: { type: "auto" } },
    thinking: { thinking: { type: "adaptive" } },
    thinkingBlock: { messages: [{ role: "user", content: "q" }, { role: "assistant", content: [{ type: "thinking", thinking: "", signature: "s" }] }] },
    toolUse: { messages: [{ role: "assistant", content: [{ type: "tool_use", id: "t1", name: "x", input: {} }] }] },
    toolResult: user({ type: "tool_result", tool_use_id: "t1", content: "ok" }),
    topP: { top_p: 0.9 },
    topK: { top_k: 5 },
    geo: { inference_geo: "us" },
    speed: { speed: "fast" },
    oneHour: { system: [{ type: "text", text: "sys", cache_control: { type: "ephemeral", ttl: "1h" } }] },
    citations: user({ type: "text", text: "hi", citations: [] }),
    systemRole: { messages: [{ role: "system", content: "x" }] },
  };
  for (const [name, body] of Object.entries(cases)) {
    const res = await handleAiMessages(messages(body), makeEnv(), ctx, db, fetcher, NOW);
    expect(res.status, name).toBe(400);
    expect(await res.json(), name).toEqual({ error: "unsupported_request" });
    expect(calls, name).toHaveLength(0);
    expect(db.rawAiHolds(), name).toHaveLength(0);
  }
  expect(await aiBalance(db, id)).toBe(START);
});

it("text_shapes_pass_the_allowlist", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const ctx = createExecutionContext();
  const body = {
    system: [{ type: "text", text: "sys", cache_control: { type: "ephemeral", ttl: "5m" } }],
    temperature: 0.2,
    stop_sequences: ["END"],
    metadata: { user_id: "u" },
    messages: [
      { role: "user", content: [{ type: "text", text: "hi", cache_control: { type: "ephemeral" } }] },
      { role: "assistant", content: "hello" },
      { role: "user", content: "more" },
    ],
  };
  const res = await handleAiMessages(messages(body), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  await res.text();
  await waitOnExecutionContext(ctx);
  expect(calls).toHaveLength(1);
  expect(JSON.parse(String(calls[0].init.body))).toEqual({ model: MODEL, max_tokens: 1024, stream: true, ...body });
});

it("non_ascii_hold_covers_the_cost_at_one_token_per_character", async () => {
  const { db, id } = await seeded("approved", 100_000_000);
  const { fetcher } = sseFetcher();
  for (const model of Object.keys(AI_PRICES)) {
    const text = "\u6f22\u5b57\u304b\u306a\u4ea4\u3058\u308a\u6587".repeat(400);
    const req = messages({ model, max_tokens: 2000, messages: [{ role: "user", content: text }] });
    const chars = [...(await req.clone().text())].length;
    const before = await aiBalance(db, id);
    const ctx = createExecutionContext();
    const res = await handleAiMessages(req, makeEnv(), ctx, db, fetcher, NOW);
    const hold = before - Number(res.headers.get("x-whimbrel-credit"));
    for (const usage of [
      { input_tokens: chars, output_tokens: 2000 },
      { cache_creation_input_tokens: chars, output_tokens: 2000 },
    ]) {
      expect(hold).toBeGreaterThanOrEqual(costMicros(model, usage));
    }
    await res.text();
    await waitOnExecutionContext(ctx);
  }
});

it("one_hour_cache_writes_bill_at_twice_input", () => {
  const p = AI_PRICES[MODEL];
  const usage = { cache_creation_input_tokens: 3000, cache_creation: { ephemeral_5m_input_tokens: 1000, ephemeral_1h_input_tokens: 2000 } };
  const raw = (1000 * p.cacheWrite + 2000 * 2 * p.input) / 1e6;
  expect(costMicros(MODEL, usage)).toBe(Math.ceil((raw * 1.3) / 100) * 100);
  expect(costMicros(MODEL, usage)).toBeGreaterThan(costMicros(MODEL, { cache_creation_input_tokens: 3000 }));
});

it("four_open_holds_is_429_not_402", async () => {
  const { db, id } = await seeded();
  for (let i = 0; i < 4; i++) expect(await placeHold(db, id, 100, NOW)).not.toBeNull();
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages(), makeEnv(), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(429);
  expect(await res.json()).toEqual({ error: "too_many_in_flight" });
  expect(calls).toHaveLength(0);
});

it("a_failed_balance_read_still_settles_from_usage", async () => {
  const { db, id } = await seeded();
  const real = db.execute.bind(db);
  let failBalance = true;
  db.execute = (async (sql: string, params: unknown[]) => {
    if (failBalance && sql.startsWith("SELECT ai_credit_micros")) throw new Error("db blip");
    return real(sql, params);
  }) as typeof db.execute;
  const { fetcher } = sseFetcher();
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages(), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  expect(res.headers.get("x-whimbrel-credit")).toBeNull();
  expect(await res.text()).toBe(SSE.join(""));
  await waitOnExecutionContext(ctx);
  failBalance = false;
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, USAGE));
  expect(db.rawAiHolds()).toHaveLength(0);
});

it("a_failed_json_read_charges_the_whole_hold", async () => {
  const { db, id } = await seeded();
  const fetcher = (async () =>
    new Response(
      new ReadableStream<Uint8Array>({
        start(c) {
          c.error(new Error("connection reset"));
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    )) as unknown as typeof fetch;
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages({ stream: false }), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(503);
  await waitOnExecutionContext(ctx);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(0);
  const charged = START - (await aiBalance(db, id));
  expect(charged).toBeGreaterThan(0);
  const usage = db.rawAiLedger().filter((r) => r.kind === "usage");
  expect(usage).toHaveLength(1);
  // settleHold(null): the hold itself is the charge, with no usage recorded.
  expect(usage[0]).toMatchObject({ micros: -charged, model: null });
});

it("duplicate_keys_are_never_forwarded_raw", async () => {
  const { db, id } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const raw =
    '{"model":"claude-fable-5-1","model":"claude-haiku-4-5","max_tokens":1024,"stream":true,' +
    '"messages":[{"role":"user","content":"hi"}],"messages":[{"role":"user","content":"hi"}]}';
  const req = new Request("https://example.com/ai/v1/messages", {
    method: "POST",
    headers: { authorization: "Bearer t", "x-whimbrel-device": "mac-key" },
    body: raw,
  });
  const ctx = createExecutionContext();
  const res = await handleAiMessages(req, makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  const sent = String(calls[0].init.body);
  expect(sent).not.toBe(raw);
  expect(sent.match(/"model"/g)).toHaveLength(1);
  expect(sent.match(/"messages"/g)).toHaveLength(1);
  expect(JSON.parse(sent).model).toBe("claude-haiku-4-5");
  // The hold was priced on that model and on the bytes actually sent.
  const p = AI_PRICES["claude-haiku-4-5"];
  const bytes = new TextEncoder().encode(sent).byteLength;
  const hold = Math.ceil(Math.ceil(((bytes * Math.max(p.input, p.cacheWrite) + 1024 * p.output) * 13) / 1e7) / 100) * 100;
  expect(START - Number(res.headers.get("x-whimbrel-credit"))).toBe(hold);
  await res.text();
  await waitOnExecutionContext(ctx);
  expect(db.rawAiLedger().find((r) => r.kind === "usage")).toMatchObject({ model: "claude-haiku-4-5" });
  expect(await aiBalance(db, id)).toBe(START - costMicros("claude-haiku-4-5", USAGE));
});

it("upstream_500_costs_nothing", async () => {
  const { db, id } = await seeded();
  const err = { type: "error", error: { type: "api_error", message: "boom" } };
  const { fetcher } = jsonFetcher(500, err);
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages(), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(500);
  expect(await res.json()).toEqual(err);
  await waitOnExecutionContext(ctx);
  expect(await aiBalance(db, id)).toBe(START);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(0);
});

it("non_streaming_json_response_settles_from_usage", async () => {
  const { db, id } = await seeded();
  const message = { id: "msg_1", type: "message", model: MODEL, content: [{ type: "text", text: "Hello" }], usage: USAGE };
  const { fetcher } = jsonFetcher(200, message);
  const ctx = createExecutionContext();
  const res = await handleAiMessages(messages({ stream: false }), makeEnv(), ctx, db, fetcher, NOW);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual(message);
  await waitOnExecutionContext(ctx);
  expect(await aiBalance(db, id)).toBe(START - costMicros(MODEL, USAGE));
});

it("missing_key_returns_503", async () => {
  const { db, id } = await seeded();
  const { fetcher, calls } = sseFetcher();
  const res = await handleAiMessages(messages(), makeEnv({ ANTHROPIC_API_KEY: undefined }), createExecutionContext(), db, fetcher, NOW);
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "ai_unavailable" });
  expect(calls).toHaveLength(0);
  expect(await aiBalance(db, id)).toBe(START);
});

it("sixty_one_requests_in_a_minute_is_429", async () => {
  // One fixed minute, so the window cannot roll over mid-test.
  vi.spyOn(Date, "now").mockReturnValue(NOW);
  const { db } = await seeded("approved", 100_000_000);
  const { fetcher } = jsonFetcher(400, { type: "error", error: { type: "invalid_request_error", message: "x" } });
  const ctx = createExecutionContext();
  const statuses: number[] = [];
  for (let i = 0; i < 61; i++) statuses.push((await handleAiMessages(messages(), makeEnv(), ctx, db, fetcher, NOW)).status);
  await waitOnExecutionContext(ctx);
  expect(statuses.slice(0, 60).every((s) => s === 400)).toBe(true);
  expect(statuses[60]).toBe(429);
});

it("the_worker_routes_ai_messages_with_its_context", async () => {
  const worker = (await import("../src/index")).default;
  const res = await worker.fetch(messages(), makeEnv({ ANTHROPIC_API_KEY: undefined }), createExecutionContext());
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "ai_unavailable" });
});
