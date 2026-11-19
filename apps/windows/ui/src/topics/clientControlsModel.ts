// Client controls on a seed topic's page (X6, spec docs/superpowers/specs/2026-11-12-client-controls-design.md
// sections 4-7), the Mac's ClientControlsModel.swift ported: delete with a review and a receipt, keep for N
// months, export, exclude from hosted models. The models hold no store and no React: every core call goes
// through `ClientControlsBridge`, so the tests run on a fake. The words are the Mac's, with this PC's facts
// (Windows backups, no Keychain, no Finder).

import {
  clientCancelWait,
  clientControlGet,
  clientControlUpdate,
  clientDelete,
  clientDueItems,
  clientExpire,
  clientExport,
  clientInventory,
  clientJobsRunning,
  clientPendingDeviceIds,
  clientPendingDevices,
  clientResummaryOffer,
  clientRetireDevice,
  clientRetentionDecide,
  clientReviewFlag,
  clientReviewFlagDismiss,
  meetingRegenerate,
} from "../api";
import type {
  ClientControl,
  ClientControlChange,
  ClientControlUpdate,
  ClientDeleteReceipt,
  ClientDeleteStage,
  ClientDue,
  ClientExpiryReceipt,
  ClientExportDone,
  ClientInventory,
  ClientItem,
  ClientItemKind,
  ClientItemRef,
  ClientKindCount,
  ClientLeftForYou,
  ClientOpErrorKind,
  ClientPendingDevices,
  ClientResult,
  ClientResummaryOffer,
  ClientRetentionDecision,
  ClientReviewFlag,
} from "../clientControls";
import { isCommandError } from "../errors";
import { t } from "../l10n";
import { dayLabel } from "./topicPageModel";

// MARK: - Seam

/** The core calls the client controls make. Every call resolves (a shell failure is `failed`), never rejects. */
export interface ClientControlsBridge {
  inventory(key: string): Promise<ClientResult<ClientInventory>>;
  /**
   * Waits for running background work as long as it takes (it ends early with `cancelled` after
   * `cancelWait(token)`), then deletes. `token` is new for each call. `reviewedLinked`: the linked items the user
   * saw; core answers `selection_changed` if the linked set differs once the wait is over.
   */
  delete(
    key: string,
    includePossible: ClientItemRef[],
    deleteExports: boolean,
    reviewedLinked: ClientItemRef[],
    token: number,
  ): Promise<ClientResult<ClientDeleteReceipt>>;
  /** Background ticks still running, for "Waiting for background work to finish…". 0 when unreadable. */
  jobsRunning(): Promise<number>;
  /** False: too late, the delete has started (or the shell could not be reached). */
  cancelWait(token: number): Promise<boolean>;
  control(key: string): Promise<ClientResult<ClientControl>>;
  /** Changes one setting on the stored value, read and written in one job on the store. */
  updateControl(key: string, change: ClientControlChange): Promise<ClientResult<ClientControlUpdate>>;
  resummaryOffer(key: string): Promise<ClientResult<ClientResummaryOffer>>;
  /** Summarises these meetings again, one at a time; core keeps an excluded client's meeting on this PC. Resolves
   *  with how many were not made again. */
  resummarize(meetings: number[]): Promise<number>;
  reviewFlag(): Promise<ClientResult<ClientReviewFlag | null>>;
  dismissReviewFlag(): Promise<ClientResult<null>>;
  dueItems(): Promise<ClientResult<ClientDue[]>>;
  dueItemsFor(key: string): Promise<ClientResult<ClientItem[]>>;
  decide(key: string, item: ClientItemRef, decision: ClientRetentionDecision): Promise<ClientResult<null>>;
  /** One budgeted expiry call; the caller repeats while `more`. */
  expire(key: string, items: ClientItemRef[]): Promise<ClientResult<ClientExpiryReceipt>>;
  /** Devices on the relay that hold back every client's settings from the others (`topic_rows_held_devices`);
   *  null when not measured yet or unreadable. */
  heldDevices(): Promise<number | null>;
  /** The shell asks for the folder; `data` is null when the user cancelled the pick. */
  export(key: string, includePossible: ClientItemRef[], includeMedia: boolean): Promise<ClientResult<ClientExportDone | null>>;
}

/** A re-summary the pipeline refused only because it was busy with another meeting is tried again this far apart, for
 *  at most this many tries (about ten minutes); any other refusal counts as not made at once. */
const RESUMMARY_BUSY_TRIES = 60;
const RESUMMARY_RETRY_MS = 10_000;

/** The pipeline's "busy with another meeting" refusal. The shell renders it from the same catalog, in the same
 *  language, so it is matched on that sentence (and on the English one). */
function pipelineBusy(e: unknown): boolean {
  if (!isCommandError(e) || !("message" in e)) return false;
  const key = "Whimbrel is still working on a meeting. Try again when it's done.";
  return e.message === key || e.message === t("Whimbrel is still working on a meeting. Try again when it's done.");
}

/** Regenerates each meeting in turn, keeping the user's edits; resolves with how many were not made again. */
export async function regenerateInTurn(
  meetings: number[],
  regenerate: (id: number) => Promise<unknown>,
  wait: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
): Promise<number> {
  let notMade = 0;
  for (const id of meetings) {
    let made = false;
    for (let attempt = 0; attempt < RESUMMARY_BUSY_TRIES; attempt++) {
      const busy = await regenerate(id).then(
        () => ((made = true), false),
        (e: unknown) => pipelineBusy(e),
      );
      if (made || !busy) break;
      await wait(RESUMMARY_RETRY_MS);
    }
    if (!made) notMade += 1;
  }
  return notMade;
}

/** Every item in a keep-for-N-months expiry call: core stops starting new items after this. */
export const EXPIRE_BUDGET_MS = 5_000;

/** A rejected invoke means the shell could not run the call (database not open, account key unreadable): it is
 *  `failed`, never "no sync" and never success. */
function settled<T>(p: Promise<ClientResult<T>>): Promise<ClientResult<T>> {
  return p.catch(() => ({ ok: false, error: { kind: "failed" } }) as ClientResult<T>);
}

