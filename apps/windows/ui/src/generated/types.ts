// Generated from apps/windows/src-tauri's command DTOs by `npm run gen:types`.
// Do not edit. Change the Rust type, run `cargo run --bin gen-schema`, then `npm run gen:types`.

/**
 * What a pending action would do, in the user's terms.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ActionView".
 */
export type ActionView =
  | {
      endUnixMs: number;
      kind: "createCalendarEvent";
      startUnixMs: number;
      title: string;
    }
  | {
      body: string;
      kind: "sendEmail";
      subject: string;
      to: string;
    };
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "Provider".
 */
export type Provider = ("ollama" | "anthropic" | "openai" | "google") | "openrouter";
/**
 * The LLM call sites that carry their own free-text instructions — the Mac's
 * `PromptSurface` (`PromptLibrarySettings.swift`). Stored under `prompts.<raw value>`,
 * the Mac's settings-sync field key; empty means "no extra instructions".
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PromptSurface".
 */
export type PromptSurface =
  "ask" | "meetingSummary" | "dailyDigest" | "weeklyDigest" | "episodeSummary" | "threadSummary";
/**
 * What the UI hears about one question, over a Tauri `Channel`.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ChatEvent".
 */
export type ChatEvent =
  | {
      context: ContextEntry[];
      kind: "started";
      model: string;
      provider: string;
      runId?: string | null;
    }
  | {
      kind: "delta";
      text: string;
    }
  | {
      kind: "progress";
      label: string;
    }
  | {
      kind: "fell_back";
    }
  | {
      answer: string;
      citations: ContextItemRef[];
      conversationId?: number | null;
      kind: "completed";
    }
  | {
      kind: "cancelled";
    }
  | {
      error: CommandError;
      kind: "failed";
      partial: boolean;
    }
  | {
      conversationId: number;
      kind: "titled";
      title: string;
    };
/**
 * A citation pointing back to one retrieved context item, tagged with which table
 * (`capture_item` or `transcript_segment`) the id belongs to — see [`ContextItem`]'s
 * doc comment for why a bare `i64` isn't safe here. `PartialOrd`/`Ord` (variant
 * declaration order, then field order) give `assemble_context` a deterministic tiebreak
 * for score ties, independent of `HashMap` iteration order in `rrf_fuse`.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ContextItemRef".
 */
export type ContextItemRef =
  | {
      id: number;
      kind: "capture";
    }
  | {
      id: number;
      kind: "transcript";
    }
  | {
      id: number;
      kind: "routine_run";
    }
  | {
      id: number;
      kind: "episode";
    }
  | {
      id: number;
      kind: "thread";
    }
  | {
      id: number;
      kind: "snapshot";
    };
/**
 * A Library detail view a citation can open (the UI's `Selection`).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "LibraryTarget".
 */
export type LibraryTarget =
  | {
      id: number;
      kind: "capture";
    }
  | {
      id: number;
      kind: "meeting";
    }
  | {
      id: number;
      kind: "episode";
    };
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CommandError".
 */
export type CommandError =
  | {
      cap_cents: number;
      kind: "spend_cap";
      scope: SpendScope;
      spent_cents: number;
    }
  | {
      kind: "provider_failure";
    }
  | {
      kind: "store";
      message: string;
    }
  | {
      kind: "secrets";
      message: string;
    }
  | {
      kind: "network";
      message: string;
    }
  | {
      kind: "not_configured";
      message: string;
    }
  | {
      kind: "invalid_input";
      message: string;
    };
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SpendScope".
 */
export type SpendScope = "user" | "global";
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "KeyCheckStatus".
 */
export type KeyCheckStatus = ("valid" | "other") | "refused" | "unreachable";
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "LibraryRow".
 */
export type LibraryRow =
  | (CaptureRow & {
      kind: "capture";
    })
  | (MeetingRow & {
      kind: "meeting";
    })
  | (EpisodeRow & {
      kind: "episode";
    })
  | (TranscriptRow & {
      kind: "transcript";
    })
  | (SnapshotRow & {
      kind: "snapshot";
    });
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "Stage".
 */
export type Stage = "transcribing" | "summarizing" | "identifying_speakers";
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RecordingState".
 */
export type RecordingState = "idle" | "starting" | "recording";
/**
 * The built-in routines, as the UI names them.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RoutineKind".
 */
export type RoutineKind =
  "dailyBrief" | "focusTime" | "eodRecap" | "weeklyDigest" | "replyPriorities" | "meetingPrep";
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RunOutcome".
 */
export type RunOutcome = ("success" | "failed" | "rejected") | "pending";
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SyncState".
 */
export type SyncState =
  | "notConfigured"
  | "notSignedIn"
  | "signingIn"
  | "waitingForApproval"
  | "waitingForGroupKey"
  | "ready"
  | "error";
