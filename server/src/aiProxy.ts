import type { ExecutionContext } from "@cloudflare/workers-types";
import { findAccountByGoogleSub, findDeviceByPublicKey, type SqlExecutor } from "./accounts";
import { aiBalance, placeHold, settleHold, type AiUsage } from "./aiCredit";
import { isSignInAllowed } from "./allowlist";
import { allowedAudiences, googleJwks, verifyGoogleIdToken } from "./googleAuth";
import { rateLimit, rateLimited } from "./rateLimit";
import type { Env } from "./index";

/** Anthropic list prices in micro-dollars per million tokens ($1/MTok =
 * 1_000_000), for every Anthropic model id the app offers (the ids in
 * core/llm/src/lib.rs `pricing_for` and core/api/src/routing.rs).
 *
 * Source: the claude-api skill's "Current Models" table and
 * shared/models.md (skill cache dated 2026-10-06), read on 2026-11-10.
 * Ruling R1: these prices change, so re-read them when Anthropic changes
 * them. Cache writes are the 5-minute TTL rate, 1.25x input
 * (shared/prompt-caching.md). Cache reads are 0.1x input, except where the
 * skill names a model's own rate (Fable 5.1: $0.25/MTok). */
export const AI_PRICES: Record<string, { input: number; output: number; cacheWrite: number; cacheRead: number }> = {
  "claude-haiku-4-5": { input: 1_000_000, output: 5_000_000, cacheWrite: 1_250_000, cacheRead: 100_000 },
  "claude-haiku-4-5-20251001": { input: 1_000_000, output: 5_000_000, cacheWrite: 1_250_000, cacheRead: 100_000 },
  "claude-sonnet-4-6": { input: 3_000_000, output: 15_000_000, cacheWrite: 3_750_000, cacheRead: 300_000 },
  "claude-sonnet-5": { input: 2_000_000, output: 10_000_000, cacheWrite: 2_500_000, cacheRead: 200_000 },
  "claude-opus-4-6": { input: 5_000_000, output: 25_000_000, cacheWrite: 6_250_000, cacheRead: 500_000 },
  "claude-opus-4-7": { input: 5_000_000, output: 25_000_000, cacheWrite: 6_250_000, cacheRead: 500_000 },
  "claude-opus-4-8": { input: 5_000_000, output: 25_000_000, cacheWrite: 6_250_000, cacheRead: 500_000 },
  "claude-opus-5": { input: 5_000_000, output: 25_000_000, cacheWrite: 6_250_000, cacheRead: 500_000 },
  "claude-fable-5": { input: 10_000_000, output: 50_000_000, cacheWrite: 12_500_000, cacheRead: 1_000_000 },
  "claude-fable-5-1": { input: 10_000_000, output: 50_000_000, cacheWrite: 12_500_000, cacheRead: 250_000 },
};

/** The `usage` object of an Anthropic Messages response. */
export interface AnthropicUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
  /** Per-TTL split of `cache_creation_input_tokens`, when Anthropic sends it. */
  cache_creation?: { ephemeral_5m_input_tokens?: number; ephemeral_1h_input_tokens?: number };
}

const ANTHROPIC_URL = "https://api.anthropic.com/v1/messages";
/** Always this version, whatever the client sends: the usage shape that
 * `costMicros` reads depends on it. */
const ANTHROPIC_VERSION = "2023-06-01";
const MAX_TOKENS_CAP = 8192;
/** Ruling R16: when the balance can't hold the requested max_tokens, the
 * proxy forwards the most it can hold, down to this floor, before it
 * answers 402. */
const MIN_SHRUNK_MAX_TOKENS = 512;

const tokens = (n: unknown): number => (typeof n === "number" && Number.isSafeInteger(n) && n > 0 ? n : 0);

/** What a request costs the user: list price x 1.3, rounded up to 100
 * micros. Integer math up to the one division, so there is no float drift.
 * Cache writes are 1.25x input (the `cacheWrite` price), except 1-hour
 * writes, which are 2x input. The proxy rejects a 1-hour TTL, so those
 * should never appear; they are priced in case they do. */