/** The real bridge over the Tauri commands. */
export const tauriClientControls: ClientControlsBridge = {
  inventory: (key) => settled(clientInventory(key)),
  delete: (key, includePossible, deleteExports, reviewedLinked, token) =>
    settled(clientDelete({ key, includePossible, deleteExports, reviewedLinked, token })),
  jobsRunning: () => clientJobsRunning().catch(() => 0),
  cancelWait: (token) => clientCancelWait(token).catch(() => false),
  control: (key) => settled(clientControlGet(key)),
  updateControl: (key, change) => settled(clientControlUpdate(key, change)),
  resummaryOffer: (key) => settled(clientResummaryOffer(key)),
  // Each meeting through the normal summary command, keeping the user's edits.
  resummarize: (meetings) => regenerateInTurn(meetings, (id) => meetingRegenerate(id, true)),
  reviewFlag: () => settled(clientReviewFlag()),
  dismissReviewFlag: () => settled(clientReviewFlagDismiss()),
  dueItems: () => settled(clientDueItems()),
  dueItemsFor: (key) => settled(clientDueItems(key)),
  decide: (key, item, decision) => settled(clientRetentionDecide(key, item, decision)),
  expire: (key, items) => settled(clientExpire(key, items, EXPIRE_BUDGET_MS)),
  heldDevices: () => clientPendingDevices().then((d) => d?.topic_rows_held_devices ?? null, () => null),
  export: (key, includePossible, includeMedia) => settled(clientExport(key, includePossible, includeMedia)),
};

/** A number for each delete, unique across a reload of the page (the shell may still hold an older wait), below
 *  2^53 so it passes through JSON exactly. */
let lastDeleteToken = 0;
function nextDeleteToken(): number {
  lastDeleteToken = Math.max(lastDeleteToken + 1, Date.now() * 1000 + Math.floor(Math.random() * 1000));
  return lastDeleteToken;
}

const refOf = (item: ClientItemRef): ClientItemRef => ({
  kind: item.kind,
  id: item.id,
});
const refKey = (item: ClientItemRef) => `${item.kind}\u0000${item.id}`;

/** A model the view subscribes to: `version` changes on every change, for useSyncExternalStore. */
export class Observable {
  private listeners = new Set<() => void>();
  version = 0;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getVersion = () => this.version;
  protected changed() {
    this.version += 1;
    this.listeners.forEach((l) => l());
  }
}

// MARK: - Settings: exclude from hosted models, keep for N months

/** What turning exclusion on shows: the already-left sentence and the meetings a hosted model summarised. */
export interface ExclusionNotice {
  alreadyLeft: string;
  hostedSummaries: number[];
  resummaryStarted: boolean;
  /** After the re-summary finished: how many were not made again (null while it runs or before it starts). */
  resummaryNotMade: number | null;
}

/** The keep periods offered, in months; null is Off. Spec 5 offers 12 by default. */
export const KEEP_CHOICES: (number | null)[] = [null, 3, 6, 12, 24, 36];

export class ClientSettingsModel extends Observable {
  /** The last value core saved (or read). */
  saved: ClientControl | null = null;
  /** False when core says this topic is not a live client: the page shows no Client section. */
  isClient = true;
  errorText: string | null = null;
  exclusionNotice: ExclusionNotice | null = null;
  isSaving = false;
  /** Devices that keep this setting from reaching the others (`heldDevicesLine`); null: not known. */
  heldDevices: number | null = null;
  private writes: Promise<void> = Promise.resolve();

  constructor(
    readonly key: string,
    private readonly bridge: ClientControlsBridge,
  ) {
    super();
  }

  get excludeHosted(): boolean {
    return this.saved?.exclude_hosted ?? false;
  }

  /** null: Off (core's null or 0). */
  get keepMonths(): number | null {
    const m = this.saved?.keep_months;
    return m != null && m > 0 ? m : null;
  }

  /** The picker's choices, with a saved period that is not one of them (set on another device) kept. */
  get keepOptions(): (number | null)[] {
    const current = this.keepMonths;
    if (current == null || KEEP_CHOICES.includes(current)) return KEEP_CHOICES;
    return [...KEEP_CHOICES, current].sort((a, b) => (a ?? 0) - (b ?? 0));
  }

  async load(): Promise<void> {
    const [r, held] = await Promise.all([this.bridge.control(this.key), this.bridge.heldDevices().catch(() => null)]);
    this.heldDevices = held;
    if (r.ok) {
      this.saved = r.data;
      this.isClient = true;
      this.errorText = null;
    } else if (r.error.kind === "not_found") {
      this.isClient = false;
    } else {
      this.errorText = clientMessage(r.error.kind);
    }
    this.changed();
  }

  setExcludeHosted(on: boolean): Promise<void> {
    return this.write({ exclude_hosted: on });
  }

  setKeepMonths(months: number | null): Promise<void> {
    return this.write({ keep_months: months });
  }

  /** Summarises the listed meetings again (a local model, or not made), then says how many were not made. */
  async resummarize(): Promise<void> {
    const n = this.exclusionNotice;
    if (!n || n.hostedSummaries.length === 0 || n.resummaryStarted) return;
    this.exclusionNotice = { ...n, resummaryStarted: true, resummaryNotMade: null };
    this.changed();
    const notMade = await this.bridge.resummarize(n.hostedSummaries);
    if (this.exclusionNotice) this.exclusionNotice = { ...this.exclusionNotice, resummaryNotMade: notMade };
    this.changed();
  }

  /** Writes one at a time; core applies each change to the value stored at that moment, in one job on the store. */
  private write(change: ClientControlChange): Promise<void> {
    const next = this.writes.then(() => this.apply(change));
    this.writes = next;
    return next;
  }

