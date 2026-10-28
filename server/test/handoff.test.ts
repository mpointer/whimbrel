import { env } from "cloudflare:test";
import { it, expect } from "vitest";
import { storeWrappedKeyForHandoff, fetchAndConsumeWrappedKey } from "../src/handoff";

it("returns null when nothing has been stored for a pending id", async () => {
  const result = await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, "no-such-id");
  expect(result).toBeNull();
});

it("stores and then fetches-and-consumes a wrapped key blob", async () => {
  await storeWrappedKeyForHandoff(env.ENROLLMENT_HANDOFF, "pending-1", "the-wrapped-key-blob");

  const first = await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, "pending-1");
  expect(first).toBe("the-wrapped-key-blob");

  const second = await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, "pending-1");
  expect(second).toBeNull();
});

it("keeps separate pending ids independent", async () => {
  await storeWrappedKeyForHandoff(env.ENROLLMENT_HANDOFF, "pending-a", "blob-a");
  await storeWrappedKeyForHandoff(env.ENROLLMENT_HANDOFF, "pending-b", "blob-b");

  expect(await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, "pending-a")).toBe("blob-a");
  expect(await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, "pending-b")).toBe("blob-b");
});
