import type { Env } from "./index";
import { isUnlimited } from "./allowlist";
import { deleteAccountDatabase } from "./turso";
import { EMAIL_FROM, EMAIL_REPLY_TO, sendEmail } from "./email";
import {
  accountsToDelete,
  accountsToWarn,
  backfillLastSeen,
  claimInactiveForDeletion,
  clearResetNotice,
  deleteAccountRows,
  deletePendingDevice,
  expiredRetiredDatabases,
  findAccountById,
  forgetRetiredDatabase,
  isCurrentDatabase,
  markWarned,
  pendingResetNotices,
  recoverStuckReset,
  retiredDatabasesForAccount,
  staleDeletionClaims,
  stuckResettingDevices,
  swapDevices,
  type AccountRecord,
  type SqlExecutor,
} from "./accounts";

const MINUTE = 60 * 1000;
const DAY = 24 * 60 * MINUTE;
const INACTIVE_MS = 180 * DAY;
const WARNING_MS = 14 * DAY;
const SWAP_EXPIRY_MS = 7 * DAY;
const RETIRED_DB_MS = 30 * DAY;
const STUCK_RESET_MS = 10 * MINUTE;
const STUCK_DELETION_MS = 10 * MINUTE;

const turso = (env: Env) => ({ platformApiToken: env.TURSO_PLATFORM_API_TOKEN, organizationSlug: env.TURSO_ORG_SLUG });

/** Deletes a retired database unless an account uses it now (checked right
 * before the Turso call); the row goes only after Turso confirms. */
async function deleteRetired(db: SqlExecutor, env: Env, name: string): Promise<void> {
  if (await isCurrentDatabase(db, name)) return;
  await deleteAccountDatabase(turso(env), name);
  await forgetRetiredDatabase(db, name);
}

/** Deletes the account's data database, its retired databases, then its
 * registry rows. The caller must already hold the deletion claim
 * (`deleting_since_unix_ms`), which shuts every route out of the account and
 * fences resets. Everything is read fresh after the claim; the caller's
 * account is used only for its id, since its snapshot may predate a reset.
 * Databases go first, so a failure leaves the claimed account for the cron
 * to finish. A retired database that some account is using now is never
 * deleted (the account's own current one is deleted as current). */
export async function deleteAccount(db: SqlExecutor, env: Env, claimed: Pick<AccountRecord, "id">): Promise<void> {
  const account = await findAccountById(db, claimed.id);
  if (!account) return;
  for (const name of await retiredDatabasesForAccount(db, account.id)) await deleteRetired(db, env, name);
  await deleteAccountDatabase(turso(env), account.dataDbName);
  await deleteAccountRows(db, account.id);
}

const longDate = (ms: number) =>
  new Date(ms).toLocaleDateString("en-US", { month: "long", day: "numeric", year: "numeric", timeZone: "UTC" });

/** "14:05", UTC. */
const time = (ms: number) => new Date(ms).toISOString().slice(11, 16);

/** Runs `step` on each item; one failure is logged and the rest carry on. */
async function each<T>(label: string, items: T[], step: (item: T) => Promise<boolean | void>): Promise<number> {
  let done = 0;
  for (const item of items) {
    try {
      if ((await step(item)) !== false) done++;
    } catch (err) {
      console.error(`inactivity: ${label} failed`, err);
    }
  }
  return done;
}

/** The daily cron. Every write that follows a read is conditional, because a
 * device can make contact (touchLastSeen) at any point during the run. Without
 * RESEND_API_KEY nothing is sent, so no one is warned and no one is deleted. */
