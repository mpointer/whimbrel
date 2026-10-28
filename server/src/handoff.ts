import type { KVNamespace } from "@cloudflare/workers-types";

/** How long a pending device enrollment's wrapped group key waits in KV
 * for the requesting device to fetch it: long enough for a human to
 * approve on another device, short enough not to leave stale blobs
 * around indefinitely. */
const HANDOFF_TTL_SECONDS = 300;

function handoffKey(pendingEnrollmentId: string): string {
  return `handoff:${pendingEnrollmentId}`;
}

export async function storeWrappedKeyForHandoff(
  kv: KVNamespace,
  pendingEnrollmentId: string,
  wrappedKeyBlob: string,
): Promise<void> {
  await kv.put(handoffKey(pendingEnrollmentId), wrappedKeyBlob, {
    expirationTtl: HANDOFF_TTL_SECONDS,
  });
}

/** Fetches the wrapped key blob for a pending enrollment and deletes it
 * so it can only be consumed once. Returns null if nothing is stored
 * (never enrolled, already consumed, or expired). */
export async function fetchAndConsumeWrappedKey(
  kv: KVNamespace,
  pendingEnrollmentId: string,
): Promise<string | null> {
  const key = handoffKey(pendingEnrollmentId);
  const value = await kv.get(key);
  if (value === null) return null;
  await kv.delete(key);
  return value;
}
