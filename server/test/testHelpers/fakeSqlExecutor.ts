import type { SqlExecutor, SqlRow, SqlStatement } from "../../src/accounts";

/** Minimal in-memory fake covering exactly the queries accounts.ts issues
 * — not a general SQL engine, just enough to test real accounts.ts query
 * logic (and callers built on top of it) without a live Turso database.
 * Shared between accounts.test.ts and index.test.ts so there is one
 * source of truth for "what accounts.ts's SQL actually looks like". */
export class FakeSqlExecutor implements SqlExecutor {
  private accounts = new Map<
    string,
    {
      id: string;
      email: string;
      googleSub: string;
      dataDbName: string;
      dataDbUrl: string;
      lastSeenUnixMs?: number | null;
      warnedAtUnixMs?: number | null;
      resetNoticePendingUnixMs?: number | null;
      deletingSinceUnixMs?: number | null;
      plan?: "free" | "cloud";
      planUntilUnixMs?: number | null;
      planInterval?: "month" | "year" | null;
      planRenews?: boolean;
      stripeCustomerId?: string | null;
      stripeSubscriptionId?: string | null;
      aiCreditMicros?: number;
      aiOpenHolds?: number;
    }
  >();
  private aiLedger: SqlRow[] = [];
  private aiHolds = new Map<string, SqlRow>();
  private devices = new Map<string, SqlRow>();
  private waitlist: SqlRow[] = [];
  private retired: SqlRow[] = [];
  private creations: SqlRow[] = [];
  private stripeEvents = new Set<string>();
  /** SQLite's changes(): rows changed by the last UPDATE that sets it.
   * Only the AI credit statements that read it also set it. */
  private lastChanges = 0;
  /** Number of batches run (test peek). */
  batches = 0;

  /** Test seam: simulate a pending inactivity warning. */
  markWarned(accountId: string, at: number) {
    const a = this.accounts.get(accountId);
    if (a) this.accounts.set(accountId, { ...a, warnedAtUnixMs: at });
  }

  /** Test seam: set an account's plan columns (omitted optional ones become null). */
  setPlan(
    accountId: string,
    p: {
      plan: "free" | "cloud";
      planUntilUnixMs: number | null;
      planInterval?: "month" | "year" | null;
      planRenews?: boolean;
      stripeCustomerId?: string | null;
      stripeSubscriptionId?: string | null;
    },
  ) {
    const a = this.accounts.get(accountId);
    if (!a) throw new Error(`setPlan: no account ${accountId}`);
    this.accounts.set(accountId, {
      ...a,
      plan: p.plan,
      planUntilUnixMs: p.planUntilUnixMs,
      planInterval: p.planInterval ?? null,
      ...(p.planRenews === undefined ? {} : { planRenews: p.planRenews }),
      stripeCustomerId: p.stripeCustomerId ?? null,
      stripeSubscriptionId: p.stripeSubscriptionId ?? null,
    });
  }

  /** Test peeks at columns the app queries never SELECT. */
  rawDevice(id: string): SqlRow | undefined {
    return this.devices.get(id);
  }
  rawAccount(id: string) {
    return this.accounts.get(id);
  }
  rawWaitlist(): SqlRow[] {
    return this.waitlist;
  }
  rawRetired(): SqlRow[] {
    return this.retired;
  }
  rawAiLedger(): SqlRow[] {
    return this.aiLedger;
  }
  rawAiHolds(): SqlRow[] {
    return [...this.aiHolds.values()];
  }

  private hooks: { prefix: string; fn: () => Promise<void> }[] = [];

  /** Test seam for races: runs `fn` once, right after the first query whose
   * text starts with `prefix` (its result is already computed). */
  after(prefix: string, fn: () => Promise<void>) {
    this.hooks.push({ prefix, fn });
  }

  /** All statements run synchronously, so no other call can interleave,
   * on live state; a throw restores the snapshot taken first. That is the
   * same all-or-nothing contract as one SQLite transaction. */
  async batch(stmts: SqlStatement[]): Promise<SqlRow[][]> {
    const fields = ["accounts", "devices", "waitlist", "retired", "creations", "stripeEvents", "aiLedger", "aiHolds", "lastChanges"] as const;
    const self = this as unknown as Record<string, unknown>;
    const snapshot = fields.map((f) => structuredClone(self[f]));
    try {
      const out = stmts.map((s) => this.run(s.sql, s.params));
      this.batches++;
      return out;
    } catch (err) {
      fields.forEach((f, i) => (self[f] = snapshot[i]));
      throw err;
    }
  }