  private async apply(change: ClientControlChange): Promise<void> {
    if (this.saved == null) return;
    this.isSaving = true;
    this.changed();
    try {
      // Applied to the stored value, not `saved`: another device may have changed it since the page loaded.
      const r = await this.bridge.updateControl(this.key, change);
      if (!r.ok) return this.fail(r.error.kind);
      const { before, after } = r.data;
      this.saved = after;
      this.errorText = null;
      if (after.exclude_hosted && !before.exclude_hosted) {
        // Core's offer and sentence; the sentence shows even if the offer could not be read.
        const offer = await this.bridge.resummaryOffer(this.key);
        const note = offer.ok && offer.data.already_left_note ? offer.data.already_left_note : alreadyLeftSentence();
        this.exclusionNotice = {
          alreadyLeft: note,
          hostedSummaries: offer.ok ? offer.data.meetings : [],
          resummaryStarted: false,
          resummaryNotMade: null,
        };
      } else if (!after.exclude_hosted) {
        this.exclusionNotice = null;
      }
    } finally {
      this.isSaving = false;
      this.changed();
    }
  }

  private fail(kind: ClientOpErrorKind) {
    if (kind === "not_found") this.isClient = false;
    else this.errorText = clientMessage(kind);
  }
}

// MARK: - Delete and export review

export type ReviewMode = "delete" | "export";

export interface ExportOutcome {
  folder: string;
  media: ClientExportDone["media"] | null;
  partial: boolean;
}

export type ReviewPhase =
  | { kind: "loading" }
  | { kind: "review" }
  /** The delete or export is running: the button is off and a spinner shows. */
  | { kind: "running" }
  /** Confirmed, but background work is still running: nothing is deleted yet and Cancel is offered (PR 875). */
  | { kind: "waiting" }
  | { kind: "receipt"; receipt: ClientDeleteReceipt }
  | { kind: "exported"; outcome: ExportOutcome }
  /** The client is not there any more (deleted elsewhere, or merged). */
  | { kind: "gone" }
  /** This PC has no device identity: no delete is offered, and retrying cannot help. */
  | { kind: "noIdentity" }
  | { kind: "failed"; error: ClientOpErrorKind };

/** Why the review screen shows again after a Delete press. */
export type ReviewNotice = "selection_changed" | "busy";

export class ClientReviewModel extends Observable {
  phase: ReviewPhase = { kind: "loading" };
  inventory: ClientInventory | null = null;
  /** Possible matches the user ticked. Empty to start: nothing that only names the client is deleted unless ticked. */
  private ticked = new Map<string, ClientItemRef>();
  confirmText = "";
  /** Also delete the export folders Whimbrel recorded for this client. Off unless the user ticks it. */
  deleteExports = false;
  /** Export only: audio and slide images too (off by default, because of size). */
  includeMedia = false;
  notice: ReviewNotice | null = null;
  /** A partial delete whose client record is already gone: nothing is left to run again. */
  retryGone = false;
  private lastReceipt: ClientDeleteReceipt | null = null;
  /** The user pressed Delete with the name typed. A retry deletes only after that; otherwise it reloads the review. */
  private confirmed = false;

  /**
   * `name` is the client's name, typed to confirm and never put in a receipt. `hasDeviceIdentity` is false when
   * the shell already knows this PC has none; otherwise core's `no_device_id` leads to the same state.
   */
  constructor(
    readonly key: string,
    readonly name: string,
    readonly mode: ReviewMode,
    private readonly bridge: ClientControlsBridge,
    readonly hasDeviceIdentity = true,
  ) {
    super();
  }

  get isRunning(): boolean {
    return this.phase.kind === "running" || this.phase.kind === "waiting";
  }

  /** Exact, as typed: a mis-tap or a near miss cannot delete a client. */
  get nameMatches(): boolean {
    return this.name.length > 0 && this.confirmText === this.name;
  }

  get canDelete(): boolean {
    return this.mode === "delete" && this.hasDeviceIdentity && this.phase.kind === "review" && this.inventory != null && this.nameMatches;
  }

  get canExport(): boolean {
    return this.mode === "export" && this.phase.kind === "review" && this.inventory != null;
  }

  get tickedRefs(): ClientItemRef[] {
    return [...this.ticked.values()];
  }

  isTicked(item: ClientItemRef): boolean {
    return this.ticked.has(refKey(item));
  }

  /** A group's box: ticked when every item in it is. */
  isGroupTicked(items: ClientItemRef[]): boolean {
    return items.length > 0 && items.every((i) => this.isTicked(i));
  }

  tickedCount(items: ClientItemRef[]): number {
    return items.filter((i) => this.isTicked(i)).length;
  }

  /** Ticks or unticks every item of a group; the delete still gets exactly the ticked refs. */
  setGroupTicked(items: ClientItem[], on: boolean): void {
    for (const i of items) this.setTicked(i, on);
  }

  setTicked(item: ClientItem, on: boolean): void {
    if (this.phase.kind !== "review" || !this.inventory?.possible.some((p) => refKey(p) === refKey(item))) return;
    if (on) this.ticked.set(refKey(item), refOf(item));
    else this.ticked.delete(refKey(item));
    this.changed();
  }

  setConfirmText(text: string): void {
    this.confirmText = text;
    this.changed();
  }

  setDeleteExports(on: boolean): void {
    this.deleteExports = on;
    this.changed();
  }

  setIncludeMedia(on: boolean): void {
    this.includeMedia = on;
    this.changed();
  }

  private set(phase: ReviewPhase) {
    this.phase = phase;
    this.changed();
  }

  private loaded = false;

  /** Runs once: a dialog mounted again (the topic list emptied after the delete) keeps its receipt. */
  async load(): Promise<void> {
    if (this.loaded) return;
    this.loaded = true;
    if (this.mode === "delete" && !this.hasDeviceIdentity) return this.set({ kind: "noIdentity" });
    this.set({ kind: "loading" });
    if (await this.refresh()) this.set({ kind: "review" });
  }

  /** Keeps only the ticks a fresh inventory still lists. */
  private keepListed(inv: ClientInventory) {
    this.inventory = inv;
    const listed = new Set(inv.possible.map(refKey));
    for (const k of [...this.ticked.keys()]) if (!listed.has(k)) this.ticked.delete(k);
  }

  /** Reads the inventory again. False when it could not (phase set). */
  private async refresh(): Promise<boolean> {
    const r = await this.bridge.inventory(this.key);
    if (r.ok) {
      this.keepListed(r.data);
      return true;
    }
    this.set(r.error.kind === "not_found" ? { kind: "gone" } : { kind: "failed", error: r.error.kind });
    return false;
  }