export async function runInactivity(
  db: SqlExecutor,
  env: Env,
  now: number,
  fetcher: typeof fetch,
): Promise<{ warned: number; deleted: number; expiredSwaps: number }> {
  await backfillLastSeen(db, now);

  // A reset that crashed mid-way hands its claim back so the device can retry.
  const resetCutoff = now - STUCK_RESET_MS;
  await each("reset recovery", await stuckResettingDevices(db, resetCutoff), (id) => recoverStuckReset(db, id, resetCutoff));

  // A failure keeps the row, so it is retried tomorrow.
  await each("retired database", await expiredRetiredDatabases(db, now - RETIRED_DB_MS), (name) => deleteRetired(db, env, name));

  // A deletion (cron or self-serve) that failed after its claim is finished here.
  const finished = await each("unfinished deletion", await staleDeletionClaims(db, now - STUCK_DELETION_MS), (a) =>
    deleteAccount(db, env, a),
  );

  const apiKey = env.RESEND_API_KEY;
  let warned = 0;
  if (apiKey) {
    await each("reset notice", await pendingResetNotices(db), async (a) => {
      const sent = await sendEmail(
        apiKey,
        {
          from: EMAIL_FROM,
          to: a.email,
          replyTo: EMAIL_REPLY_TO,
          subject: "Your Whimbrel sync was reset",
          text:
            `On ${longDate(a.resetAtUnixMs)} at ${time(a.resetAtUnixMs)} UTC, a new computer started your Whimbrel sync fresh. ` +
            "It made a new encryption key, and all your other devices were signed out of sync. " +
            "Nothing on your devices themselves was deleted.\n\nIf this wasn't you, reply to this email.",
        },
        fetcher,
      );
      if (sent) await clearResetNotice(db, a.id, a.resetAtUnixMs);
      return sent;
    });

    const seenBefore = now - INACTIVE_MS;
    const toWarn = (await accountsToWarn(db, seenBefore)).filter((a) => !isUnlimited(a.email, env.UNLIMITED_EMAILS));
    warned = await each("warning", toWarn, async (a) => {
      const date = longDate(now + WARNING_MS);
      const sent = await sendEmail(
        apiKey,
        {
          from: EMAIL_FROM,
          to: a.email,
          replyTo: EMAIL_REPLY_TO,
          subject: `Your Whimbrel sync data will be deleted on ${date}`,
          text:
            "None of your devices has connected to Whimbrel sync for six months. " +
            `Open Whimbrel on any of your devices before ${date} to keep your synced data. ` +
            "Nothing on your devices themselves will be deleted.",
        },
        fetcher,
      );
      // A failed send leaves warned_at empty, so it is retried tomorrow and
      // the 14 days only start once a warning has actually gone out.
      if (sent) await markWarned(db, a.id, now, seenBefore);
      return sent;
    });
  }

  // The claim re-checks inactivity atomically, so an account that made
  // contact since this read is skipped, and from the claim on no route can
  // use it (so nothing can reconnect halfway through the deletion).
  const toDelete = (await accountsToDelete(db, now - WARNING_MS)).filter((a) => !isUnlimited(a.email, env.UNLIMITED_EMAILS));
  // The claim is the commit point: once it succeeds the account is deleted,
  // by this run or, if this attempt fails, by a later run's unfinished step.
  const deleted =
    finished +
    (await each("deletion", toDelete, async (a) => {
      if (!(await claimInactiveForDeletion(db, a.id, now, now - WARNING_MS))) return false;
      await deleteAccount(db, env, a);
    }));

  // Abandoned swaps expire, except one that a kept swap still points at
  // (directly or down a chain), so expiry never leaves a dangling link.
  const swaps = await swapDevices(db);
  const keep = new Set(swaps.filter((s) => s.status === "resetting" || s.enrolledAtUnixMs >= now - SWAP_EXPIRY_MS).map((s) => s.id));
  for (let grew = true; grew; ) {
    grew = false;
    for (const s of swaps) {
      if (keep.has(s.id) && swaps.some((t) => t.id === s.replacesDeviceId) && !keep.has(s.replacesDeviceId)) {
        keep.add(s.replacesDeviceId);
        grew = true;
      }
    }
  }
  const expiredSwaps = await each(
    "swap expiry",
    swaps.filter((s) => !keep.has(s.id)),
    (s) => deletePendingDevice(db, s.id),
  );

  return { warned, deleted, expiredSwaps };
}