  async execute(sql: string, params: unknown[]): Promise<SqlRow[]> {
    const rows = this.run(sql, params);
    const q = sql.replace(/\s+/g, " ").trim();
    const hook = this.hooks.find((h) => q.startsWith(h.prefix));
    if (hook) {
      this.hooks.splice(this.hooks.indexOf(hook), 1);
      await hook.fn();
    }
    return rows;
  }

  private run(sql: string, params: unknown[]): SqlRow[] {
    const q = sql.replace(/\s+/g, " ").trim();
    const accountRow = (a: {
      id: string;
      email: string;
      googleSub: string;
      dataDbName: string;
      dataDbUrl: string;
      deletingSinceUnixMs?: number | null;
      plan?: "free" | "cloud";
      planUntilUnixMs?: number | null;
      planInterval?: "month" | "year" | null;
      planRenews?: boolean;
      stripeCustomerId?: string | null;
      stripeSubscriptionId?: string | null;
    }) => ({
      id: a.id,
      email: a.email,
      google_sub: a.googleSub,
      data_db_name: a.dataDbName,
      data_db_url: a.dataDbUrl,
      deleting_since_unix_ms: a.deletingSinceUnixMs ?? null,
      plan: a.plan ?? "free",
      plan_until_unix_ms: a.planUntilUnixMs ?? null,
      plan_interval: a.planInterval ?? null,
      stripe_customer_id: a.stripeCustomerId ?? null,
      stripe_subscription_id: a.stripeSubscriptionId ?? null,
      plan_renews: a.planRenews === false ? 0 : 1,
    });

    if (q === "SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE google_sub = ?") {
      const [googleSub] = params as [string];
      const found = [...this.accounts.values()].find((a) => a.googleSub === googleSub);
      return found ? [accountRow(found)] : [];
    }
    if (q.startsWith("INSERT OR IGNORE INTO account")) {
      const [id, email, googleSub, dataDbName, dataDbUrl] = params as [string, string, string, string, string];
      const alreadyExists = [...this.accounts.values()].some((a) => a.googleSub === googleSub);
      if (!alreadyExists) {
        this.accounts.set(id, { id, email, googleSub, dataDbName, dataDbUrl });
      }
      return [];
    }
    // NOTE: this check must come BEFORE the listDevicesForAccount check
    // below, since that query's text ("...WHERE account_id = ?") is a
    // string prefix of this one's ("...WHERE account_id = ? AND public_key = ?").
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms, replaces_device_id FROM device WHERE account_id = ? AND public_key = ?",
      )
    ) {
      const [accountId, publicKey] = params as [string, string];
      const found = [...this.devices.values()].find(
        (d) => d.account_id === accountId && d.public_key === publicKey,
      );
      return found ? [found] : [];
    }
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms, replaces_device_id FROM device WHERE id = ?",
      )
    ) {
      const [deviceId] = params as [string];
      const found = this.devices.get(deviceId);
      return found ? [found] : [];
    }
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms, replaces_device_id FROM device WHERE account_id = ?",
      )
    ) {
      // Map iteration is insertion order, which stands in for ORDER BY rowid.
      const [accountId] = params as [string];
      return [...this.devices.values()].filter((d) => d.account_id === accountId);
    }
    // NOTE: this check must come BEFORE the plain count check below, since
    // the plain count query text is a string prefix of this one.
    if (q.startsWith("SELECT COUNT(*) as count FROM device WHERE account_id = ? AND status != 'revoked'")) {
      const [accountId] = params as [string];
      const count = [...this.devices.values()].filter(
        (d) => d.account_id === accountId && d.status !== "revoked",
      ).length;
      return [{ count }];
    }
    if (q.startsWith("SELECT COUNT(*) as count FROM device WHERE account_id = ?")) {
      const [accountId] = params as [string];
      const count = [...this.devices.values()].filter((d) => d.account_id === accountId).length;
      return [{ count }];
    }
    if (q.startsWith("INSERT INTO device")) {
      const [id, accountId, publicKey, status, enrolledAtUnixMs, platform] = params as [
        string,
        string,
        string,
        string,
        number,
        string,
      ];
      this.devices.set(id, {
        id,
        account_id: accountId,
        public_key: publicKey,
        name: null,
        platform,
        status,
        enrolled_at_unix_ms: enrolledAtUnixMs,
        revoked_at_unix_ms: null,
        last_seen_unix_ms: null,
        replaces_device_id: null,
        resetting_since_unix_ms: null,
      });
      return [];
    }
    if (q.startsWith("UPDATE device SET status = 'approved' WHERE id = ? AND status IN ('pending', 'approved') RETURNING id")) {
      const [deviceId] = params as [string];
      const device = this.devices.get(deviceId);
      if (device?.status !== "pending" && device?.status !== "approved") return [];
      this.devices.set(deviceId, { ...device, status: "approved" });
      return [{ id: deviceId }];
    }
    if (q.startsWith("UPDATE device SET status = 'revoked', revoked_at_unix_ms = ? WHERE id = ?")) {
      const [revokedAtUnixMs, deviceId] = params as [number, string];
      const device = this.devices.get(deviceId);
      if (device) {
        this.devices.set(deviceId, { ...device, status: "revoked", revoked_at_unix_ms: revokedAtUnixMs });
      }
      return [];
    }
    if (q.startsWith("UPDATE device SET name = ? WHERE id = ?")) {
      const [name, deviceId] = params as [string, string];
      const device = this.devices.get(deviceId);
      if (device) {
        this.devices.set(deviceId, { ...device, name });
      }
      return [];
    }
    if (q.startsWith("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?")) {
      const [now, deviceId] = params as [number, string];
      const device = this.devices.get(deviceId);
      if (device) this.devices.set(deviceId, { ...device, last_seen_unix_ms: now });
      return [];
    }
    if (
      q ===
      "UPDATE account SET last_seen_unix_ms = ?, warned_at_unix_ms = NULL WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id"
    ) {
      const [now, accountId] = params as [number, string];
      const account = this.accounts.get(accountId);
      if (!account || account.deletingSinceUnixMs != null) return [];
      this.accounts.set(accountId, { ...account, lastSeenUnixMs: now, warnedAtUnixMs: null });
      return [{ id: accountId }];
    }
    if (q.startsWith("UPDATE device SET replaces_device_id = ? WHERE id = ?")) {
      const [replaces, deviceId] = params as [string | null, string];
      const device = this.devices.get(deviceId);
      if (device) this.devices.set(deviceId, { ...device, replaces_device_id: replaces });
      return [];
    }
    // NOTE: must come BEFORE the plain "DELETE FROM device WHERE id = ?" (a prefix of this one).
    if (
      q ===
      "DELETE FROM device WHERE id = ? AND status = 'pending' AND NOT EXISTS (SELECT 1 FROM device d2 WHERE d2.replaces_device_id = device.id AND d2.status IN ('pending', 'resetting'))"
    ) {
      const id = params[0] as string;
      const pointedAt = [...this.devices.values()].some(
        (d) => d.replaces_device_id === id && (d.status === "pending" || d.status === "resetting"),
      );
      if (this.devices.get(id)?.status === "pending" && !pointedAt) this.devices.delete(id);
      return [];
    }
    if (q.startsWith("DELETE FROM device WHERE id = ?")) {
      this.devices.delete(params[0] as string);
      return [];
    }
    if (
      q ===
      "UPDATE account SET data_db_name = ?, data_db_url = ?, reset_notice_pending_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id"
    ) {
      const [dataDbName, dataDbUrl, resetNoticePendingUnixMs, accountId] = params as [string, string, number, string];
      const account = this.accounts.get(accountId);
      if (!account || account.deletingSinceUnixMs != null) return [];
      this.accounts.set(accountId, { ...account, dataDbName, dataDbUrl, resetNoticePendingUnixMs });
      return [{ id: accountId }];
    }
    if (
      q ===
      "INSERT INTO account_creation (ip_bucket, created_at_unix_ms) SELECT ?, ? WHERE (SELECT COUNT(*) FROM account_creation WHERE ip_bucket = ? AND created_at_unix_ms > ?) < ? RETURNING ip_bucket"
    ) {
      const [ip_bucket, created_at_unix_ms, , since, limit] = params as [string, number, string, number, number];
      const used = this.creations.filter((c) => c.ip_bucket === ip_bucket && (c.created_at_unix_ms as number) > since).length;
      if (used >= limit) return [];
      this.creations.push({ ip_bucket, created_at_unix_ms });
      return [{ ip_bucket }];
    }
    if (q.startsWith("INSERT INTO retired_database (name, account_id, retired_at_unix_ms) VALUES (?, ?, ?)")) {
      const [name, account_id, retired_at_unix_ms] = params;
      this.retired.push({ name, account_id, retired_at_unix_ms });
      return [];
    }
    if (
      q.startsWith(
        "UPDATE device SET status = 'resetting', resetting_since_unix_ms = ? WHERE id = ? AND status = 'pending' AND replaces_device_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM account WHERE account.id = device.account_id AND account.deleting_since_unix_ms IS NOT NULL) RETURNING id",
      )
    ) {
      const device = this.devices.get(params[1] as string);
      if (!device || device.status !== "pending" || device.replaces_device_id == null) return [];
      if (this.accounts.get(device.account_id as string)?.deletingSinceUnixMs != null) return [];
      this.devices.set(device.id as string, { ...device, status: "resetting", resetting_since_unix_ms: params[0] });
      return [{ id: device.id }];
    }
    if (q === "UPDATE device SET status = 'pending', resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting'") {
      const device = this.devices.get(params[0] as string);
      if (device?.status === "resetting")
        this.devices.set(device.id as string, { ...device, status: "pending", resetting_since_unix_ms: null });
      return [];
    }
    if (
      q ===
      "UPDATE device SET status = 'pending', resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting' AND resetting_since_unix_ms < ?"
    ) {
      const device = this.devices.get(params[0] as string);
      if (device?.status === "resetting" && device.resetting_since_unix_ms != null && (device.resetting_since_unix_ms as number) < (params[1] as number))
        this.devices.set(device.id as string, { ...device, status: "pending", resetting_since_unix_ms: null });
      return [];
    }
    if (
      q.startsWith(
        "UPDATE device SET status = 'approved', replaces_device_id = NULL, resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting' RETURNING id",
      )
    ) {
      const device = this.devices.get(params[0] as string);
      if (device?.status !== "resetting") return [];
      this.devices.set(device.id as string, { ...device, status: "approved", replaces_device_id: null, resetting_since_unix_ms: null });
      return [{ id: device.id }];
    }
    if (q.startsWith("DELETE FROM retired_database WHERE account_id = ? AND name = ? AND retired_at_unix_ms = ?")) {
      const [accountId, name, at] = params;
      this.retired = this.retired.filter((r) => !(r.account_id === accountId && r.name === name && r.retired_at_unix_ms === at));
      return [];
    }
    if (
      q.startsWith(
        "SELECT id, name, platform FROM device WHERE account_id = ? AND status = 'approved' AND id != ? AND last_seen_unix_ms >= ?",
      )
    ) {
      const [accountId, exceptId, since] = params as [string, string, number];
      return [...this.devices.values()]
        .filter(
          (d) =>
            d.account_id === accountId &&
            d.status === "approved" &&
            d.id !== exceptId &&
            d.last_seen_unix_ms != null &&
            (d.last_seen_unix_ms as number) >= since,
        )
        .map((d) => ({ id: d.id, name: d.name, platform: d.platform }));
    }
    if (
      q ===
      "INSERT INTO waitlist (account_id, wants, platform, created_at_unix_ms) SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL) RETURNING account_id"
    ) {
      const [account_id, wants, platform, created_at_unix_ms] = params;
      const a = this.accounts.get(account_id as string);
      if (!a || a.deletingSinceUnixMs != null) return [];
      this.waitlist.push({ account_id, wants, platform, created_at_unix_ms });
      return [{ account_id }];
    }
    if (q === "UPDATE account SET stripe_customer_id = ? WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id") {
      const [customerId, id] = params as [string, string];
      const a = this.accounts.get(id);
      if (!a || a.deletingSinceUnixMs != null) return [];
      this.accounts.set(id, { ...a, stripeCustomerId: customerId });
      return [{ id }];
    }
    const applyPlanSet = "UPDATE account SET plan = ?, plan_until_unix_ms = ?, plan_interval = ?, plan_renews = ?, stripe_subscription_id = ?";
    if (q === `${applyPlanSet}, warned_at_unix_ms = NULL WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id`) {
      const [plan, planUntilUnixMs, planInterval, renews, stripeSubscriptionId, id] = params as ["cloud", number, "month" | "year" | null, number, string, string];
      const a = this.accounts.get(id);
      if (!a || a.deletingSinceUnixMs != null) return [];
      this.accounts.set(id, { ...a, plan, planUntilUnixMs, planInterval, planRenews: renews !== 0, stripeSubscriptionId, warnedAtUnixMs: null });
      return [{ id }];
    }
    if (q === `${applyPlanSet} WHERE id = ? AND stripe_subscription_id = ? AND deleting_since_unix_ms IS NULL RETURNING id`) {
      const [plan, planUntilUnixMs, planInterval, renews, stripeSubscriptionId, id, current] = params as ["free", number, null, number, string, string, string];
      const a = this.accounts.get(id);
      if (!a || a.deletingSinceUnixMs != null || a.stripeSubscriptionId !== current) return [];
      this.accounts.set(id, { ...a, plan, planUntilUnixMs, planInterval, planRenews: renews !== 0, stripeSubscriptionId });
      return [{ id }];
    }
    if (q === "SELECT 1 FROM stripe_event WHERE id = ?") {
      return this.stripeEvents.has(params[0] as string) ? [{ 1: 1 }] : [];
    }
    if (q === "INSERT OR IGNORE INTO stripe_event (id, received_at_unix_ms) VALUES (?, ?)") {
      this.stripeEvents.add(params[0] as string);
      return [];
    }
    // ---- account deletion and the inactivity cron ----
    const resetting = (accountId: string) =>
      [...this.devices.values()].some((d) => d.account_id === accountId && d.status === "resetting");
    if (q === "SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE id = ?") {
      const a = this.accounts.get(params[0] as string);
      return a ? [accountRow(a)] : [];
    }
    const currentDbNames = () => new Set([...this.accounts.values()].map((a) => a.dataDbName));
    if (q === "SELECT name FROM retired_database WHERE account_id = ? AND name NOT IN (SELECT data_db_name FROM account)") {
      const live = currentDbNames();
      return this.retired.filter((r) => r.account_id === params[0] && !live.has(r.name as string)).map((r) => ({ name: r.name }));
    }
    if (q === "SELECT name FROM retired_database WHERE retired_at_unix_ms < ? AND name NOT IN (SELECT data_db_name FROM account)") {
      const live = currentDbNames();
      return this.retired
        .filter((r) => (r.retired_at_unix_ms as number) < (params[0] as number) && !live.has(r.name as string))
        .map((r) => ({ name: r.name }));
    }
    if (q === "DELETE FROM retired_database WHERE name = ?") {
      this.retired = this.retired.filter((r) => r.name !== params[0]);
      return [];
    }
    if (q === "DELETE FROM retired_database WHERE account_id = ?") {
      this.retired = this.retired.filter((r) => r.account_id !== params[0]);
      return [];
    }
    if (q === "DELETE FROM account WHERE id = ?") {
      this.accounts.delete(params[0] as string);
      return [];
    }
    if (q === "DELETE FROM device WHERE account_id = ?") {
      for (const [id, d] of this.devices) if (d.account_id === params[0]) this.devices.delete(id);
      return [];
    }
    if (q === "DELETE FROM waitlist WHERE account_id = ?") {
      this.waitlist = this.waitlist.filter((w) => w.account_id !== params[0]);
      return [];
    }
    if (q === "UPDATE account SET last_seen_unix_ms = ? WHERE last_seen_unix_ms IS NULL") {
      for (const a of this.accounts.values()) if (a.lastSeenUnixMs == null) a.lastSeenUnixMs = params[0] as number;
      return [];
    }
    if (q === "SELECT id FROM device WHERE status = 'resetting' AND resetting_since_unix_ms < ?") {
      return [...this.devices.values()]
        .filter((d) => d.status === "resetting" && d.resetting_since_unix_ms != null && (d.resetting_since_unix_ms as number) < (params[0] as number))
        .map((d) => ({ id: d.id }));
    }
    if (q === "SELECT id, email, reset_notice_pending_unix_ms FROM account WHERE reset_notice_pending_unix_ms IS NOT NULL") {
      return [...this.accounts.values()]
        .filter((a) => a.resetNoticePendingUnixMs != null)
        .map((a) => ({ id: a.id, email: a.email, reset_notice_pending_unix_ms: a.resetNoticePendingUnixMs }));
    }
    if (q === "UPDATE account SET reset_notice_pending_unix_ms = NULL WHERE id = ? AND reset_notice_pending_unix_ms = ?") {
      const a = this.accounts.get(params[0] as string);
      if (a && a.resetNoticePendingUnixMs === params[1]) a.resetNoticePendingUnixMs = null;
      return [];
    }
    if (
      q ===
      "SELECT id, email FROM account WHERE warned_at_unix_ms IS NULL AND last_seen_unix_ms < ? AND (plan_until_unix_ms IS NULL OR plan_until_unix_ms < ?) AND ai_credit_micros <= 0"
    ) {
      return [...this.accounts.values()]
        .filter(
          (a) =>
            a.warnedAtUnixMs == null &&
            a.lastSeenUnixMs != null &&
            a.lastSeenUnixMs < (params[0] as number) &&
            (a.planUntilUnixMs == null || a.planUntilUnixMs < (params[1] as number)) &&
            (a.aiCreditMicros ?? 0) <= 0,
        )
        .map((a) => ({ id: a.id, email: a.email }));
    }
    if (
      q ===
      "UPDATE account SET warned_at_unix_ms = ? WHERE id = ? AND warned_at_unix_ms IS NULL AND last_seen_unix_ms < ? AND (plan_until_unix_ms IS NULL OR plan_until_unix_ms < ?) AND ai_credit_micros <= 0"
    ) {
      const [at, id, before] = params as [number, string, number];
      const a = this.accounts.get(id);
      if (a && a.warnedAtUnixMs == null && a.lastSeenUnixMs != null && a.lastSeenUnixMs < before && (a.planUntilUnixMs == null || a.planUntilUnixMs < (params[3] as number)) && (a.aiCreditMicros ?? 0) <= 0) a.warnedAtUnixMs = at;
      return [];
    }
    if (
      q ===
      "SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE warned_at_unix_ms < ? AND last_seen_unix_ms < warned_at_unix_ms AND (plan_until_unix_ms IS NULL OR plan_until_unix_ms < warned_at_unix_ms) AND ai_credit_micros <= 0"
    ) {
      return [...this.accounts.values()]
        .filter(
          (a) =>
            a.warnedAtUnixMs != null &&
            a.warnedAtUnixMs < (params[0] as number) &&
            a.lastSeenUnixMs != null &&
            a.lastSeenUnixMs < a.warnedAtUnixMs &&
            (a.planUntilUnixMs == null || a.planUntilUnixMs < a.warnedAtUnixMs) &&
            (a.aiCreditMicros ?? 0) <= 0,
        )
        .map(accountRow);
    }
    if (
      q ===
      "SELECT id, status, replaces_device_id, enrolled_at_unix_ms FROM device WHERE status IN ('pending', 'resetting') AND replaces_device_id IS NOT NULL"
    ) {
      return [...this.devices.values()].filter(
        (d) => (d.status === "pending" || d.status === "resetting") && d.replaces_device_id != null,
      );
    }
    if (
      q ===
      "UPDATE account SET deleting_since_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL AND warned_at_unix_ms < ? AND last_seen_unix_ms < warned_at_unix_ms AND (plan_until_unix_ms IS NULL OR plan_until_unix_ms < warned_at_unix_ms) AND ai_credit_micros <= 0 AND NOT EXISTS (SELECT 1 FROM device WHERE device.account_id = account.id AND device.status = 'resetting') RETURNING id"
    ) {
      const [at, id, warnedBefore] = params as [number, string, number];
      const a = this.accounts.get(id);
      if (
        !a ||
        a.deletingSinceUnixMs != null ||
        a.warnedAtUnixMs == null ||
        !(a.warnedAtUnixMs < warnedBefore) ||
        a.lastSeenUnixMs == null ||
        !(a.lastSeenUnixMs < a.warnedAtUnixMs) ||
        (a.planUntilUnixMs != null && !(a.planUntilUnixMs < a.warnedAtUnixMs)) ||
        (a.aiCreditMicros ?? 0) > 0
      )
        return [];
      if (resetting(id)) return [];
      a.deletingSinceUnixMs = at;
      return [{ id }];
    }
    if (q === "UPDATE account SET deleting_since_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL AND NOT EXISTS (SELECT 1 FROM device WHERE device.account_id = account.id AND device.status = 'resetting') RETURNING id") {
      const [at, id] = params as [number, string];
      const a = this.accounts.get(id);
      if (!a || a.deletingSinceUnixMs != null || resetting(id)) return [];
      a.deletingSinceUnixMs = at;
      return [{ id }];
    }
    if (q === "SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE deleting_since_unix_ms < ?") {
      return [...this.accounts.values()]
        .filter((a) => a.deletingSinceUnixMs != null && a.deletingSinceUnixMs < (params[0] as number))
        .map(accountRow);
    }
    if (q === "UPDATE account SET plan = 'free', stripe_subscription_id = NULL WHERE id = ?") {
      const a = this.accounts.get(params[0] as string);
      if (a) this.accounts.set(a.id, { ...a, plan: "free", stripeSubscriptionId: null, planRenews: true });
      return [];
    }
    if (q === "SELECT 1 FROM account WHERE data_db_name = ?") {
      return [...this.accounts.values()].filter((a) => a.dataDbName === params[0]).map(() => ({ 1: 1 }));
    }
    // ---- Whimbrel AI credit (aiCredit.ts) ----
    const holdOf = (id: unknown) => this.aiHolds.get(id as string);
    const pushLedger = (r: SqlRow) => {
      if (this.aiLedger.some((l) => l.id === r.id)) throw new Error("UNIQUE constraint failed: ai_ledger.id");
      if (!["purchase", "usage", "refund"].includes(r.kind as string)) throw new Error("CHECK constraint failed: kind");
      this.aiLedger.push(r);
    };
    const ledgerCols = "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms)";
    if (q === "SELECT ai_credit_micros FROM account WHERE id = ?") {
      const a = this.accounts.get(params[0] as string);
      return a ? [{ ai_credit_micros: a.aiCreditMicros ?? 0 }] : [];
    }
    if (
      q ===
      "UPDATE account SET ai_credit_micros = ai_credit_micros - ?, ai_open_holds = ai_open_holds + 1 WHERE id = ? AND ai_credit_micros >= ? AND ai_open_holds < 4 RETURNING ai_credit_micros, ai_open_holds"
    ) {
      const [micros, id, atLeast] = params as [number, string, number];
      const a = this.accounts.get(id);
      this.lastChanges = 0;
      if (!a || !((a.aiCreditMicros ?? 0) >= atLeast) || !((a.aiOpenHolds ?? 0) < 4)) return [];
      a.aiCreditMicros = (a.aiCreditMicros ?? 0) - micros;
      a.aiOpenHolds = (a.aiOpenHolds ?? 0) + 1;
      this.lastChanges = 1;
      return [{ ai_credit_micros: a.aiCreditMicros, ai_open_holds: a.aiOpenHolds }];
    }
    if (q === "INSERT INTO ai_hold (id, account_id, micros, created_at_unix_ms) SELECT ?, ?, ?, ? WHERE changes() = 1 RETURNING id") {
      const [id, account_id, micros, created_at_unix_ms] = params;
      if (this.lastChanges !== 1) return [];
      if (this.aiHolds.has(id as string)) throw new Error("UNIQUE constraint failed: ai_hold.id");
      this.aiHolds.set(id as string, { id, account_id, micros, created_at_unix_ms });
      return [{ id }];
    }
    if (
      q ===
      "UPDATE account SET ai_credit_micros = ai_credit_micros + (SELECT micros FROM ai_hold WHERE id = ?) - MIN(COALESCE(?, (SELECT micros FROM ai_hold WHERE id = ?)), ai_credit_micros + (SELECT micros FROM ai_hold WHERE id = ?)), ai_open_holds = MAX(ai_open_holds - 1, 0) WHERE id = (SELECT account_id FROM ai_hold WHERE id = ?)"
    ) {
      const h = holdOf(params[0]);
      const a = h && this.accounts.get(h.account_id as string);
      if (!h || !a) return [];
      const held = h.micros as number;
      const balance = a.aiCreditMicros ?? 0;
      const cost = (params[1] as number | null) ?? held;
      a.aiCreditMicros = balance + held - Math.min(cost, balance + held);
      a.aiOpenHolds = Math.max((a.aiOpenHolds ?? 0) - 1, 0);
      return [];
    }
    if (q === `${ledgerCols} SELECT ?, account_id, 'usage', -COALESCE(?, micros), ?, ?, ?, NULL, ? FROM ai_hold WHERE id = ?`) {
      const [id, cost, model, input_tokens, output_tokens, created_at_unix_ms, holdId] = params;
      const h = holdOf(holdId);
      if (!h) return [];
      const micros = -((cost as number | null) ?? (h.micros as number));
      pushLedger({ id, account_id: h.account_id, kind: "usage", micros, model, input_tokens, output_tokens, stripe_ref: null, created_at_unix_ms, shortfall_micros: null });
      return [];
    }
    if (q === "DELETE FROM ai_hold WHERE id = ?") {
      this.aiHolds.delete(params[0] as string);
      return [];
    }
    if (q === `${ledgerCols} SELECT ?, id, 'purchase', ?, NULL, NULL, NULL, ?, ? FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL ON CONFLICT(stripe_ref) DO NOTHING RETURNING id`) {
      const [id, micros, stripe_ref, created_at_unix_ms, account_id] = params;
      const a = this.accounts.get(account_id as string);
      if (!a || a.deletingSinceUnixMs != null) return [];
      if (this.aiLedger.some((l) => l.stripe_ref === stripe_ref)) return [];
      pushLedger({ id, account_id, kind: "purchase", micros, model: null, input_tokens: null, output_tokens: null, stripe_ref, created_at_unix_ms, shortfall_micros: null });
      return [{ id }];
    }
    if (q === "UPDATE account SET ai_credit_micros = ai_credit_micros + ? WHERE id = ? AND EXISTS (SELECT 1 FROM ai_ledger WHERE id = ?)") {
      const [plus, id, ledgerId] = params as [number, string, string];
      const a = this.accounts.get(id);
      if (a && this.aiLedger.some((l) => l.id === ledgerId)) a.aiCreditMicros = (a.aiCreditMicros ?? 0) + plus;
      return [];
    }
    if (
      q ===
      "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms, shortfall_micros) SELECT ?, id, 'refund', -MIN(?, ai_credit_micros), NULL, NULL, NULL, ?, ?, ? - MIN(?, ai_credit_micros) FROM account WHERE id = ? ON CONFLICT(stripe_ref) DO NOTHING RETURNING micros"
    ) {
      const [id, refund, stripe_ref, created_at_unix_ms, , , accountId] = params;
      const a = this.accounts.get(accountId as string);
      if (!a || this.aiLedger.some((l) => l.stripe_ref === stripe_ref)) return [];
      const removed = Math.min(refund as number, a.aiCreditMicros ?? 0);
      const micros = -removed;
      pushLedger({
        id,
        account_id: accountId,
        kind: "refund",
        micros,
        model: null,
        input_tokens: null,
        output_tokens: null,
        stripe_ref,
        created_at_unix_ms,
        shortfall_micros: (refund as number) - removed,
      });
      return [{ micros }];
    }
    if (
      q ===
      "UPDATE account SET ai_credit_micros = ai_credit_micros + (SELECT micros FROM ai_ledger WHERE id = ?) WHERE id = ? AND EXISTS (SELECT 1 FROM ai_ledger WHERE id = ?)"
    ) {
      const row = this.aiLedger.find((l) => l.id === params[0]);
      const a = this.accounts.get(params[1] as string);
      if (a && row) a.aiCreditMicros = (a.aiCreditMicros ?? 0) + (row.micros as number);
      return [];
    }
    if (q === "SELECT micros FROM ai_ledger WHERE account_id = ? AND kind = 'purchase' ORDER BY created_at_unix_ms DESC, rowid DESC LIMIT 1") {
      // Insertion order stands in for rowid.
      const rows = this.aiLedger.filter((l) => l.account_id === params[0] && l.kind === "purchase");
      let newest: SqlRow | undefined;
      for (const r of rows) if (!newest || (r.created_at_unix_ms as number) >= (newest.created_at_unix_ms as number)) newest = r;
      return newest ? [{ micros: newest.micros }] : [];
    }
    if (q === "SELECT account_id, micros FROM ai_ledger WHERE stripe_ref = ? AND kind = 'purchase'") {
      return this.aiLedger.filter((l) => l.stripe_ref === params[0] && l.kind === "purchase").map((l) => ({ account_id: l.account_id, micros: l.micros }));
    }
    if (q === "SELECT id FROM ai_hold WHERE created_at_unix_ms < ?") {
      return [...this.aiHolds.values()]
        .filter((h) => (h.created_at_unix_ms as number) < (params[0] as number))
        .map((h) => ({ id: h.id }));
    }
    if (q === "DELETE FROM ai_hold WHERE account_id = ?") {
      for (const [id, h] of this.aiHolds) if (h.account_id === params[0]) this.aiHolds.delete(id);
      return [];
    }
    if (q === "DELETE FROM ai_ledger WHERE account_id = ?") {
      this.aiLedger = this.aiLedger.filter((l) => l.account_id !== params[0]);
      return [];
    }
    throw new Error(`FakeSqlExecutor: unrecognized query: ${q}`);
  }
}