  async delete(): Promise<void> {
    if (!this.canDelete) return;
    this.confirmed = true;
    await this.runDelete();
  }

  /** "Try again" after a partial receipt, a failure or `busy`: the selection is rebuilt from a fresh inventory first
   *  (core answers `selection_changed` to a tick it no longer lists). */
  async retry(): Promise<void> {
    if (this.mode !== "delete" || !this.hasDeviceIdentity) return;
    const p = this.phase;
    if (p.kind === "receipt") {
      if (!receiptRetryHelps(p.receipt) || this.retryGone) return;
    } else if (p.kind === "failed") {
      if (!isRetryable(p.error)) return;
    } else if (p.kind === "review") {
      if (this.notice !== "busy") return;
    } else {
      return;
    }
    this.set({ kind: "running" });
    const r = await this.bridge.inventory(this.key);
    if (!r.ok) {
      if (r.error.kind === "not_found") {
        // The client record went in the partial run: what is left is listed under Left for you.
        if (this.lastReceipt) {
          this.retryGone = true;
          this.set({ kind: "receipt", receipt: this.lastReceipt });
        } else {
          this.set({ kind: "gone" });
        }
      } else {
        this.set({ kind: "failed", error: r.error.kind });
      }
      return;
    }
    this.keepListed(r.data);
    // Never a delete the user did not confirm (a failed first load also offers Try again).
    if (!this.confirmed || !this.nameMatches) return this.set({ kind: "review" });
    await this.runDelete();
  }

  /** An export folder was unreachable (its drive is gone): run again, leaving the export folders to the user. */
  retryWithoutDeletingExports(): Promise<void> {
    this.deleteExports = false;
    return this.retry();
  }

  /** The last receipt has an export folder Whimbrel could not reach. */
  get exportFolderUnreachable(): boolean {
    return this.phase.kind === "receipt" && this.phase.receipt.failed.includes("export_folder");
  }

  /** The token of the delete now under way, for Cancel. */
  private waitToken = 0;
  /** How often the screen looks for background work still running while a delete waits. */
  waitPollMs = 300;
  /** Cancel came after the delete had started: it finished anyway, and the receipt says so. */
  cancelTooLate = false;

  /** "Cancel" from the moment the delete is confirmed until it starts: nothing is deleted and the review shows
   *  again. Once it has started, core says it is too late and the receipt notes that. */
  async cancelWait(): Promise<void> {
    if (this.mode !== "delete" || !this.isRunning) return;
    if (!(await this.bridge.cancelWait(this.waitToken))) {
      this.cancelTooLate = true;
      this.changed();
    }
  }

  private async runDelete(): Promise<void> {
    this.notice = null;
    this.cancelTooLate = false;
    const picks = this.tickedRefs;
    // The linked items the user saw on this screen, not a fresh read.
    const seen = (this.inventory?.linked ?? []).map(refOf);
    this.waitToken = nextDeleteToken();
    const waitingOrRunning = async () => {
      const n = await this.bridge.jobsRunning();
      if (this.isRunning) this.set({ kind: n > 0 ? "waiting" : "running" });
    };
    this.set({ kind: "running" });
    void waitingOrRunning();
    const poll = setInterval(() => void waitingOrRunning(), this.waitPollMs);
    let r: ClientResult<ClientDeleteReceipt>;
    try {
      r = await this.bridge.delete(this.key, picks, this.deleteExports, seen, this.waitToken);
    } finally {
      clearInterval(poll);
    }
    if (r.ok) {
      this.lastReceipt = r.data;
      return this.set({ kind: "receipt", receipt: r.data });
    }
    switch (r.error.kind) {
      case "selection_changed":
        // Nothing was touched. Show the list as it is now and ask again: the user confirms the new list.
        this.confirmed = false;
        if (await this.refresh()) {
          this.notice = "selection_changed";
          this.set({ kind: "review" });
        }
        return;
      case "busy":
        this.notice = "busy";
        return this.set({ kind: "review" });
      case "cancelled":
        // Nothing was touched. The ticks and the typed name stay; Delete must be pressed again.
        this.confirmed = false;
        this.cancelTooLate = false;
        return this.set({ kind: "review" });
      case "no_device_id":
        return this.set({ kind: "noIdentity" });
      case "not_found":
        return this.set({ kind: "gone" });
      default:
        return this.set({ kind: "failed", error: r.error.kind });
    }
  }

  // MARK: Export

  /** The shell asks for the parent folder, writes the export into a new folder inside it, then copies the media
   *  with the store released. A cancelled pick goes back to the review. */
  async export(): Promise<void> {
    if (!this.canExport) return;
    this.set({ kind: "running" });
    const r = await this.bridge.export(this.key, this.tickedRefs, this.includeMedia);
    if (!r.ok) return this.set(r.error.kind === "not_found" ? { kind: "gone" } : { kind: "failed", error: r.error.kind });
    if (r.data == null) return this.set({ kind: "review" });
    const done = r.data;
    this.set({
      kind: "exported",
      outcome: {
        folder: done.folder,
        media: this.includeMedia ? done.media : null,
        partial: done.partial,
      },
    });
  }

  /** Back to the review after an export failure the user can fix (another folder). */
  backToReview(): void {
    if (this.phase.kind === "failed" && this.inventory != null) this.set({ kind: "review" });
  }
}

// MARK: - Keep for N months: the weekly review

export interface RetentionGroup {
  key: string;
  name: string;
  items: ClientItem[];
}

export type RetentionPhase =
  | { kind: "loading" }
  | { kind: "review" }
  | { kind: "running" }
  | { kind: "done"; summary: ExpirySummary }
  | { kind: "failed"; error: ClientOpErrorKind };

/** Snooze puts an item off for 30 days (spec 5). */
export const SNOOZE_DAYS = 30;
/** Rounds of a budgeted expiry before the screen stops and says so. */
export const MAX_ROUNDS = 50;