export function costMicros(model: string, usage: AnthropicUsage): number {
  const p = AI_PRICES[model];
  if (!p) throw new RangeError(`no price for model ${model}`);
  const written = tokens(usage.cache_creation_input_tokens);
  const oneHour = tokens(usage.cache_creation?.ephemeral_1h_input_tokens);
  const fiveMinute = Math.max(written - oneHour, tokens(usage.cache_creation?.ephemeral_5m_input_tokens));
  const sum =
    tokens(usage.input_tokens) * p.input +
    tokens(usage.output_tokens) * p.output +
    fiveMinute * p.cacheWrite +
    oneHour * 2 * p.input +
    tokens(usage.cache_read_input_tokens) * p.cacheRead;
  // sum is micros x 1e6. x13/10 is the markup; /100 then x100 rounds up.
  return Math.ceil((sum * 13) / (10 * 1_000_000 * 100)) * 100;
}

/** The hold placed before forwarding, an upper bound on the cost: every
 * UTF-8 byte of the body as one input token at the dearer of the input and
 * cache-write prices, plus every output token `max_tokens` allows, x 1.3.
 * The bound holds because the allowlist below admits text only (ruling R9):
 * no tokenizer yields more tokens than bytes of text. */
function holdEstimate(model: string, bodyBytes: number, maxTokens: number): number {
  const p = AI_PRICES[model];
  return Math.ceil(((bodyBytes * Math.max(p.input, p.cacheWrite) + maxTokens * p.output) * 13) / (10 * 1_000_000));
}

const roundUp100 = (micros: number): number => Math.ceil(micros / 100) * 100;

/** The largest max_tokens from MIN_SHRUNK_MAX_TOKENS to `ceiling` whose
 * hold, rounded up as placeHold rounds it, fits `balance`; null when even
 * the floor does not fit. */
