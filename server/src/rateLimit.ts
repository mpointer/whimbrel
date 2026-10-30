import type { KVNamespace } from "@cloudflare/workers-types";

/** Fixed-window counter in KV: true while `key` has been used fewer than
 * `limit` times in the current `windowSeconds` window, counting this call.
 * On a KV error (an outage, or KV's one-write-per-second-per-key limit) it
 * allows the call, so sync never locks people out.
 * ponytail: KV is not atomic, so a burst can overshoot the limit by a few; a
 * Durable Object counter if exact limits ever matter. */
export async function rateLimit(
  kv: KVNamespace,
  key: string,
  limit: number,
  windowSeconds: number,
): Promise<boolean> {
  const window = Math.floor(Date.now() / 1000 / windowSeconds);
  const k = `rl:${key}:${window}`;
  try {
    const count = Number((await kv.get(k)) ?? 0);
    if (count >= limit) return false;
    // KV's minimum TTL is 60 seconds.
    await kv.put(k, String(count + 1), { expirationTtl: Math.max(60, windowSeconds) });
    return true;
  } catch (err) {
    console.error("rateLimit: KV failed, allowing", err);
    return true;
  }
}

const IPV4 = /^(\d{1,3}\.){3}\d{1,3}$/;

/** The bucket for a client IP: IPv4 (and IPv4-mapped ::ffff:a.b.c.d) as the
 * IPv4 address, any other IPv6 by its /64 prefix (one customer usually holds
 * a whole /64, so per-address limits are free to dodge). Malformed input is
 * its own bucket. Never throws. */
export function ipBucket(ip: string): string {
  if (IPV4.test(ip)) return ip;
  const mapped = /^::ffff:((\d{1,3}\.){3}\d{1,3})$/i.exec(ip);
  if (mapped) return mapped[1];
  const groups = expandIpv6(ip);
  return groups ? `${groups.slice(0, 4).join(":")}::/64` : ip;
}

/** The eight hextets of an IPv6 address (an embedded IPv4 tail becomes the
 * last two), normalised, or null if it isn't one. */
function expandIpv6(ip: string): string[] | null {
  let s = ip.toLowerCase();
  const tail = /^(.*:)((?:\d{1,3}\.){3}\d{1,3})$/.exec(s);
  if (tail) {
    const o = tail[2].split(".").map(Number);
    if (o.some((n) => n > 255)) return null;
    s = `${tail[1]}${((o[0] << 8) | o[1]).toString(16)}:${((o[2] << 8) | o[3]).toString(16)}`;
  }
  const halves = s.split("::");
  if (halves.length > 2) return null;
  const left = halves[0] ? halves[0].split(":") : [];
  const right = halves.length === 2 && halves[1] ? halves[1].split(":") : [];
  const fill = 8 - left.length - right.length;
  if (halves.length === 2 ? fill < 1 : fill !== 0) return null;
  const all = [...left, ...Array<string>(halves.length === 2 ? fill : 0).fill("0"), ...right];
  if (!all.every((g) => /^[0-9a-f]{1,4}$/.test(g))) return null;
  return all.map((g) => parseInt(g, 16).toString(16));
}

export const rateLimited = () => Response.json({ error: "rate_limited" }, { status: 429 });
