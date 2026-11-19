// The shapes core's client controls answer with (X6 Task 9; `core/api/src/client_controls.rs`). The shell
// passes core's JSON through unchanged, the same JSON the Mac decodes, so these are written by hand rather
// than generated from the shell's schema.

/** Why a client-control call did not run. Never names a client. */
export type ClientOpErrorKind =
  /** Not a live client (unknown, deleted, or merged into another). */
  | "not_found"
  /** A delete, or a keep-for-N-months expiry while sync is on, was refused before it started: this PC has no
   *  device id. Not retryable until it has one. */
  | "no_device_id"
  /** The request could not be read (a programming error on this side). */
  | "invalid_input"
  /** A ticked item is no longer in the client's inventory. Nothing was touched: reload the review screen. */
  | "selection_changed"
  /** Background work did not finish in time. Nothing was touched: try again in a moment. */
  | "busy"
  /** The user cancelled a delete while it waited for background work (PR 875). Nothing was touched. */
  | "cancelled"
  /** An export may not go to a network share. Nothing was written. */
  | "folder_not_allowed"
  /** The database or a folder could not be read or written; running it again may help. */
  | "failed";

/** core's envelope. */
export type ClientResult<T> = { ok: true; data: T } | { ok: false; error: { kind: ClientOpErrorKind } };

export type ClientItemKind =
  | "meeting"
  | "email"
  | "thread"
  | "calendar_event"
  | "capture"
  | "episode"
  | "conversation"
  | "step"
  | "vault_note"
  | "export_folder";

/** An item as the review screen sends it back: core matches it against a fresh inventory by kind and id. */
export interface ClientItemRef {
  kind: ClientItemKind;
  /** A capture's id may be `i:<n>`. */
  id: string;
}

export interface ClientItem extends ClientItemRef {
  /** Another client that also holds this item (display only). */
  also_client: string | null;
  date_unix_ms: number;
  /** A calendar event marked removed: still stored, still deleted ("removed events"). */
  removed: boolean;
  /**
   * The row's short name (a title, a subject, a step's text) for this screen only. Client text: never in a
   * receipt or a log line. Null or missing: the row shows its kind and date.
   */
  label?: string | null;
  /**
   * Why it matched: the line of the item's text that names the client, trimmed around the name (PR 875). Client
   * text, for this screen only, like `label`. Missing when the item has no such text.
   */
  excerpt?: string;
  /** A possible screen capture's app id, for grouping captures from one app. Not client text. */
  app?: string;
}

/** `[kind, n]`. */
export type ClientKindCount = [ClientItemKind, number];

/** Places Whimbrel cannot clear for the user. */
export type ClientLeftForYou =
  | "mailbox_originals"
  | "backups_and_cloud"
  | "relay_copies_before_this_release"
  /** Derived copies other devices keep until cleared there. */
  | "copies_on_other_devices"
  /** The Obsidian vault integration is off on this PC: the vault was not touched. */
  | "vault_sync_off"
  | { media_folder: string }
  | { vault_folder: string }
  | { export_folders: string[] }
  | { files_not_removed: string[] }
  /** This many items were removed earlier by keep-for-N-months on this PC only. */
  | { expired_here_only: number };

export interface ClientInventory {
  linked: ClientItem[];
  /** Name matches, offered unticked; never deleted unless ticked. */
  possible: ClientItem[];
  possible_truncated: boolean;
  counts: ClientKindCount[];
  left_for_you: ClientLeftForYou[];
}

export type ClientDeleteStage = "derived_text" | "client_record" | "cleanup" | "files" | "no_device_id";

/** What a client delete did (no names, no text). */
export interface ClientDeleteReceipt {
  date_unix_ms: number;
  counts: ClientKindCount[];
  /** Of `counts`: not sent to other devices. */
  not_sent: ClientKindCount[];
  /** Of `counts`: only ever on this PC. */
  local_only: ClientKindCount[];
  client: "removed" | "removed_here_only" | "still_present";
  left_for_you: ClientLeftForYou[];
  /** Devices that still need to update; null = not known yet (never show it as 0). */
  pending_devices: number | null;
  failed: ClientItemKind[];
  /** `no_device_id` is not retryable. */
  failed_stages: ClientDeleteStage[];
  complete: boolean;
  /**
   * "Try again" can finish it, but only with a selection rebuilt from a fresh `clientInventory`: an item the
   * partial run already removed is no longer listed, so resending the same ticked items answers
   * `selection_changed`.
   */
  retry_helps: boolean;
  /**
   * The unencrypted copy kept from the encryption upgrade (`<db>.plain.bak`) was removed too, because it still
   * held this client's material. Missing from an older core. A copy that could not be removed is the `files` stage.
   */
  rollback_copy_removed?: boolean;
}

export interface ClientControl {
  exclude_hosted: boolean;
  /** null (or 0): no keep period. */
  keep_months: number | null;
}

/** One setting to change on the stored value (`client_control_update`). */
export type ClientControlChange = { exclude_hosted: boolean } | { keep_months: number | null };

/** A settings write: the value read in the same call, and the value saved. */
export interface ClientControlUpdate {
  before: ClientControl;
  after: ClientControl;
}

/** What turning exclusion on offers. Re-summarise each meeting with the existing summary command: core keeps an
 *  excluded client's meeting on this PC. */
export interface ClientResummaryOffer {
  meetings: number[];
  already_left_note: string;
}

export interface ClientDue {
  key: string;
  items: ClientItem[];
}

export type ClientRetentionDecision = { decision: "keep" } | { decision: "snooze"; until_ms: number };

export interface ClientReviewFlag {
  clients: number;
  items: number;
  raised_at_unix_ms: number;
}

/** What a keep-for-N-months expiry did on this PC. */
export interface ClientExpiryReceipt {
  date_unix_ms: number;
  counts: ClientKindCount[];
  /** Confirmed but no longer due: left. */
  not_due: ClientKindCount[];
  /** Not sent to other devices yet: left, offered again later. */
  waiting_to_sync: ClientKindCount[];
  /** Stopped at its time budget: call again. */
  more: boolean;
  failed: ClientItemKind[];
  failed_stages: ClientDeleteStage[];
  left_for_you: ClientLeftForYou[];
  complete: boolean;
}

export interface ClientExportDone {
  folder: string;
  media: { files: number; skipped: number; failed: number };
  /** The text is complete, but some media was not copied. */
  partial: boolean;
}

export interface ClientPendingDevices {
  pending_devices: number | null;
  /**
   * Devices on the relay without the topics marker: while any is there, every client's settings row (and so an
   * exclusion) is held on this PC and reaches no other device. Removed devices still count. null = not measured
   * yet: never show it as 0.
   */
  topic_rows_held_devices: number | null;
}

/** The devices `ClientPendingDevices` counts, by sync device id (a device's public key). */
export interface ClientPendingDeviceIds {
  devices: string[] | null;
}