/** What a keep-for-N-months run did, summed over its rounds and clients. */
export interface ExpirySummary {
  counts: ClientKindCount[];
  notDue: ClientKindCount[];
  waitingToSync: ClientKindCount[];
  failed: ClientItemKind[];
  leftForYou: ClientLeftForYou[];
  /** It stopped after `MAX_ROUNDS`: run it again. */
  unfinished: boolean;
  stopped: ClientOpErrorKind | null;
}

export const emptySummary = (): ExpirySummary => ({
  counts: [],
  notDue: [],
  waitingToSync: [],
  failed: [],
  leftForYou: [],
  unfinished: false,
  stopped: null,
});

export function mergeCounts(a: ClientKindCount[], b: ClientKindCount[]): ClientKindCount[] {
  const out = a.map((c) => [...c] as ClientKindCount);
  for (const [kind, n] of b) {
    const i = out.findIndex((c) => c[0] === kind);
    if (i >= 0) out[i] = [kind, out[i][1] + n];
    else out.push([kind, n]);
  }
  return out;
}

export const summaryComplete = (s: ExpirySummary) => s.failed.length === 0 && s.stopped == null && !s.unfinished;

export class ClientRetentionReviewModel extends Observable {
  phase: RetentionPhase = { kind: "loading" };
  groups: RetentionGroup[] = [];
  /** Ticked items are deleted on confirm; Keep and Snooze take an item off the list. */
  private ticked = new Set<string>();
  errorText: string | null = null;

  /** `names`: client key to name, for the headings (a client not in it is shown without a name). */
  constructor(
    private readonly names: Record<string, string>,
    private readonly bridge: ClientControlsBridge,
    private readonly now: () => number = Date.now,
  ) {
    super();
  }

  private pick = (key: string, item: ClientItemRef) => `${key}\u0001${refKey(item)}`;

  get tickedCount(): number {
    return this.ticked.size;
  }

  private set(phase: RetentionPhase) {
    this.phase = phase;
    this.changed();
  }

  private loaded = false;

  /** Runs once, like the review's: a dialog mounted again keeps what it shows. */
  async load(): Promise<void> {
    if (this.loaded) return;
    this.loaded = true;
    this.set({ kind: "loading" });
    const r = await this.bridge.dueItems();
    if (!r.ok) return this.set({ kind: "failed", error: r.error.kind });
    this.groups = r.data
      .filter((d) => d.items.length > 0)
      .map((d) => ({
        key: d.key,
        name: this.names[d.key] ?? t("A client"),
        items: d.items,
      }));
    // Linked items past a period the user set: ticked to start; deleting still needs the explicit button.
    this.ticked = new Set(this.groups.flatMap((g) => g.items.map((i) => this.pick(g.key, i))));
    this.set({ kind: "review" });
  }

  isTicked(key: string, item: ClientItemRef): boolean {
    return this.ticked.has(this.pick(key, item));
  }

  setTicked(key: string, item: ClientItemRef, on: boolean): void {
    if (on) this.ticked.add(this.pick(key, item));
    else this.ticked.delete(this.pick(key, item));
    this.changed();
  }

  /** "Keep": never asked again for this item, on this PC. */
  keep(key: string, item: ClientItem): Promise<void> {
    return this.decide(key, item, { decision: "keep" });
  }

  snooze(key: string, item: ClientItem): Promise<void> {
    return this.decide(key, item, {
      decision: "snooze",
      until_ms: this.now() + SNOOZE_DAYS * 86_400_000,
    });
  }

  private async decide(key: string, item: ClientItem, decision: ClientRetentionDecision): Promise<void> {
    const r = await this.bridge.decide(key, refOf(item), decision);
    if (!r.ok) {
      this.errorText = clientMessage(r.error.kind);
      return this.changed();
    }
    this.errorText = null;
    this.ticked.delete(this.pick(key, item));
    this.groups = this.groups
      .map((g) => (g.key === key ? { ...g, items: g.items.filter((i) => refKey(i) !== refKey(item)) } : g))
      .filter((g) => g.items.length > 0);
    this.changed();
  }

  /** Deletes the ticked items on this PC, client by client, repeating each budgeted call while core says `more`
   *  (each round sends only the ticked items still due). */
  async deleteTicked(): Promise<void> {
    if (this.phase.kind !== "review" || this.ticked.size === 0) return;
    this.set({ kind: "running" });
    const summary = emptySummary();
    const stop = (e: ClientOpErrorKind) => {
      summary.stopped = e;
      this.set({ kind: "done", summary });
    };
    for (const group of this.groups) {
      const wanted = new Set(group.items.filter((i) => this.isTicked(group.key, i)).map(refKey));
      if (wanted.size === 0) continue;
      let items = group.items.filter((i) => wanted.has(refKey(i))).map(refOf);
      let rounds = 0;
      while (items.length > 0) {
        rounds += 1;
        const r = await this.bridge.expire(group.key, items);
        if (!r.ok) return stop(r.error.kind);
        const receipt = r.data;
        summary.counts = mergeCounts(summary.counts, receipt.counts);
        summary.notDue = mergeCounts(summary.notDue, receipt.not_due);
        for (const k of receipt.failed) if (!summary.failed.includes(k)) summary.failed.push(k);
        for (const l of receipt.left_for_you) {
          if (!summary.leftForYou.some((x) => JSON.stringify(x) === JSON.stringify(l))) summary.leftForYou.push(l);
        }
        if (!receipt.more) {
          summary.waitingToSync = mergeCounts(summary.waitingToSync, receipt.waiting_to_sync);
          break;
        }
        if (rounds >= MAX_ROUNDS) {
          summary.unfinished = true;
          break;
        }
        // Only the ones still due go again; done ones would count as "no longer due".
        const still = await this.bridge.dueItemsFor(group.key);
        // Never "deleted" while ticked items may remain.
        if (!still.ok) return stop(still.error.kind);
        items = still.data.filter((i) => wanted.has(refKey(i))).map(refOf);
      }
    }
    this.set({ kind: "done", summary });
  }

  /** "Later": the weekly prompt comes back in a week. */
  async later(): Promise<void> {
    await this.bridge.dismissReviewFlag();
  }
}

// MARK: - Settings → Sync: devices still to update

