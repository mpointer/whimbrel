/** The Worker's cron triggers. wrangler.jsonc `triggers.crons` must list
 * exactly these two strings (a test checks it); `scheduled` dispatches on
 * them. */

/** Daily at 09:00 UTC: inactivity (warnings, deletions, expired swaps,
 * reset notices, retired databases) and the stale AI hold settle. */
export const DAILY_CRON = "0 9 * * *";

/** Hourly at minute 30: the yearly Plus monthly grants only. */
export const YEARLY_GRANTS_CRON = "30 * * * *";