/**
 * The workspace's phase, matching core's `WorkspaceState`.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "WorkspacePhase".
 */
export type WorkspacePhase = "before" | "during" | "after" | "cancelled";

/**
 * Generated from apps/windows/src-tauri's command DTOs. Do not edit; run `cargo run --bin gen-schema`.
 */
export interface WhimbrelDtos {}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ActionItemView".
 */
export interface ActionItemView {
  completed: boolean;
  id: number;
  /**
   * "generated" (from the summary) or "user".
   */
  source: string;
  text: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "AiSettings".
 */
export interface AiSettings {
  /**
   * The Brave Search key the skin maker finds images with, masked; `None` when unset.
   */
  imageSearchKey?: string | null;
  keys: ProviderKey[];
  prompts: PromptText[];
  providers: ProviderInfoView[];
  routing: RoutingSettings;
  spend?: SpendView | null;
  /**
   * Extra instructions for every summary (`summary.customPromptInstructions`).
   */
  summaryInstructions: string;
  usage?: UsageView | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProviderKey".
 */
export interface ProviderKey {
  /**
   * The key with all but its last four characters hidden; `None` when unset.
   */
  masked?: string | null;
  provider: Provider;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PromptText".
 */
export interface PromptText {
  surface: PromptSurface;
  text: string;
}
/**
 * One row of the key on-ramp table (core/api::providers): where to get a key, its data
 * policy, what a key looks like, and the dated data-terms note where a free tier may
 * use prompts.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProviderInfoView".
 */
export interface ProviderInfoView {
  /**
   * The light-tier model; empty where the user picks it (OpenRouter) or there is none.
   */
  cheapModel: string;
  dataTerms?: DataTermsView | null;
  id: Provider;
  keyPageUrl?: string | null;
  keyPrefix?: string | null;
  name: string;
  policyUrl?: string | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DataTermsView".
 */
export interface DataTermsView {
  /**
   * ISO date the policy page was last read.
   */
  checked: string;
  text: string;
}
/**
 * Provider choice, default models and governance, as the Settings UI edits them.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RoutingSettings".
 */
export interface RoutingSettings {
  anthropicModel: string;
  /**
   * Light tasks (titles, topic names, profile suggestions) use the cheapest model the
   * user has. On by default, matching core.
   */
  cheapFirst: boolean;
  globalDailyCapCents?: number | null;
  googleModel: string;
  /**
   * FR-CHAT-5 local-only mode: Ollama only, whatever else is configured.
   */
  localOnly: boolean;
  ollamaModel: string;
  openaiModel: string;
  /**
   * The OpenRouter pick, also the model light tasks use there. Empty means none yet:
   * core then leaves OpenRouter out of every chain, because their catalogue changes
   * weekly and there is no fallback model to substitute.
   */
  openrouterModel: string;
  preferredProvider: Provider;
  /**
   * Whether Ollama is a link in the chain at all; ignored in local-only mode. On by default.
   */
  useLocalModel: boolean;
  /**
   * `None` = the built-in default cap; `Some(0)` = no cap (Mac semantics).
   */
  userDailyCapCents?: number | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SpendView".
 */
export interface SpendView {
  globalCents: number;
  userCents: number;
}
/**
 * Spend in cents for the Model usage section (core/api::usage::totals), UTC days.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "UsageView".
 */
export interface UsageView {
  byProvider: ProviderSpendView[];
  last30DaysCents: number;
  monthToDateCents: number;
  todayCents: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProviderSpendView".
 */
export interface ProviderSpendView {
  cents: number;
  provider: string;
}
/**
 * What the alert window shows.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "AlertView".
 */
export interface AlertView {
  /**
   * Whether the countdown runs (Settings → Meetings → start recording automatically).
   */
  autoStart: boolean;
  countdownSeconds: number;
  meeting?: DetectedMeeting | null;
}
/**
 * A meeting Whimbrel is offering to record: a call it noticed, or a calendar event whose
 * lead window has opened (M3.3 Task 7).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DetectedMeeting".
 */
export interface DetectedMeeting {
  appId: string;
  /**
   * A calendar meeting's invitees, which become the expected speaker count.
   */
  attendees: string[];
  /**
   * `detected-<app>-<epoch seconds>` from core's debounce, or the calendar event's id.
   */
  id: string;
  /**
   * When the alert appeared, for its countdown.
   */
  promptedUnixMs: number;
  /**
   * The process tree to record, or 0 for a calendar meeting with no app in sight.
   */
  rootPid: number;
  /**
   * "detected" or "calendar": which trigger the meeting row gets.
   */
  source: string;
  /**
   * A calendar meeting's own start, so the row spans the event, not the moment Accept
   * was pressed.
   */
  startUnixMs?: number | null;
  /**
   * "Zoom", "Google Meet", or the event's title.
   */
  title: string;
}
/**
 * What Settings → About shows.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "AppStatus".
 */
export interface AppStatus {
  dataDir: string;
  /**
   * This device's sync id (base64 public key), if the identity loaded.
   */
  deviceId?: string | null;
  embeddingModelLoaded: boolean;
  startupErrors: string[];
  storeOpen: boolean;
  storeQueueWait?: QueueWaitStats | null;
  version: string;
}
/**
 * How long jobs waited for the store, over the last 4,096 jobs.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "QueueWaitStats".
 */
export interface QueueWaitStats {
  /**
   * Jobs run since launch.
   */
  jobs: number;
  maxMs: number;
  p50Ms: number;
  p95Ms: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ApprovalView".
 */
export interface ApprovalView {
  account?: string | null;
  /**
   * The accounts this could be done with, and the one used unless the user picks another.
   */
  accounts: string[];
  /**
   * `None` when the draft could not be read; it can still be rejected.
   */
  action?: ActionView | null;
  /**
   * Why Approve cannot work yet (no account connected), or `None`.
   */
  blocked?: string | null;
  ranAtUnixMs: number;
  routineName: string;
  runId: number;
  /**
   * The routine's own one-line description of the draft.
   */
  summary: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ApproveResult".
 */
export interface ApproveResult {
  /**
   * The list as it is now.
   */
  approvals: ApprovalView[];
  /**
   * What to tell the user: why it is still waiting, or that someone else already handled it.
   */
  message?: string | null;
}
/**
 * Today's Daily Brief as the recap: the routine's own summary, from this PC or a synced device
 * (docs/superpowers/plans/2026-10-17-today-recap-daily-brief-windows.md).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "BriefRecap".
 */
export interface BriefRecap {
  /**
   * The device that ran it, when that is not this PC.
   */
  from?: DeviceTag | null;
  ranAtUnixMs: number;
  summary: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DeviceTag".
 */
export interface DeviceTag {
  deviceId: string;
  label: string;
  thisDevice: boolean;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CalendarAccounts".
 */
export interface CalendarAccounts {
  /**
   * Why calendar syncing can't run right now (no Node, connector not staged, sync not
   * configured), or `None` when it can.
   */
  blocked?: string | null;
  emails: string[];
  /**
   * When the last round finished, and why it failed if it did.
   */
  lastSyncedUnixMs?: number | null;
  /**
   * The next one's title and start, for the card's line.
   */
  nextMeeting?: string | null;
  nextStartUnixMs?: number | null;
  syncError?: string | null;
  /**
   * Meetings still to come that a prompt could fire for.
   */
  upcoming: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CaptureDetail".
 */
export interface CaptureDetail {
  app: string;
  device: DeviceTag;
  id: number;
  snippet: string;
  /**
   * `uia`, `ocr` (Windows) or `ax` (Mac); `None` for rows older than the field.
   */
  source?: string | null;
  text: string;
  title: string;
  tsUnixMs: number;
  url?: string | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CaptureRow".
 */
export interface CaptureRow {
  app: string;
  device: DeviceTag;
  id: number;
  snippet: string;
  /**
   * `uia`, `ocr` (Windows) or `ax` (Mac); `None` for rows older than the field.
   */
  source?: string | null;
  title: string;
  tsUnixMs: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CaptureSettings".
 */
export interface CaptureSettings {
  excluded: string[];
  ocr: OcrLanguages;
  runningApps: RunningApp[];
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "OcrLanguages".
 */
export interface OcrLanguages {
  /**
   * Languages Windows can OCR (installed language packs with OCR).
   */
  available: string[];
  /**
   * Whether the most preferred language has an OCR pack.
   */
  covered: boolean;
  /**
   * The user's preferred languages, most preferred first.
   */
  profile: string[];
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RunningApp".
 */
export interface RunningApp {
  /**
   * The §4.3 identity capture stores (normalized); what an exclusion matches.
   */
  appId: string;
  exe: string;
  title: string;
}
/**
 * What Settings shows about capture.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "CaptureStatus".
 */
export interface CaptureStatus {
  enabled: boolean;
  /**
   * Reads skipped because the window is excluded.
   */
  excluded: number;
  lastApp?: string | null;
  lastError?: string | null;
  lastStoredAtMs?: number | null;
  ocrRuns: number;
  /**
   * Why capture is paused right now, if it is.
   */
  paused?: string | null;
  /**
   * False if the service could not start (see `last_error`).
   */
  running: boolean;
  /**
   * Reads that produced new stored items.
   */
  stored: number;
  /**
   * Reads skipped as near-identical to the window's last stored text.
   */
  unchanged: number;
}
/**
 * One item the answer may cite. The UI matches a citation to it by `item`. A saved
 * exchange stores the entries it cited in this shape.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ContextEntry".
 */
export interface ContextEntry {
  item: ContextItemRef & {
    [k: string]: unknown;
  };
  label: string;
  /**
   * Where clicking the citation goes; `None` for routine runs and threads, which have
   * no Library view yet.
   */
  open?: LibraryTarget | null;
  tsUnixMs?: number | null;
}
/**
 * One message in a skin's chat, stored as `chat.json` in the skin's folder in the Mac's shape.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ChatLine".
 */
export interface ChatLine {
  /**
   * Seconds since 2001-01-01 UTC, as Swift's `JSONEncoder` writes a `Date`.
   */
  date: number;
  /**
   * `"user"` or `"whimbrel"`.
   */
  role: string;
  text: string;
}
/**
 * The Ask composer's "X¢ / cap¢ today". Spend is the anonymous identity's, which is
 * what the user cap gates.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ChatSpend".
 */
export interface ChatSpend {
  /**
   * `None`: no cap (the setting is 0).
   */
  capCents?: number | null;
  userCents: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ConversationRow".
 */
export interface ConversationRow {
  id: number;
  starred: boolean;
  title: string;
  updatedAtUnixMs: number;
}
/**
 * A saved conversation.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ConversationView".
 */
export interface ConversationView {
  exchanges: ExchangeView[];
  id: number;
  starred: boolean;
  title: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ExchangeView".
 */
export interface ExchangeView {
  answer: string;
  /**
   * The entries the answer cited, as saved. Empty for exchanges saved before
   * citations were stored.
   */
  citations: ContextEntry[];
  createdAtUnixMs: number;
  /**
   * The model that answered; `None` for exchanges saved before it was stored.
   */
  model?: string | null;
  query: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DeviceCount".
 */
export interface DeviceCount {
  captures: number;
  device: DeviceTag;
  /**
   * The newest capture today; `None` when the device captured nothing today.
   */
  lastTsUnixMs?: number | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DeviceView".
 */
export interface DeviceView {
  /**
   * Short code both devices show while approving (`None` for an unparseable key).
   */
  comparisonCode?: string | null;
  enrolledAtUnixMs: number;
  /**
   * The Worker's device id (what approve / revoke take).
   */
  id: string;
  name?: string | null;
  platform?: string | null;
  /**
   * The device's public key, which is its `device_id` on timeline rows.
   */
  publicKey: string;
  /**
   * `pending`, `approved` or `revoked`.
   */
  status: string;
  thisDevice: boolean;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "DownloadView".
 */
export interface DownloadView {
  cancelled: boolean;
  done: boolean;
  error?: string | null;
  fraction: number;
  jobId: string;
  status: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "EpisodeRow".
 */
export interface EpisodeRow {
  app: string;
  /**
   * Its app is in this PC's hidden-apps list.
   */
  appHidden: boolean;
  device: DeviceTag;
  endTsUnixMs: number;
  /**
   * Hidden on its own (synced across devices by episode identity).
   */
  hidden: boolean;
  id: number;
  itemCount: number;
  /**
   * The core's summary state, e.g. `pending` or `summarize_failed`.
   */
  status: string;
  summary?: string | null;
  title?: string | null;
  tsUnixMs: number;
}
/**
 * One learned fact: a single line, with the meeting it was suggested from when it came from a
 * suggestion.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "Fact".
 */
export interface Fact {
  id: string;
  sourceMeetingId?: number | null;
  text: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "GeneralSettings".
 */
export interface GeneralSettings {
  /**
   * Windows starts Whimbrel at sign-in (the `Run` entry exists).
   */
  autostart: boolean;
  /**
   * False in debug builds, where the toggle is read-only.
   */
  autostartAvailable: boolean;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "GmailAccount".
 */
export interface GmailAccount {
  email: string;
  /**
   * When Whimbrel last read this account's mail, and why the last round failed if it did.
   */
  lastReadUnixMs?: number | null;
  /**
   * The stored grant can no longer send: its tokens are gone, or Google did not grant the
   * permission to send. Connecting the same address again fixes it.
   */
  needsReconnect: boolean;
  readError?: string | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "GmailAccounts".
 */
export interface GmailAccounts {
  accounts: GmailAccount[];
  /**
   * Why Gmail can't be connected right now (no Google client, no Node, connector not staged), or `None`.
   */
  blocked?: string | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "HostOption".
 */
export interface HostOption {
  /**
   * The device's public key: what the choice is stored as.
   */
  id: string;
  label: string;
}
/**
 * The "Routines run on" picker.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "HostView".
 */
export interface HostView {
  caption?: string | null;
  /**
   * The approved devices to choose from, this PC first.
   */
  options: HostOption[];
  /**
   * What is stored; empty means any device. A choice that is no longer an approved device is kept
   * here (it means "any device" to the gate) but not shown as selected.
   */
  primaryDeviceId: string;
  /**
   * What the picker shows as selected: the stored choice if it is still an option, else empty.
   */
  selected: string;
}
/**
 * A found or made image waiting for approval (`llm::images::ImageCandidate`). A made one has a
 * `generated:` url, and its bytes wait here until it is approved or rejected.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ImageCandidateView".
 */
export interface ImageCandidateView {
  /**
   * `"web"` or `"generated"`.
   */
  source: string;
  thumbnailUrl: string;
  title: string;
  url: string;
}
/**
 * What saving a key found out (core/api::hosted_models::validate_key).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "KeyCheckView".
 */
export interface KeyCheckView {
  /**
   * The one line the key row shows.
   */
  message: string;
  models: string[];
  status: KeyCheckStatus;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "MeetingRow".
 */
export interface MeetingRow {
  device: DeviceTag;
  endUnixMs?: number | null;
  hasSummary: boolean;
  id: number;
  status: string;
  title: string;
  /**
   * How it started: "calendar", "detected" or "manual".
   */
  trigger: string;
  tsUnixMs: number;
}
/**
 * A transcript search hit. Transcript segments carry no device of their own.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TranscriptRow".
 */
export interface TranscriptRow {
  id: number;
  meetingId: number;
  meetingTitle: string;
  snippet: string;
  speaker: string;
  /**
   * Offset from the meeting start, in centiseconds.
   */
  startCs: number;
}
/**
 * A slide-text search hit: a snapshot saved during a meeting (local to this device).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SnapshotRow".
 */
export interface SnapshotRow {
  /**
   * The `items` row id, as Ask cites it.
   */
  id: number;
  meetingId: number;
  meetingTitle: string;
  snippet: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "LibrarySettings".
 */
export interface LibrarySettings {
  /**
   * Apps whose episodes the Episodes list hides, on this PC only.
   */
  hiddenEpisodeApps: string[];
  showScreenCaptures: boolean;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "McpClientView".
 */
export interface McpClientView {
  createdUnixMs: number;
  id: number;
  name: string;
  revokedUnixMs?: number | null;
  scopes: string[];
}
/**
 * Returned once, when a client is made. The token is not recoverable afterwards.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "McpNewClient".
 */
export interface McpNewClient {
  id: number;
  token: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "McpServerView".
 */
export interface McpServerView {
  address: string;
  /**
   * The user's switch. It can be on while the server is not running (it failed).
   */
  enabled: boolean;
  error?: string | null;
  /**
   * `stopped`, `running` or `failed`.
   */
  status: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "MeetingDetail".
 */
export interface MeetingDetail {
  actionItems: string[];
  attendees: string[];
  decisions: string[];
  device: DeviceTag;
  endUnixMs?: number | null;
  hasSummary: boolean;
  id: number;
  notes?: string | null;
  openQuestions: string[];
  status: string;
  summaryTldr?: string | null;
  title: string;
  /**
   * How it started: "calendar", "detected" or "manual".
   */
  trigger: string;
  tsUnixMs: number;
}
/**
 * Which meeting models are on disk.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "MeetingModelsStatus".
 */
export interface MeetingModelsStatus {
  dir: string;
  installed: boolean;
  /**
   * Display names of the models still missing.
   */
  missing: string[];
}
/**
 * What the UI and the tray show about meeting recording.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "MeetingRecordingView".
 */
export interface MeetingRecordingView {
  /**
   * Why this recording may be missing audio, while it runs.
   */
  degraded?: string | null;
  /**
   * The last meeting whose transcript or summary failed.
   */
  failure?: PipelineFailure | null;
  meetingId?: number | null;
  /**
   * The meeting being transcribed or summarized right now, if any (M3.3 Task 4).
   */
  processing?: Processing | null;
  startedUnixMs?: number | null;
  state: RecordingState;
}
/**
 * A meeting whose pipeline failed, kept until another one runs.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PipelineFailure".
 */
export interface PipelineFailure {
  meetingId: number;
  message: string;
}
/**
 * What the pipeline is doing, for the Meetings tab.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "Processing".
 */
export interface Processing {
  label: string;
  meetingId: number;
  stage: Stage;
}
/**
 * Settings → Meetings: how calls are detected and started.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "MeetingSettings".
 */
export interface MeetingSettings {
  /**
   * Start recording when the alert's countdown ends, instead of waiting. Off by default.
   */
  autoStart: boolean;
  /**
   * Watch for calls at all (FR-MTG-2). On by default.
   */
  heuristicDetection: boolean;
  /**
   * Minutes before a calendar meeting the alert appears, 1–10 (FR-MTG-1).
   */
  leadMinutes: number;
  /**
   * Minutes before a calendar meeting its Meeting Prep brief is made, 5-120. Kept on this PC.
   */
  prepLeadMinutes: number;
}
/**
 * One entry of a routine's Model picker.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ModelOptionView".
 */
export interface ModelOptionView {
  label: string;
  /**
   * The `model_pref` value this entry sets.
   */
  value: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ModelStatus".
 */
export interface ModelStatus {
  dir: string;
  /**
   * All three files are on disk.
   */
  installed: boolean;
  /**
   * The running store has the model loaded (false until a restart after install).
   */
  loaded: boolean;
}
/**
 * The next meeting Today shows, with its brief when one exists.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "NextMeetingView".
 */
export interface NextMeetingView {
  brief?: PrepBriefView | null;
  eventId: string;
  startUnixMs: number;
  title: string;
}
/**
 * A prepared brief, flattened for the card. Claims keep the ids of the sources they rest on.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PrepBriefView".
 */
export interface PrepBriefView {
  attendees: PrepAttendee[];
  generatedAtUnixMs: number;
  openItems: PrepClaim[];
  openQuestions: PrepClaim[];
  sources: PrepSourceView[];
  summary: string;
  summarySources: string[];
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PrepAttendee".
 */
export interface PrepAttendee {
  note: string;
  sources: string[];
  who: string;
}
/**
 * An open item or question and the ids of the sources it rests on.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PrepClaim".
 */
export interface PrepClaim {
  sources: string[];
  text: string;
}
/**
 * A source the brief cites: the id the claims name ("M1", "E2") and what it was.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "PrepSourceView".
 */
export interface PrepSourceView {
  id: string;
  /**
   * The meeting on **this** PC the source is, when it provably is one (see `local_meeting`); the
   * chip opens it. Absent for an email, and for a meeting this PC cannot vouch for.
   */
  meetingId?: number | null;
  title: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProfileFields".
 */
export interface ProfileFields {
  about: string;
  name: string;
  organization: string;
  role: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProfileSwitches".
 */
export interface ProfileSwitches {
  enabled: boolean;
  suggestEnabled: boolean;
}
/**
 * Everything the pane shows.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ProfileView".
 */
export interface ProfileView {
  about: string;
  /**
   * The block's length before the cap, for the counter. Above `maxChars`, the rest is not sent.
   */
  composedChars: number;
  /**
   * "Use my profile in summaries and answers".
   */
  enabled: boolean;
  facts: Fact[];
  maxChars: number;
  maxFactChars: number;
  maxFacts: number;
  name: string;
  organization: string;
  /**
   * "What Whimbrel sees": exactly the block sent with a prompt. Empty when the switch is off.
   */
  preview: string;
  role: string;
  /**
   * "Suggest additions from my meetings".
   */
  suggestEnabled: boolean;
  /**
   * Whether the daily job would run now: the suggest switch is on and something is written.
   */
  suggestionsActive: boolean;
}
/**
 * What a sweep would remove right now, for the first-run notice. The shell's own shape
 * so it can cross to TypeScript; the numbers are core's.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RetentionPreview".
 */
export interface RetentionPreview {
  captures: number;
  conversations: number;
  meetings: number;
  orphanBytes: number;
  /**
   * Recordings no meeting claims any more, and what they take up.
   */
  orphans: number;
}
/**
 * What one sweep removed, for the Settings card.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RetentionRun".
 */
export interface RetentionRun {
  capturesDeleted: number;
  conversationsDeleted: number;
  finishedUnixMs: number;
  meetingsDeleted: number;
  /**
   * Recordings on disk that no meeting claimed any more.
   */
  orphansDeleted: number;
}
/**
 * Settings → Retention. `None` days means "keep forever", which is how core reads a
 * missing window too.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RetentionSettings".
 */
export interface RetentionSettings {
  captureDays?: number | null;
  conversationDays?: number | null;
  meetingDays?: number | null;
}
/**
 * The windows, what the last sweep removed, and whether the first-run notice is behind
 * us — until it is, the schedule counts but never deletes.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RetentionView".
 */
export interface RetentionView {
  acknowledged: boolean;
  lastRun?: RetentionRun | null;
  settings: RetentionSettings;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RoutineView".
 */
export interface RoutineView {
  /**
   * Whether a write routine acts without a click. Off unless the user turned it on.
   */
  autoApprove: boolean;
  /**
   * Whether this routine's runs call a model through its own preference (daily brief,
   * end-of-day recap, weekly digest). Reply Priorities calls none; Meeting Prep runs
   * through the standard chain.
   */
  callsModel: boolean;
  enabled: boolean;
  id: number;
  kind: RoutineKind;
  lastRun?: RunView | null;
  /**
   * Which model chain the routine's runs use: `default`, `cheapest`, or `<provider>/<model>`
   * (core `routine.model_pref`). Shown only where `calls_model` is true.
   */
  modelPref: string;
  name: string;
  /**
   * `HH:MM`, local time. Unused by Meeting Prep, which is event-driven.
   */
  schedule: string;
  /**
   * Why this routine cannot run on this PC yet; the tab shows it and will not let it be switched on.
   */
  unavailable?: string | null;
  /**
   * Whether this routine ends in something that needs approving (an event, an email).
   */
  writes: boolean;
}
/**
 * One run, for the history and for a routine's "last ran" line.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "RunView".
 */
export interface RunView {
  id: number;
  /**
   * The device that made this run, set only when it was **another** device's (the run synced
   * here); `None` for a run this PC made.
   */
  originDeviceId?: string | null;
  /**
   * That device as a person reads it, for "from <device>".
   */
  originLabel?: string | null;
  outcome: RunOutcome;
  ranAtUnixMs: number;
  routineId: number;
  routineName: string;
  /**
   * What the routine reported: the brief itself, or why it failed.
   */
  summary: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ScheduleEvent".
 */
export interface ScheduleEvent {
  attendees: string[];
  endUnixMs: number;
  eventId: string;
  /**
   * The local meeting recorded for this event, when there is one (recording now or done).
   */
  recordedMeetingId?: number | null;
  startUnixMs: number;
  title: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ScheduleView".
 */
export interface ScheduleView {
  /**
   * Meetings that started today and are not one of the events above, newest first.
   */
  earlierToday: MeetingRow[];
  /**
   * Today's events that are not over yet, soonest first.
   */
  events: ScheduleEvent[];
  /**
   * A meeting is recording now, so no row offers Start.
   */
  recording: boolean;
}
/**
 * A chat line from an image action, and whether it was stored in the skin's chat.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SkinLine".
 */
export interface SkinLine {
  stored: boolean;
  text: string;
}
/**
 * What the skin chat may use on this device. Chat follows Ask's rule on the preferred provider
 * (`TodayComposer.askAvailable` on the Mac, so local-only mode blocks a hosted one); search and
 * image generation are hosted, so local-only mode turns them off too. `provider` is the preferred
 * provider, which the chat sends to (the page names it in its status line).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SkinSources".
 */
export interface SkinSources {
  chat: boolean;
  /**
   * Making an image: needs an OpenAI or Google key.
   */
  generate: boolean;
  provider: Provider;
  /**
   * A web image search: needs the image search key.
   */
  search: boolean;
}
/**
 * What a turn did, for the page.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SkinTurnResult".
 */
export interface SkinTurnResult {
  /**
   * The skin whose chat the turn belongs to: the one saved, or the one asked about.
   */
  chatSkinId: string;
  /**
   * Images to make, likewise.
   */
  imagePrompts: string[];
  /**
   * Web searches to run for images, already limited to what this device may use and the request asked for.
   */
  imageQueries: string[];
  /**
   * `"new"`, `"refine"`, `"clarify"` or `"decline"`; `None` when the turn was stopped, so nothing changed.
   */
  intent?: string | null;
  /**
   * Whimbrel's chat lines for the turn, in order.
   */
  lines: string[];
  /**
   * The model's status lines, one per part of the skin.
   */
  notes: string[];
  /**
   * The skin saved, which the page switches to; `None` when nothing was saved.
   */
  savedSkinId?: string | null;
  /**
   * Whether the request and `lines` were stored in `chat_skin_id`'s chat.
   */
  stored: boolean;
}
/**
 * Settings → Meetings' snapshot rows. Local to this PC, never synced.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SnapshotSettings".
 */
export interface SnapshotSettings {
  /**
   * "Save slides and shared screens". On by default.
   */
  enabled: boolean;
  /**
   * Where new meetings' folders go.
   */
  folder: string;
  /**
   * No folder has been chosen, so `folder` is the default.
   */
  isDefault: boolean;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SnapshotView".
 */
export interface SnapshotView {
  capturedUnixMs: number;
  fileName: string;
  height: number;
  id: number;
  /**
   * The file's modification time: a build that replaces a slide can reuse an earlier
   * file name, so the strip keys its thumbnails on this too.
   */
  modifiedUnixMs: number;
  /**
   * The start of the slide's text.
   */
  text: string;
  width: number;
}
/**
 * One meeting's snapshots, oldest first, leaving out any whose file is gone.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SnapshotsView".
 */
export interface SnapshotsView {
  /**
   * "Keep snapshots": retention leaves this meeting's folder alone.
   */
  keep: boolean;
  snapshots: SnapshotView[];
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "StarterPromptView".
 */
export interface StarterPromptView {
  id: string;
  query: string;
  title: string;
}
/**
 * A fact the daily job proposed, waiting for Add or Dismiss.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SuggestionView".
 */
export interface SuggestionView {
  createdAtUnixMs: number;
  id: number;
  sourceMeetingId?: number | null;
  /**
   * The meeting it came from, when that meeting still exists.
   */
  sourceMeetingTitle?: string | null;
  text: string;
}
/**
 * What Settings → Sync shows. Never carries the Turso token or database URL.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "SyncView".
 */
export interface SyncView {
  /**
   * The last content-sync failure, cleared by the next success.
   */
  contentError?: string | null;
  /**
   * When content last synced successfully (Task 6), if it has this session.
   */
  contentLastSyncedMs?: number | null;
  deviceId: string;
  message?: string | null;
  state: SyncState;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "ThreadRow".
 */
export interface ThreadRow {
  id: number;
  lastActivityTsUnixMs: number;
  lastMessageFrom: string;
  /**
   * `waiting_on_me` or `waiting_on_them`.
   */
  status: string;
  subject?: string | null;
  summary?: string | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "Thumbnail".
 */
export interface Thumbnail {
  /**
   * A `data:image/png;base64,` URL, which the CSP's `img-src data:` allows.
   */
  dataUrl: string;
  height: number;
  width: number;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TodayOverview".
 */
export interface TodayOverview {
  /**
   * Today's Daily Brief, when one has run on any device. It is shown in place of the recap.
   */
  briefRecap?: BriefRecap | null;
  day: string;
  devices: DeviceCount[];
  latestMeeting?: MeetingRow | null;
  openLoops: ThreadRow[];
  /**
   * Today's cached recap; `None` until one is generated.
   */
  recap?: TodayRecap | null;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TodayRecap".
 */
export interface TodayRecap {
  /**
   * Served from the cache, so no model was called this time.
   */
  cached: boolean;
  /**
   * The local calendar day, `YYYY-MM-DD`: the digest cache's key.
   */
  day: string;
  /**
   * Nothing to summarize yet today: no model was called and nothing was cached.
   */
  emptyDay: boolean;
  openLoops: string[];
  summary: string;
}
/**
 * A meeting's transcript for display: turns from the file when it's on this device,
 * else from the stored segments.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TranscriptTurn".
 */
export interface TranscriptTurn {
  speaker: string;
  text: string;
  timestamp: string;
}
/**
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TranscriptView".
 */
export interface TranscriptView {
  /**
   * More segments remain beyond this page.
   */
  more: boolean;
  /**
   * "file" or "segments" — the segments are what a synced meeting has.
   */
  source: string;
  turns: TranscriptTurn[];
}
/**
 * The popup's view of the tray state.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "TrayStatus".
 */
export interface TrayStatus {
  captureEnabled: boolean;
  /**
   * "Capturing · Synced 2:05 PM", also the tray tooltip.
   */
  line: string;
  /**
   * When the recording meeting started, if one is recording.
   */
  recordingStartedUnixMs?: number | null;
}
/**
 * One call as the Model usage list shows it (core/api::usage::UsageRow).
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "UsageRowView".
 */
export interface UsageRowView {
  estimatedCostCents: number;
  inputTokens?: number | null;
  /**
   * A local call is not metered; the list says "local", never a zero.
   */
  local: boolean;
  model: string;
  outputTokens?: number | null;
  provider: string;
  task: string;
  tsUnixMs: number;
}
/**
 * One meeting's workspace: what to show, and which parts the user has edited.
 *
 * This interface was referenced by `WhimbrelDtos`'s JSON-Schema
 * via the `definition` "WorkspaceView".
 */
export interface WorkspaceView {
  actionItems: ActionItemView[];
  attendees: string[];
  /**
   * The meeting app or conference link, when the row has one.
   */
  conferenceLink?: string | null;
  decisions: string[];
  decisionsAreEdited: boolean;
  /**
   * Seconds recorded so far, while this process is the one recording.
   */
  elapsedSeconds?: number | null;
  hasTranscript: boolean;
  openQuestions: string[];
  openQuestionsAreEdited: boolean;
  personalNotes: string;
  phase: WorkspacePhase;
  /**
   * "Transcribing…" and friends, while the pipeline is on this meeting.
   */
  processingStage?: string | null;
  /**
   * A meeting recorded on another device: its audio isn't here, so it can't be
   * re-summarized, and Forget would only remove this device's copy.
   */
  readOnly: boolean;
  row: MeetingRow;
  summary?: string | null;
  summaryIsEdited: boolean;
  trigger: string;
}
