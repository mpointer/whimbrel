import { it, expect } from "vitest";
import { env } from "cloudflare:test";
import type { KVNamespace } from "@cloudflare/workers-types";
import { rateLimit } from "../src/rateLimit";

it("allows up to the limit in a window, then refuses", async () => {
  const key = `test:${crypto.randomUUID()}`;
  const results = [];
  for (let i = 0; i < 4; i++) results.push(await rateLimit(env.ENROLLMENT_HANDOFF, key, 3, 3600));
  expect(results).toEqual([true, true, true, false]);
  // Keys are independent.
  expect(await rateLimit(env.ENROLLMENT_HANDOFF, `${key}-other`, 3, 3600)).toBe(true);
});

it("fails open when KV throws", async () => {
  const broken = {
    get: async () => {
      throw new Error("kv down");
    },
  } as unknown as KVNamespace;
  expect(await rateLimit(broken, "k", 1, 60)).toBe(true);
});

it("buckets IPv6 by /64 and leaves IPv4 alone", async () => {
  const { ipBucket } = await import("../src/rateLimit");
  expect(ipBucket("203.0.113.9")).toBe("203.0.113.9");
  expect(ipBucket("2001:DB8:0:1::5")).toBe("2001:db8:0:1::/64");
  expect(ipBucket("2001:0db8:0000:0001:ffff:1:2:3")).toBe("2001:db8:0:1::/64");
  expect(ipBucket("2001:db8::1")).toBe("2001:db8:0:0::/64");
  expect(ipBucket("::ffff:198.51.100.4")).toBe("198.51.100.4");
});

it("ipv4-mapped maps to ipv4", async () => {
  const { ipBucket } = await import("../src/rateLimit");
  expect(ipBucket("::ffff:198.51.100.4")).toBe("198.51.100.4");
  expect(ipBucket("::FFFF:198.51.100.4")).toBe("198.51.100.4");
  expect(ipBucket("198.51.100.4")).toBe("198.51.100.4");
});

it("embedded ipv4 in a non-mapped ipv6 buckets by /64", async () => {
  const { ipBucket } = await import("../src/rateLimit");
  const bucket = "2001:db8:1:2::/64";
  for (const ip of ["2001:db8:1:2::1.1.1.1", "2001:db8:1:2::2.2.2.2", "2001:db8:1:2:3:4:9.9.9.9", "2001:db8:1:2::ffff:5.6.7.8"]) {
    expect(ipBucket(ip), ip).toBe(bucket);
  }
  // Translated and compatible forms are not "mapped": they bucket by /64 too.
  expect(ipBucket("::ffff:0:1.2.3.4")).toBe("0:0:0:0::/64");
  expect(ipBucket("::1.2.3.4")).toBe("0:0:0:0::/64");
  expect(ipBucket("64:ff9b::192.0.2.33")).toBe("64:ff9b:0:0::/64");
});

it("malformed input does not throw", async () => {
  const { ipBucket } = await import("../src/rateLimit");
  for (const ip of ["", "not-an-ip", ":::", "1:2:3", "2001:db8::1::2", "1:2:3:4:5:6:7:8:9", "2001:db8::gggg", "::1.2.3.999", "a.b:c.d"]) {
    expect(ipBucket(ip), ip).toBe(ip);
  }
});