/** The core calls behind the Sync card's update line. */
export interface PendingDevicesBridge {
  /** Devices that still need to update before they apply client deletes; null when not known yet. */
  count(): Promise<number | null>;
  /** Those devices' sync ids (public keys); null when not known yet. */
  ids(): Promise<string[] | null>;
  /** "Remove this device": stops counting one of them, on every device after this PC's next sync round. */
  retire(id: string): Promise<ClientResult<ClientPendingDevices>>;
}

export const tauriPendingDevices: PendingDevicesBridge = {
  count: async () => (await clientPendingDevices())?.pending_devices ?? null,
  ids: async () => (await clientPendingDeviceIds())?.devices ?? null,
  retire: async (id) => {
    try {
      return await clientRetireDevice(id);
    } catch {
      return { ok: false, error: { kind: "failed" } };
    }
  },
};

/**
 * "N devices still need to update" under the sync status (spec 13.3), each such device named, with "Remove this
 * device…" for one that is retired and will never update. Removing one only stops counting it; it releases
 * nothing (client deletes are never held for a device).
 */
export class PendingDevicesModel extends Observable {
  count: number | null = null;
  /** The devices counted, by sync id. Empty when none or not known. */
  ids: string[] = [];
  /** The device the confirmation is open for. */
  confirming: string | null = null;
  errorText: string | null = null;
  isRemoving = false;

  constructor(private readonly bridge: PendingDevicesBridge) {
    super();
  }

  /** Null for none and for not known yet. */
  get line(): string | null {
    return pendingCountLine(this.count);
  }

  async load(): Promise<void> {
    try {
      this.count = await this.bridge.count();
      this.ids = (this.count ?? 0) > 0 ? ((await this.bridge.ids()) ?? []) : [];
    } catch {
      // The shell could not run the call: keep what is shown.
    }
    this.changed();
  }

  ask(id: string): void {
    if (!this.ids.includes(id) || this.isRemoving) return;
    this.errorText = null;
    this.confirming = id;
    this.changed();
  }

  cancel(): void {
    this.confirming = null;
    this.changed();
  }

  /** The user pressed Remove in the confirmation: removes the device it was opened for, and only that one. */
  async confirm(): Promise<void> {
    const id = this.confirming;
    if (id == null || !this.ids.includes(id) || this.isRemoving) return;
    this.confirming = null;
    this.isRemoving = true;
    this.changed();
    const r = await this.bridge.retire(id);
    this.isRemoving = false;
    if (r.ok) {
      this.count = r.data.pending_devices;
      this.ids = this.ids.filter((d) => d !== id);
      this.errorText = null;
    } else if (r.error.kind === "not_found") {
      // Updated since, or already removed: read the list again.
      this.errorText = deviceNoLongerListed();
      await this.load();
    } else {
      this.errorText = clientMessage(r.error.kind);
    }
    this.changed();
  }
}

/** "1 device still needs to update before it applies client deletes"; null for none and for not known yet. */
export function pendingCountLine(n: number | null): string | null {
  if (n == null || n <= 0) return null;
  if (n === 1) return t("1 device still needs to update before it applies client deletes.");
  return t("{n} devices still need to update before they apply client deletes.", { n });
}

export const removeDeviceLabel = () => t("Remove this device…");
export const removeDeviceConfirmLine = () =>
  t(
    "Only do this for a device you no longer use. Your devices stop counting it as needing an update. It keeps its access to sync: to end that too, revoke it under Devices on this account. If it is used again before it updates, it will not apply client deletes, and nothing here will say so.",
  );
export const deviceNoLongerListed = () => t("That device is no longer on the list. The list was read again.");

// MARK: - Words

/** Shown when exclusion is turned on, when core's offer could not be read (core's `ALREADY_LEFT_NOTE`). */
export const alreadyLeftSentence = () => t("Material that already left cannot be recalled.");

export function kindName(k: ClientItemKind): string {
  switch (k) {
    case "meeting":
      return t("Meetings");
    case "email":
      return t("Emails");
    case "thread":
      return t("Email threads");
    case "calendar_event":
      return t("Calendar events");
    case "capture":
      return t("Screen captures");
    case "episode":
      return t("Screen sessions");
    case "conversation":
      return t("Ask conversations");
    case "step":
      return t("Steps");
    case "vault_note":
      return t("Vault notes");
    case "export_folder":
      return t("Export folders");
  }
}

export function kindSingular(k: ClientItemKind): string {
  switch (k) {
    case "meeting":
      return t("Meeting");
    case "email":
      return t("Email");
    case "thread":
      return t("Email thread");
    case "calendar_event":
      return t("Calendar event");
    case "capture":
      return t("Screen capture");
    case "episode":
      return t("Screen session");
    case "conversation":
      return t("Ask conversation");
    case "step":
      return t("Step");
    case "vault_note":
      return t("Vault note");
    case "export_folder":
      return t("Export folder");
  }
}

/** "Meetings: 3", for each kind with a count. */
export function countLines(list: ClientKindCount[]): string[] {
  return list.filter(([, n]) => n > 0).map(([kind, n]) => t("{kind}: {n}", { kind: kindName(kind), n }));
}

/** "Email, Tue, Nov 4". */
export function itemLine(item: ClientItem, locale: string): string {
  return t("{kind}, {day}", {
    kind: kindSingular(item.kind),
    day: dayLabel(new Date(item.date_unix_ms), locale),
  });
}

/**
 * A review row's main line: the item's label (client text, for this screen only), or its kind and date when it
 * has none. Kept apart from the receipt lines, which must never hold client text.
 */
export function rowTitle(item: ClientItem, locale: string): string {
  const label = item.label?.trim();
  return label ? label : itemLine(item, locale);
}

/** The kind and date under a label; null when the row title already is the kind and date. */
export function rowDetail(item: ClientItem, locale: string): string | null {
  return item.label?.trim() ? itemLine(item, locale) : null;
}

/** Screen captures of one app (or, with no app, one window title) on one day, shown as one row (PR 875). */
export interface CaptureGroup {
  id: string;
  items: ClientItem[];
  /** The app id, else the window title of the first capture that has one, else null. */
  source: string | null;
}