function affordableMaxTokens(model: string, bodyBytes: number, ceiling: number, balance: number): number | null {
  const fits = (n: number) => roundUp100(holdEstimate(model, bodyBytes, n)) <= balance;
  let lo = MIN_SHRUNK_MAX_TOKENS;
  if (ceiling < lo || !fits(lo)) return null;
  let hi = ceiling;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (fits(mid)) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

function toAiUsage(model: string, usage: AnthropicUsage): AiUsage {
  return {
    model,
    inputTokens: tokens(usage.input_tokens) + tokens(usage.cache_creation_input_tokens) + tokens(usage.cache_read_input_tokens),
    outputTokens: tokens(usage.output_tokens),
    costMicros: costMicros(model, usage),
  };
}

/** Top-level request keys the proxy forwards: what core/llm sends (model,
 * max_tokens, stream, messages) plus plain-text options (ruling R9).
 * Everything else is refused, including tools, tool_choice, thinking,
 * top_p, top_k, inference_geo and speed: each is billed in ways the
 * byte-bounded hold does not cover. */
const ALLOWED_KEYS = new Set(["model", "max_tokens", "stream", "messages", "system", "temperature", "stop_sequences", "metadata"]);
const TEXT_BLOCK_KEYS = new Set(["type", "text", "cache_control"]);

const isObject = (x: unknown): x is Record<string, any> => !!x && typeof x === "object" && !Array.isArray(x);

/** Only the default 5-minute TTL: a 1-hour write costs 2x input. */
function cacheControlOk(x: Record<string, any>): boolean {
  const cc = x.cache_control;
  if (cc === undefined || cc === null) return true;
  return isObject(cc) && (cc.ttl === undefined || cc.ttl === "5m");
}

/** A string, or an array of `{type: "text", text, cache_control?}` blocks.
 * Images, documents, tool use and thinking are refused: they bill beyond
 * the body's bytes (an image or PDF page by area, not size). */
function textContentOk(content: unknown): boolean {
  if (typeof content === "string") return true;
  if (!Array.isArray(content)) return false;
  return content.every(
    (block) =>
      isObject(block) &&
      block.type === "text" &&
      typeof block.text === "string" &&
      Object.keys(block).every((k) => TEXT_BLOCK_KEYS.has(k)) &&
      cacheControlOk(block),
  );
}

/** True when the body is plain text the hold bounds. Checked before any
 * hold is placed. */
function supportedRequest(body: Record<string, any>): boolean {
  if (!Object.keys(body).every((k) => ALLOWED_KEYS.has(k))) return false;
  if (!Array.isArray(body.messages)) return false;
  const messageOk = (m: unknown) =>
    isObject(m) &&
    Object.keys(m).every((k) => k === "role" || k === "content") &&
    (m.role === "user" || m.role === "assistant") &&
    textContentOk(m.content);
  if (!body.messages.every(messageOk)) return false;
  return body.system === undefined || textContentOk(body.system);
}

const unauthorized = () => Response.json({ error: "unauthorized" }, { status: 401 });

/** The caller's account id, or the response to send. Credentials come from
 * headers here, not the body: the body is the Anthropic request itself.
 * Same checks as `verifyCaller` plus an approved device, but every refusal
 * is a 401 (an account being deleted is a 503, as on every route). */
async function aiCaller(request: Request, env: Env, db: SqlExecutor): Promise<string | Response> {
  const auth = request.headers.get("authorization") ?? "";
  const idToken = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
  const deviceKey = request.headers.get("x-whimbrel-device") ?? "";
  if (!idToken || !deviceKey) return unauthorized();
  let identity;
  try {
    identity = await verifyGoogleIdToken(idToken, allowedAudiences(env), googleJwks());
  } catch {
    return unauthorized();
  }
  if (!isSignInAllowed(identity.email, env)) return unauthorized();
  const account = await findAccountByGoogleSub(db, identity.googleSub);
  if (!account) return unauthorized();
  if (account.deletingSinceUnixMs != null) return Response.json({ error: "account_deleting" }, { status: 503 });
  const device = await findDeviceByPublicKey(db, account.id, deviceKey);
  if (!device || device.status !== "approved") return unauthorized();
  return account.id;
}

/** Reads an Anthropic SSE stream as it passes through and keeps the usage
 * seen so far: `message_start.message.usage`, overlaid with each
 * `message_delta.usage` (its counts are cumulative, so the last one wins). */
class SseMeter {
  private start: AnthropicUsage | null = null;
  private delta: AnthropicUsage | null = null;
  private pending = "";
  private readonly decoder = new TextDecoder();

  push(chunk: Uint8Array): void {
    this.pending += this.decoder.decode(chunk, { stream: true });
    const lines = this.pending.split(/\r?\n/);
    this.pending = lines.pop() ?? "";
    for (const line of lines) this.line(line);
  }

  /** Call once the stream ends or is cut off. */
  end(): void {
    this.line(this.pending + this.decoder.decode());
    this.pending = "";
  }

  private line(line: string): void {
    if (!line.startsWith("data:")) return;
    let event: any;
    try {
      event = JSON.parse(line.slice(5));
    } catch {
      return;
    }
    if (event?.type === "message_start" && isObject(event.message?.usage)) {
      this.start = { ...event.message.usage };
    } else if (event?.type === "message_delta" && isObject(event.usage)) {
      this.delta = { ...(this.delta ?? {}), ...event.usage };
    }
  }

  /** The usage to bill. Input comes from `message_start` (or, if it never
   * came, the body's byte count, the same bound the hold used). Output comes
   * from the last `message_delta` that carried `output_tokens`, or, when none
   * did (the stream was cut off first), the full `max_tokens` (ruling R9: a
   * hang-up must never cost less than what Anthropic may bill). */
  usage(bodyBytes: number, maxTokens: number): AnthropicUsage {
    const base: AnthropicUsage = this.start ?? { input_tokens: bodyBytes };
    const merged: AnthropicUsage = { ...base, ...(this.delta ?? {}) };
    if (typeof this.delta?.output_tokens !== "number") merged.output_tokens = maxTokens;
    return merged;
  }
}

/** POST /ai/v1/messages: Anthropic's Messages API on Whimbrel's key, paid
 * from the account's AI credit. Refuses anything the price table cannot
 * bill, places a hold, forwards the body (with `max_tokens` capped) and
 * passes Anthropic's answer back.
 *
 * A stream goes through one pass-through TransformStream that reads usage as
 * the bytes pass. It settles once: at the end of the stream, or at once when
 * the client hangs up, from the usage seen so far, after aborting the
 * upstream request so Anthropic stops generating. `ctx.waitUntil` holds the
 * Worker open until that settle is done. */
export async function handleAiMessages(
  request: Request,
  env: Env,
  ctx: ExecutionContext,
  db: SqlExecutor,
  fetcher: typeof fetch = fetch,
  now: number = Date.now(),
): Promise<Response> {
  const apiKey = env.ANTHROPIC_API_KEY;
  if (!apiKey) return Response.json({ error: "ai_unavailable" }, { status: 503 });

  // Set while a hold is open that nothing else will settle. The catch below
  // runs it: before Anthropic answers it gives the hold back; after a
  // successful answer it charges the whole hold.
  let unsettled: (() => Promise<void>) | null = null;
  try {
    const caller = await aiCaller(request, env, db);
    if (caller instanceof Response) return caller;
    const accountId = caller;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `ai:${accountId}`, 60, 60))) return rateLimited();

    let body: any;
    try {
      body = JSON.parse(await request.text());
    } catch {
      return Response.json({ error: "invalid JSON body" }, { status: 400 });
    }
    if (!isObject(body)) return Response.json({ error: "invalid JSON body" }, { status: 400 });
    const model = body.model;
    if (typeof model !== "string" || !Object.hasOwn(AI_PRICES, model)) {
      return Response.json({ error: "unknown_model" }, { status: 400 });
    }
    if (!Number.isSafeInteger(body.max_tokens) || body.max_tokens < 1) {
      return Response.json({ error: "invalid max_tokens" }, { status: 400 });
    }
    if (!supportedRequest(body)) return Response.json({ error: "unsupported_request" }, { status: 400 });
    if (body.max_tokens > MAX_TOKENS_CAP) body.max_tokens = MAX_TOKENS_CAP;
    // Ruling R10: always send the re-serialised object that was checked,
    // never the client's raw text. A raw body with a duplicated key (two
    // "model"s, two "messages") could be read differently upstream than by
    // JSON.parse here. The hold is sized on these same bytes.
    let bodyText = JSON.stringify(body);
    let bodyBytes = new TextEncoder().encode(bodyText).byteLength;

    let amount = holdEstimate(model, bodyBytes, body.max_tokens);
    let hold = await placeHold(db, accountId, amount, now);
    if (!hold) {
      // placeHold refuses for short credit or for 4 holds already open.
      const balance = await aiBalance(db, accountId);
      if (balance >= roundUp100(amount)) return Response.json({ error: "too_many_in_flight" }, { status: 429 });
      // Ruling R16: short credit forwards the largest max_tokens the
      // balance can hold (floor 512), so the service stops at $0, not at
      // the worst-case hold. The search uses the current body's bytes; a
      // smaller max_tokens never has more digits, so the re-serialised body
      // is no longer and its hold still fits.
      const shrunk = affordableMaxTokens(model, bodyBytes, body.max_tokens - 1, balance);
      if (shrunk === null) return Response.json({ error: "credit_exhausted" }, { status: 402 });
      body.max_tokens = shrunk;
      bodyText = JSON.stringify(body);
      bodyBytes = new TextEncoder().encode(bodyText).byteLength;
      amount = holdEstimate(model, bodyBytes, shrunk);
      hold = await placeHold(db, accountId, amount, now);
      if (!hold) {
        // Another request took credit or a slot in between.
        const short = (await aiBalance(db, accountId)) < roundUp100(amount);
        return short
          ? Response.json({ error: "credit_exhausted" }, { status: 402 })
          : Response.json({ error: "too_many_in_flight" }, { status: 429 });
      }
    }
    // The forwarded body, the hold and a hang-up charge all use this value.
    const maxTokens: number = body.max_tokens;
    const free = () => settleHold(db, hold, toAiUsage(model, {}), now);
    unsettled = free;

    const abort = new AbortController();
    let upstream: Response;
    try {
      upstream = await fetcher(ANTHROPIC_URL, {
        method: "POST",
        // Fixed headers only: nothing from the client is forwarded.
        headers: { "content-type": "application/json", "x-api-key": apiKey, "anthropic-version": ANTHROPIC_VERSION },
        body: bodyText,
        signal: abort.signal,
      });
    } catch (err) {
      console.error("handleAiMessages: upstream fetch failed", err);
      unsettled = null;
      await free();
      return Response.json({ error: "ai_upstream_unavailable" }, { status: 502 });
    }

    const headers = new Headers();
    for (const h of ["content-type", "request-id", "retry-after"]) {
      const v = upstream.headers.get(h);
      if (v) headers.set(h, v);
    }

    // The header is a convenience: a failed balance read leaves it out and
    // must never stop the bill below.
    const creditHeader = async () => {
      try {
        headers.set("x-whimbrel-credit", String(await aiBalance(db, accountId)));
      } catch (err) {
        console.error("handleAiMessages: balance read failed", err);
      }
    };

    if (!upstream.ok || !upstream.body) {
      // Anthropic does not bill an error, so neither do we.
      unsettled = null;
      await free();
      await creditHeader();
      return new Response(upstream.body, { status: upstream.status, headers });
    }

    // Anthropic accepted the request, so from here on it may bill: the hold
    // is never given back at zero cost again. Until a settle succeeds, a
    // throw charges the whole hold (ruling R9).
    unsettled = () => settleHold(db, hold, null, Date.now());
    // Read before the settle can start, so it is the balance after the hold.
    await creditHeader();

    if (!(upstream.headers.get("content-type") ?? "").includes("text/event-stream")) {
      // A JSON answer is complete (and billed) before its first byte, so
      // read it whole and settle from its usage before answering.
      const text = await upstream.text();
      let usage: AnthropicUsage | null = null;
      try {
        const parsed = JSON.parse(text)?.usage;
        if (isObject(parsed)) usage = parsed;
      } catch {
        // No readable bill: the hold is the charge, as settleHold documents.
      }
      await settleHold(db, hold, usage ? toAiUsage(model, usage) : null, Date.now());
      unsettled = null;
      return new Response(text, { status: upstream.status, headers });
    }

    const meter = new SseMeter();
    let settled = false;
    let markDone!: () => void;
    const done = new Promise<void>((resolve) => (markDone = resolve));
    const settleOnce = async () => {
      if (settled) return;
      settled = true;
      meter.end();
      try {
        await settleHold(db, hold, toAiUsage(model, meter.usage(bodyBytes, maxTokens)), Date.now());
      } catch (err) {
        console.error("handleAiMessages: settle failed", err);
      } finally {
        markDone();
      }
    };
    const hangUp = async () => {
      abort.abort();
      await settleOnce();
    };
    const metered = new TransformStream<Uint8Array, Uint8Array>({
      transform(chunk, controller) {
        meter.push(chunk);
        controller.enqueue(chunk);
      },
      flush: settleOnce,
      cancel: hangUp,
    });
    ctx.waitUntil(done);
    // From here the stream's own settleOnce owns the hold.
    unsettled = null;
    // An upstream error mid-stream lands here; settle from what was seen.
    upstream.body.pipeTo(metered.writable).catch(hangUp);
    return new Response(metered.readable, { status: upstream.status, headers });
  } catch (err) {
    console.error("handleAiMessages failed", err);
    if (unsettled) await unsettled().catch((e) => console.error("handleAiMessages: settling hold failed", e));
    return Response.json({ error: "ai_unavailable" }, { status: 503 });
  }
}