/** A row of the review list: one item, or a group of captures. */
export type ReviewRow = { kind: "item"; item: ClientItem } | { kind: "group"; group: CaptureGroup };

/** The local calendar day of a time, as a key. */
const localDay = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
};

/**
 * Captures with the same app (or, with no app, the same window title) on the same day, two or more, fold into one
 * group at the place of the first of them; everything else stays a row. Order is kept. The Mac's
 * `ClientRowText.rows`; the app shows as its id, as the Library shows it.
 */
export function reviewRows(items: ClientItem[]): ReviewRow[] {
  const keyOf = (i: ClientItem): string | null => {
    if (i.kind !== "capture") return null;
    const app = i.app?.trim();
    const who = app || i.label?.trim();
    if (!who) return null;
    return `${app ? "a" : "t"}:${who}|${localDay(i.date_unix_ms)}`;
  };
  const members = new Map<string, ClientItem[]>();
  for (const i of items) {
    const k = keyOf(i);
    if (k) members.set(k, [...(members.get(k) ?? []), i]);
  }
  const out: ReviewRow[] = [];
  const placed = new Set<string>();
  for (const i of items) {
    const k = keyOf(i);
    const group = k ? members.get(k) : undefined;
    if (!k || !group || group.length < 2) {
      out.push({ kind: "item", item: i });
      continue;
    }
    if (placed.has(k)) continue;
    placed.add(k);
    const source = i.app?.trim() || group.map((g) => g.label?.trim()).find((l) => !!l) || null;
    out.push({ kind: "group", group: { id: k, items: group, source } });
  }
  return out;
}

/** "Screen captures from Terminal, Wed, Nov 18: 14". The count is last so every language reads it. */
export function groupTitle(g: CaptureGroup, locale: string): string {
  const day = dayLabel(new Date(g.items[0].date_unix_ms), locale);
  const n = g.items.length;
  return g.source
    ? t("Screen captures from {source}, {day}: {n}", { source: g.source, day, n })
    : t("Screen captures on {day}: {n}", { day, n });
}

/** "3 of 14 ticked" for a group that is partly ticked. */
export const groupProgress = (ticked: number, n: number) => t("{ticked} of {n} ticked", { ticked, n });

/** The receipt's devices line. Null from core means not measured yet: never shown as 0. */
export function pendingDevicesLine(n: number | null): string {
  if (n == null)
    return t("Not known yet how many of your other devices still need to update to apply this delete. Sync works it out on a later round.");
  // Never "none": a device on an older version writes nothing the count can see (spec 13.3).
  if (n === 0)
    return t(
      "No device that has updated is still waiting to apply this delete. A device on an older version, or one you removed from the count in Settings, may not have applied it.",
    );
  if (n === 1) return t("1 device still needs to update before it applies this delete.");
  return t("{n} devices still need to update before they apply this delete.", {
    n,
  });
}

export const reappearsLine = () => t("A client can reappear as a new topic if new mail or a new meeting matches it.");
/** True on this PC only: other devices keep their derived copies until their own retention (Left for you says so). */
export const derivedLine = () =>
  t("On this PC, summaries, suggestions, people and organisations that Whimbrel made and that name this client are deleted too.");
export const gmailThreadsLine = () =>
  t("A Gmail thread summary may keep text from mail in that thread that already expired from Whimbrel.");
/** Shown only when the delete removed the unencrypted copy kept from the encryption upgrade. */
export const rollbackCopyRemovedLine = (r: ClientDeleteReceipt): string | null =>
  r.rollback_copy_removed === true
    ? t("The unencrypted copy of the memory database kept from the encryption upgrade was removed too.")
    : null;

export function receiptHeadline(r: ClientDeleteReceipt): string {
  if (r.client === "removed_here_only") {
    return t("Deleted on this PC only. Your other devices were not told, because this PC has no sync identity.");
  }
  if (r.client === "removed") {
    if (r.complete) return t("Deleted this client and the items you chose. Check the places listed under Left for you.");
    // The client record is gone, so a retry finds nothing to run (core's retry_helps can't know that).
    if (r.failed_stages.includes("cleanup")) return cleanupNotFinishedLine();
    return t("The client is deleted, but some items could not be removed. They are listed below.");
  }
  return r.retry_helps
    ? t("Not finished: some of this client's material is still here. Try again to finish.")
    : t("Not finished: some of this client's material is still here. It is listed below.");
}

/** A clean-up that failed after the client record went: it can't be run again from the receipt. */
export const cleanupNotFinishedLine = () =>
  t(
    "The client is deleted, but the last clean-up step did not finish, so words from the deleted material can stay in Whimbrel's search index on this PC. The client record is already gone, so this delete can't be run again. Whimbrel runs the clean-up again the next time you delete a client.",
  );

/** Whether "Try again" can do anything for this receipt: only while the client record is still here (a retry after
 *  `removed` or `removed_here_only` finds no client). */
export const receiptRetryHelps = (r: ClientDeleteReceipt) => r.retry_helps && r.client === "still_present";

export function stageName(s: ClientDeleteStage): string {
  switch (s) {
    case "derived_text":
      return t("text Whimbrel made from this material");
    case "client_record":
      return t("the client record");
    case "cleanup":
      return t("clean-up");
    case "files":
      return t("files");
    case "no_device_id":
      return t("telling your other devices");
  }
}

/** Items or steps that failed, as kind names; null when nothing failed. */
export function failedLine(r: { failed: ClientItemKind[]; failed_stages: ClientDeleteStage[] }): string | null {
  const all = [...r.failed.map(kindName), ...r.failed_stages.filter((s) => s !== "no_device_id").map(stageName)];
  return all.length === 0 ? null : t("Not removed: {list}.", { list: all.join(", ") });
}

/** One "Left for you" entry: a sentence and the paths it names (shown as is). */
export interface LeftLine {
  text: string;
  paths: string[];
}

export function leftForYouLine(l: ClientLeftForYou): LeftLine | null {
  if (typeof l === "string") {
    switch (l) {
      case "mailbox_originals":
        return {
          text: t("The original emails stay in your mailbox. Delete them there if you need to."),
          paths: [],
        };
      case "backups_and_cloud":
        return {
          text: t(
            "File History, Windows Backup and cloud backups (OneDrive, Dropbox and others) can hold copies of files, the vault and exports. Whimbrel cannot reach them; they are yours to handle. Files are often the hardest part, because backups keep copies.",
          ),
          paths: [],
        };
      case "relay_copies_before_this_release":
        return {
          text: t("Copies your devices sent to the sync relay before this version stay there, encrypted. Current devices ignore them."),
          paths: [],
        };
      case "copies_on_other_devices":
        return {
          text: t(
            "Your other devices: most copies go when each device syncs and applies the delete. Routine results, profile suggestions, people's names and meeting briefs that named this client can stay on each device: the delete does not remove them there.",
          ),
          paths: [],
        };
      case "vault_sync_off":
        return {
          text: t("Notes in your vault are not touched while vault sync is off. Delete them yourself if they name this client."),
          paths: [],
        };
      default:
        return null;
    }
  }
  if ("media_folder" in l) {
    return {
      text: t(
        "The media folder. Whimbrel removes this client's audio and slide images from it; copies that a backup or cloud sync made of it stay.",
      ),
      paths: [l.media_folder],
    };
  }
  if ("vault_folder" in l) {
    return {
      text: t("Your Obsidian vault. Whimbrel removes this client's notes from it; copies that a backup or sync made stay."),
      paths: [l.vault_folder],
    };
  }
  if ("export_folders" in l) {
    return {
      text: t(
        "Earlier exports of this client. They are plain copies, yours to keep or delete. An export you moved or renamed is not listed here: delete it yourself.",
      ),
      paths: l.export_folders,
    };
  }
  if ("files_not_removed" in l)
    return {
      text: t("Files Whimbrel could not remove:"),
      paths: l.files_not_removed,
    };
  if ("expired_here_only" in l) {
    if (l.expired_here_only === 1) {
      return {
        text: t("1 item was removed earlier on this PC only by Keep for N months. Your other devices may still hold it."),
        paths: [],
      };
    }
    return {
      text: t("{n} items were removed earlier on this PC only by Keep for N months. Your other devices may still hold them.", {
        n: l.expired_here_only,
      }),
      paths: [],
    };
  }
  return null;
}

/**
 * The Exclude toggle's warning (final review, Major 22): while a device without the topics marker is on the relay,
 * every client's settings row is held on this PC, so the others never learn of the exclusion. Removing a device
 * from the count does not release it. Null for none and for not known (never said as 0).
 */
export function heldDevicesLine(n: number | null): string | null {
  if (n == null || n <= 0) return null;
  if (n === 1)
    return t(
      "1 device on your sync hasn't updated yet. Until it does, this setting stays on this PC: your other devices don't receive it, and they could still send this client's material to a hosted model. Update that device. Until then, set it on each other device where this client appears; a client added here may not appear there yet. Removing it from the count in Settings doesn't change this.",
    );
  return t(
    "{n} devices on your sync haven't updated yet. Until they do, this setting stays on this PC: your other devices don't receive it, and they could still send this client's material to a hosted model. Update those devices. Until then, set it on each other device where this client appears; a client added here may not appear there yet. Removing them from the count in Settings doesn't change this.",
    { n },
  );
}

/** The matcher's limit (final review, Major 5; ruled: stated, matcher unchanged). */
export const matchingLimitLine = () =>
  t(
    "Matching is on whole words of the client's name, aliases and known contact addresses. A name inside another word or in text written without spaces (some inflected forms, some scripts) may not match, and that material can still go to a hosted model.",
  );

/** Under an answer a hosted model gave without some retrieved items (core's `filtered_hosted`; items, not clients). */
export function leftOutLine(n: number): string | null {
  if (!(n > 0)) return null;
  if (n === 1) return t("Left out 1 item that involves an excluded client.");
  return t("Left out {n} items that involve an excluded client.", { n });
}

/** Why a keep-for-N-months run stopped. `no_device_id` has its own words: it is about which items synced. */
export function expiryStoppedLine(e: ClientOpErrorKind): string {
  if (e === "no_device_id")
    return t(
      "Not deleted: sync is on, but this PC has no device identity, so Whimbrel can't tell which of these items your other devices already have. Quit and reopen Whimbrel to try again.",
    );
  return clientMessage(e);
}

/** One "Left for you" entry on a keep-for-N-months result. An expiry never reaches other devices, so the delete's
 *  other-devices line would be false there. */
export function expiryLeftForYouLine(l: ClientLeftForYou): LeftLine | null {
  if (l === "copies_on_other_devices")
    return { text: t("This ran on this PC only. Your other devices keep their copies until each one expires them or you delete them there."), paths: [] };
  return leftForYouLine(l);
}

export function clientMessage(e: ClientOpErrorKind): string {
  switch (e) {
    case "not_found":
      return t("This client is no longer here.");
    case "no_device_id":
      return noIdentityLine();
    case "invalid_input":
      return t("Whimbrel couldn't read the request. Nothing was changed.");
    case "selection_changed":
      return t("The list changed while you were reviewing it. Nothing was deleted. Check it and confirm again.");
    case "busy":
      return t("Background work didn't finish in time. Nothing was deleted. Try again in a moment.");
    case "cancelled":
      return t("Cancelled. Nothing was deleted.");
    case "folder_not_allowed":
      return t("Choose a folder on this PC, not a network share. Nothing was written.");
    case "failed":
      return t("Whimbrel couldn't read or write its data or a folder. Try again.");
  }
}

export const noIdentityLine = () =>
  t(
    "Whimbrel can't delete a client on this PC: it has no device identity, so your other devices would never hear of the delete. Quit and reopen Whimbrel to try again.",
  );

export const isRetryable = (e: ClientOpErrorKind) => e === "busy" || e === "failed";

export function keepLabel(months: number | null): string {
  if (months == null) return t("Off");
  if (months === 1) return t("1 month");
  return t("{n} months", { n: months });
}

/** The client names other items also belong to, for the "deleted for both" lines. */
export function sharedWith(items: ClientItem[]): string[] {
  return [...new Set(items.map((i) => i.also_client).filter((c): c is string => !!c))].sort();
}
