// Typed wrappers over the shell's Tauri commands (src-tauri/src/commands).

import { Channel, invoke } from "@tauri-apps/api/core";
import type { AudioLevel, DeleteEverythingPreview, MeetingMarkerView, NoteSourcesView, TranscriptLinesView } from "./generated/types";
import type {
  ClientControl,
  ClientDeleteReceipt,
  ClientDue,
  ClientExpiryReceipt,
  ClientExportDone,
  ClientInventory,
  ClientItem,
  ClientItemRef,
  ClientPendingDevices,
  ClientPendingDeviceIds,
  ClientResult,
  ClientResummaryOffer,
  ClientControlChange,
  ClientControlUpdate,
  ClientRetentionDecision,
  ClientReviewFlag,
} from "./clientControls";
import type {
  RecapDraft,
  RecapMailto,
  RecapSentView,
  RecapSummaryInput,
  AiSettings,
  BudgetNoticeView,
  ProviderBudgetView,
  LiveAlertView,
  LiveTranscriptView,
  OpenMeetingRequest,
  QuestionNotice,
  SeedInput,
  TopicSeedView,
  TopicSortEstimate,
  TopicNamingView,
  TopicSortView,
  TopicSuggestionView,
  UpdateCheck,
  ChatLine,
  ImageCandidateView,
  AppStatus,
  CaptureDetail,
  CaptureSettings,
  CaptureStatus,
  ChatEvent,
  HotkeyPanelRead,
  HotkeyStatus,
  WindowAskReply,
  ChatSpend,
  ConversationRow,
  ConversationView,
  SnapshotSettings,
  SnapshotsView,
  StarterPromptView,
  SuggestionView,
  Thumbnail,
  TodayOverview,
  TodayRecap,
  DeviceView,
  DownloadView,
  EpisodeRow,
  LibraryRow,
  LibrarySettings,
  FeedbackResult,
  MeetingDetail,
  MeetingModelsStatus,
  QualityChoice,
  TranscriptionQualityStatus,
  MeetingRecordingView,
  MeetingRow,
  ModelStatus,
  ActionItemView,
  VoicePersonView,
  AlertView,
  CalendarAccounts,
  ProfileFields,
  ProfileSwitches,
  ProfileView,
  RetentionPreview,
  ApprovalView,
  ApproveResult,
  GmailAccounts,
  GoogleClientStatus,
  McpClientView,
  McpNewClient,
  McpServerView,
  HostView,
  RoutineView,
  KeyCheckView,
  UsageRowView,
  ModelOptionView,
  RunView,
  RetentionSettings,
  RetentionView,
  MeetingSettings,
  LocalModelStatus,
  ImportView,
  RecordingNoticeSettings,
  NextMeetingView,
  MeetingChipsView,
  TranscriptView,
  TrimInfo,
  WorkspaceView,
  PromptSurface,
  Provider,
  ProviderKey,
  RoutingSettings,
  ScheduleView,
  StatusView,
  StorageView,
  LowDisk,
  CompactView,
  FreeUpOptions,
  CompressView,
  SyncView,
  WhimbrelAiView,
  TrayStatus,
  GeneralSettings,
  LanguageSettings,
  SkinLine,
  SkinSources,
  SkinTurnResult,
  TopicListItemView,
  VaultView,
  UpcomingView,
  TopicPageView,
  StepActionInput,
  TopicLookup,
  TopicRefView,
  TopicNamesView,
  TopicStepsView,
  MeetingAskExchange,
  LaunchView,
  LaunchAnswer,
  ChosenFile,
  UseKeyResult,
  SetupScope,
  SetupBegun,
  SaveFileResult,
  EncryptionCardView,
  SetupRefusal,
  EncryptOutcome,
  NoteAction,
  NoteActionResult,
  ForeignFilesDeleted,
  ForeignFilesView,
  SpeedCheckResult,
  PlusAiPromptAnswer,
  PlusAiPromptView,
} from "./generated/types";
import type { SkinDto } from "./skin";
import type { CloudRoutinesView, RunnerRegistration } from "./automations/cloudRoutineModel";

/** A raw-bytes reply as real bytes. The shell's `Response` can arrive as a plain array of numbers rather than an
 *  ArrayBuffer, and `new Blob([array])` would then hold the numbers' text, which no image decodes. */
export function asBytes(reply: ArrayBuffer | ArrayLike<number>): Uint8Array {
  return reply instanceof ArrayBuffer ? new Uint8Array(reply) : Uint8Array.from(reply);
}

export const appStatus = () => invoke<AppStatus>("app_status");
export const getRoutingSettings = () => invoke<RoutingSettings>("get_routing_settings");
export const setRoutingSettings = (routing: RoutingSettings) => invoke<void>("set_routing_settings", { routing });
export const captureStatus = () => invoke<CaptureStatus>("capture_status");
export const setCaptureEnabled = (enabled: boolean) => invoke<CaptureStatus>("set_capture_enabled", { enabled });
/** A timed pause: capture reads nothing until this wall-clock time (unix ms), then is back to on. Redesign only. */
export const capturePauseUntil = (resumeAtUnixMs: number) => invoke<CaptureStatus>("capture_pause_until", { resumeAtUnixMs });
/** Ends a timed pause early. */
export const captureResumeNow = () => invoke<CaptureStatus>("capture_resume_now");

// The tray's quick-glance popup (Task 10).
export const trayStatus = () => invoke<TrayStatus>("tray_status");
export const popupOpenMain = () => invoke<void>("popup_open_main");
export const popupQuit = () => invoke<void>("popup_quit");

// Auto-update (M3.5): the version of a downloaded update, and "Restart now".
export const updateReady = () => invoke<string | null>("update_ready");
/** This x64 build is running under emulation on an Arm PC (`ArmNotice`). */
export const runningEmulated = () => invoke<boolean>("running_emulated");
export const installUpdate = () => invoke<void>("install_update");
/** Settings → General's "Check for Updates": starts a check and returns at once. */
export const checkForUpdates = () => invoke<UpdateCheck>("check_for_updates");
export const updateCheckStatus = () => invoke<UpdateCheck>("update_check_status");

export const generalSettings = () => invoke<GeneralSettings>("general_settings");
export const setAutostart = (enabled: boolean) => invoke<GeneralSettings>("set_autostart", { enabled });

/** Settings → General's language pickers. A change applies at once; re-render in `uiLanguage`. */
export const languageSettings = () => invoke<LanguageSettings>("language_settings");
export const setLanguage = (choice: string) => invoke<LanguageSettings>("set_language", { choice });
export const setWriteIn = (tag: string) => invoke<LanguageSettings>("set_write_in", { tag });

export const libraryRecent = () => invoke<LibraryRow[]>("library_recent");
export const libraryEpisodes = (includeHidden: boolean) => invoke<LibraryRow[]>("library_episodes", { includeHidden });
export const librarySearch = (query: string, semantic: boolean) => invoke<LibraryRow[]>("library_search", { query, semantic });
/** Captures matching a query, from one app and inside a time range (the Home filters). The core types are not in
 *  the generated schema; they are written here as the shell serializes them (snake_case, from core/memory). */
export const librarySearchFiltered = (query: string, semantic: boolean, app: string | null, startUnixMs: number | null, endUnixMs: number | null) =>
  invoke<LibraryRow[]>("library_search_filtered", { query, semantic, app, startUnixMs, endUnixMs });
export interface CaptureApp {
  app_bundle_id: string;
  captures: number;
}
export const libraryCaptureApps = (startUnixMs: number, endUnixMs: number, limit: number) =>
  invoke<CaptureApp[]>("library_capture_apps", { startUnixMs, endUnixMs, limit });
export interface DayBlockApp {
  app_bundle_id: string;
  /** Window titles, at most three. */
  items: string[];
  /** Titles left out. */
  more_items: number;
}
export interface DayBlock {
  start_unix_ms: number;
  end_unix_ms: number;
  /** At most four. */
  apps: DayBlockApp[];
  /** Apps left out. */
  more_apps: number;
}
/** Memory blocks for `[startUnixMs, endUnixMs)`, cut around the meeting `[start, end]` pairs. */
export const libraryDayBlocks = (startUnixMs: number, endUnixMs: number, meetings: [number, number][]) =>
  invoke<DayBlock[]>("library_day_blocks", { startUnixMs, endUnixMs, meetings });
export const libraryCapture = (id: number) => invoke<CaptureDetail | null>("library_capture", { id });
export const libraryMeeting = (id: number) => invoke<MeetingDetail | null>("library_meeting", { id });
export const libraryEpisode = (id: number) => invoke<EpisodeRow | null>("library_episode", { id });
export const librarySettings = () => invoke<LibrarySettings>("library_settings");
export const setShowScreenCaptures = (show: boolean) => invoke<LibrarySettings>("set_show_screen_captures", { show });
export const setEpisodeAppHidden = (app: string, hidden: boolean) => invoke<LibrarySettings>("set_episode_app_hidden", { app, hidden });

export const syncStatus = () => invoke<SyncView>("sync_status");
export const syncSignIn = () => invoke<SyncView>("sync_sign_in");
export const syncCancelSignIn = () => invoke<SyncView>("sync_cancel_sign_in");
export const syncCheckAgain = () => invoke<SyncView>("sync_check_again");
// Cloud v1: the plan, and Stripe's checkout and portal (opened in the browser by the shell).
export const syncRefreshPlan = () => invoke<SyncView>("sync_refresh_plan");
export const syncUpgrade = (interval: "month" | "year") => invoke<SyncView>("sync_upgrade", { interval });
export const syncManageSubscription = () => invoke<SyncView>("sync_manage_subscription");
export const whimbrelAiStatus = () => invoke<WhimbrelAiView>("whimbrel_ai_status");
export const whimbrelAiRefresh = () => invoke<WhimbrelAiView>("whimbrel_ai_refresh");
export const whimbrelAiBuy = (pack: "10" | "25") => invoke<WhimbrelAiView>("whimbrel_ai_buy", { pack });
/** Plus: whether to ask "Use Whimbrel AI?" now (the shell's `plus_prompt::check` decides). */
export const plusAiPrompt = () => invoke<PlusAiPromptView>("plus_ai_prompt");
/** The answer; false when the question no longer applied and nothing was written. */
export const plusAiPromptAnswer = (answer: PlusAiPromptAnswer) => invoke<boolean>("plus_ai_prompt_answer", { answer });
/** The question is on screen: closing the window from now on is Not now. */
export const plusAiPromptDisplayed = () => invoke<void>("plus_ai_prompt_displayed");
export const syncDevices = () => invoke<DeviceView[]>("sync_devices");
export const syncSetDeviceName = (name: string) => invoke<DeviceView[]>("sync_set_device_name", { name });
export const syncApproveDevice = (id: string) => invoke<DeviceView[]>("sync_approve_device", { id });
export const syncRevokeDevice = (id: string) => invoke<DeviceView[]>("sync_revoke_device", { id });
/** "Add Whimbrel Cloud": the outcome, with the values for the runner's host on success (shown once, never kept). */
export const syncRegisterRunner = (publicKey: string) => invoke<RunnerRegistration>("sync_register_runner", { publicKey });

export const aiSettings = () => invoke<AiSettings>("ai_settings");
/** A paid provider's monthly budget in dollars; `null` removes it. */
export const setProviderBudget = (provider: Provider, dollars: number | null) =>
  invoke<ProviderBudgetView[]>("set_provider_budget", { provider, dollars });
/** "Open AI settings": the main window shows Settings at AI. */
export const openAiSettings = () => invoke<void>("open_ai_settings");
/** Opens a new email to Whimbrel with the version and Windows build filled in. */
export const sendFeedback = () => invoke<FeedbackResult>("send_feedback");
export const perfReportText = () => invoke<string>("perf_report_text");
export const setApiKey = (provider: Provider, key: string | null) => invoke<ProviderKey[]>("set_api_key", { provider, key });
/** Turns a provider's key sync on or off on this PC; resolves to the keys. */
export const setApiKeySync = (provider: Provider, enabled: boolean) => invoke<ProviderKey[]>("set_api_key_sync", { provider, enabled });
/** Saves (or, with null, removes) the skin maker's Brave Search key; resolves to it masked. */
export const setImageSearchKey = (key: string | null) => invoke<string | null>("set_image_search_key", { key });
// Topics (port of PR 631): the TypeSafe key and sorting switch in Settings → AI, and Settings → Topics.
export const topicSortState = () => invoke<TopicSortView>("topic_sort_state");
/** Saves (or, with null, removes) the TypeSafe key; it stays on this PC. */
export const setTypesafeKey = (key: string | null) => invoke<TopicSortView>("set_typesafe_key", { key });
export const setTopicSortEnabled = (enabled: boolean) => invoke<TopicSortView>("set_topic_sort_enabled", { enabled });
export const topicSortEstimate = () => invoke<TopicSortEstimate>("topic_sort_estimate");
/** Start on the estimate: allows judging up to 1.5 × `tokens`. */
export const startTopicSort = (tokens: number) => invoke<TopicSortView>("start_topic_sort", { tokens });
export const topicSeeds = () => invoke<TopicSeedView[]>("topic_seeds");
export const saveTopicSeed = (seed: SeedInput) => invoke<TopicSeedView>("save_topic_seed", { seed });
export const deleteTopicSeed = (key: string) => invoke<void>("delete_topic_seed", { key });
export const mergeTopicSeeds = (keep: string, absorb: string) => invoke<void>("merge_topic_seeds", { keep, absorb });
export const topicNamingStatus = () => invoke<TopicNamingView>("topic_naming_status");
export const topicSuggestions = () => invoke<TopicSuggestionView[]>("topic_suggestions");
export const acceptTopicSuggestion = (topicId: number, name: string) => invoke<TopicSeedView>("accept_topic_suggestion", { topicId, name });
export const dismissTopicSuggestion = (topicId: number) => invoke<void>("dismiss_topic_suggestion", { topicId });
// Client controls (X6): each resolves to core's envelope, `{ ok: true, data }` or `{ ok: false, error: { kind } }`.
// The shell supplies the media and vault folders, this PC's device id and the account key.
/** Everything about a client: linked items, possible matches (offer them unticked), and what is left for the user. */
export const clientInventory = (key: string) => invoke<ClientResult<ClientInventory>>("client_inventory", { key });
/** Deletes everything about a client. A ticked item a fresh inventory no longer lists is `selection_changed`
 *  (reload the review); unfinished background work is `busy` (try again shortly); both touch nothing. Slow; the
 *  job loops pause while it runs. */
/** Waits for running background work as long as it takes (until `clientCancelWait(token)`), then deletes.
 *  `reviewedLinked`: the linked items the review showed; a different set now answers `selection_changed`. */
export const clientDelete = (selection: {
  key: string;
  includePossible: ClientItemRef[];
  deleteExports: boolean;
  reviewedLinked: ClientItemRef[];
  token: number;
}) => invoke<ClientResult<ClientDeleteReceipt>>("client_delete", { selection });
/** Ends a delete's wait for background work. False: too late, the delete has started. */
export const clientCancelWait = (token: number) => invoke<boolean>("client_cancel_wait", { token });
/** Background ticks running now (what a waiting delete waits for). */
export const clientJobsRunning = () => invoke<number>("client_jobs_running");
/** Devices that still need to update before they apply client deletes (null: not known yet). */
export const clientPendingDevices = () => invoke<ClientPendingDevices>("client_pending_devices");
/** The sync device ids (public keys) of the devices `clientPendingDevices` counts (null: not known yet). */
export const clientPendingDeviceIds = () => invoke<ClientPendingDeviceIds>("client_pending_device_ids");
/** "Remove this device": stops counting a retired device (one of `clientPendingDeviceIds`). Releases nothing. */
export const clientRetireDevice = (deviceId: string) => invoke<ClientResult<ClientPendingDevices>>("client_retire_device", { deviceId });
/** Exports everything about a client into a new folder inside a folder the user picks in the shell's own dialog;
 *  media only when asked for. `data` is null when the pick was cancelled; a network share is `folder_not_allowed`. */
export const clientExport = (key: string, includePossible: ClientItemRef[], includeMedia: boolean) =>
  invoke<ClientResult<ClientExportDone | null>>("client_export", { key, includePossible, includeMedia });
export const clientControlGet = (key: string) => invoke<ClientResult<ClientControl>>("client_control_get", { key });
/** Changes one setting on the value stored now (read, change and write in one job on the store), so a value another
 *  device synced since the page loaded is kept. */
export const clientControlUpdate = (key: string, change: ClientControlChange) =>
  invoke<ClientResult<ClientControlUpdate>>("client_control_update", { key, change });
/** Meetings a hosted model summarised, to offer summarising again here, and core's "already left" sentence. */
export const clientResummaryOffer = (key: string) => invoke<ClientResult<ClientResummaryOffer>>("client_resummary_offer", { key });
/** Every client's items past its keep period; with `key`, that client's items. Read only. */
export function clientDueItems(): Promise<ClientResult<ClientDue[]>>;
export function clientDueItems(key: string): Promise<ClientResult<ClientItem[]>>;
export function clientDueItems(key?: string) {
  return invoke("client_due_items", { key: key ?? null });
}
/** "Keep these" or "Snooze" for one item, on this PC only. */
export const clientRetentionDecide = (key: string, item: ClientItemRef, decision: ClientRetentionDecision) =>
  invoke<ClientResult<null>>("client_retention_decide", { key, item, decision });
/** The weekly "items ready to review" flag, or null. */
export const clientReviewFlag = () => invoke<ClientResult<ClientReviewFlag | null>>("client_review_flag");
/** "Later": puts the review prompt off for a week. */
export const clientReviewFlagDismiss = () => invoke<ClientResult<null>>("client_review_flag_dismiss");
/** Deletes confirmed due items on this PC only, for at most `budgetMs`; call again while `data.more`. */
export const clientExpire = (key: string, items: ClientItemRef[], budgetMs: number) =>
  invoke<ClientResult<ClientExpiryReceipt>>("client_expire", { key, items, budgetMs });
// The topic page (topics piece 2): the Topics tab, step actions, the ways in, and Settings → Topics.
/** Today's Upcoming: the next 7 days' meetings, grouped by day. */
export const todayUpcoming = () => invoke<UpcomingView>("today_upcoming");
// The living vault (Settings → Capture & Privacy): the switch, the folder and what it last did.
export const vaultSettings = () => invoke<VaultView>("vault_settings");
export const setVaultEnabled = (enabled: boolean) => invoke<VaultView>("set_vault_enabled", { enabled });
/** The native folder picker; unchanged when cancelled. */
export const chooseVaultFolder = () => invoke<VaultView>("choose_vault_folder");
export const topicsPageList = () => invoke<TopicListItemView[]>("topics_page_list");
/** Null when the topic is gone (deleted, or merged away). */
export const topicPage = (topicId: number) => invoke<TopicPageView | null>("topic_page", { topicId });
/** False when the step no longer exists. */
export const topicStepAction = (key: string, action: StepActionInput) => invoke<boolean>("topic_step_action", { key, action });
/** The user's wording of their role; blank goes back to the inferred one. */
export const topicRoleSet = (topicKey: string, text: string) => invoke<void>("topic_role_set", { topicKey, text });
/** Each item's topic, in order, in one call. */
export const topicLookup = (items: TopicLookup[]) => invoke<(TopicRefView | null)[]>("topic_lookup", { items });
export const topicsNamedIn = (text: string) => invoke<TopicRefView[]>("topics_named_in", { text });
export const topicNames = () => invoke<TopicNamesView>("topic_names");
export const setTopicNames = (names: string[]) => invoke<TopicNamesView>("set_topic_names", { names });
export const topicStepsState = () => invoke<TopicStepsView>("topic_steps_state");
export const setTopicStepsEnabled = (enabled: boolean) => invoke<TopicStepsView>("set_topic_steps_enabled", { enabled });
export const topicStepsEstimate = () => invoke<TopicSortEstimate>("topic_steps_estimate");
/** Start on the estimate: allows TypeSafe up to 1.5 × `tokens`. */
export const startTopicSteps = (tokens: number) => invoke<TopicStepsView>("start_topic_steps", { tokens });
export const setPrompt = (surface: PromptSurface, text: string) => invoke<void>("set_prompt", { surface, text });
export const setSummaryInstructions = (text: string) => invoke<void>("set_summary_instructions", { text });
export const providerModels = (provider: Provider) => invoke<string[]>("provider_models", { provider });
// The key on-ramp and Model usage (plan part 5, cheap-first routing).
export const keyCheck = (provider: Provider) => invoke<KeyCheckView>("key_check", { provider });
export const keyFormatHint = (provider: Provider, key: string) => invoke<string | null>("key_format_hint", { provider, key });
export const usageRows = (days?: number) => invoke<UsageRowView[]>("usage_rows", { days });
/** Opens an https URL in the default browser (a provider's key page or data policy). */
export const openExternal = (url: string) => invoke<void>("open_external", { url });

export const captureSettings = () => invoke<CaptureSettings>("capture_settings");
export const setExcludedApps = (excluded: string[]) => invoke<string[]>("set_excluded_apps", { excluded });

export const modelStatus = () => invoke<ModelStatus>("model_status");
/** "Skip for now" on the launch prompt to download the search model. */
export const modelDownloadStart = () => invoke<string>("model_download_start");
export const modelDownloadPoll = (jobId: string) => invoke<DownloadView>("model_download_poll", { jobId });
export const modelDownloadCancel = (jobId: string) => invoke<boolean>("model_download_cancel", { jobId });
export const restartApp = () => invoke<void>("restart_app");
/** Settings > Capture > Delete everything (X4 Task 12): what the confirmation lists. */
export const deleteEverythingPreview = () => invoke<DeleteEverythingPreview>("delete_everything_preview");
/** Confirmed: the sync account if chosen, the Google sign-ins, the slide images, then a restart into the delete. The shell checks `confirm` is DELETE. The app exits on success. */
export const deleteEverythingNow = (args: { confirm: string; alsoDeleteSyncAccount: boolean; slidesLeftAlone: boolean }) =>
  invoke<void>("delete_everything_now", args);
export const meetingModelsStatus = () => invoke<MeetingModelsStatus>("meeting_models_status");
export const meetingModelsDownloadStart = () => invoke<string>("meeting_models_download_start");
/** "I have meetings in other languages": on adds the multilingual model to the missing list. */
export const setMeetingMultilingual = (on: boolean) => invoke<MeetingModelsStatus>("set_meeting_multilingual", { on });
/** Better transcripts (W-1.2): the Quality picker, the one offer, large-v3-turbo and the benchmark. */
export const transcriptionQualityStatus = () => invoke<TranscriptionQualityStatus>("transcription_quality_status");
export const setTranscriptionQuality = (quality: QualityChoice) => invoke<TranscriptionQualityStatus>("set_transcription_quality", { quality });
export const dismissBetterOffer = () => invoke<TranscriptionQualityStatus>("dismiss_better_offer");
export const turboDownloadStart = () => invoke<string>("turbo_download_start");
export const transcriptionBenchmark = () => invoke<TranscriptionQualityStatus>("transcription_benchmark");

export const meetingRecordStart = () => invoke<MeetingRecordingView>("meeting_record_start");
export const meetingRecordStop = () => invoke<MeetingRecordingView>("meeting_record_stop");
// Recording pause (redesign only): audio and snapshots stop until resume; the state stays "recording".
export const meetingRecordPause = () => invoke<MeetingRecordingView>("meeting_record_pause");
export const meetingRecordResume = () => invoke<MeetingRecordingView>("meeting_record_resume");
// The level meter's reading (redesign only, polled about four times a second while recording): read, drawn, thrown away.
// Marked moments (redesign only): the time is centiseconds from the start of the recording on the transcript's clock.
export const meetingMarkerAdd = (meetingId: number, atCs: number, label: string | null) => invoke<MeetingMarkerView>("meeting_marker_add", { meetingId, atCs, label });
export const meetingMarkerList = (meetingId: number) => invoke<MeetingMarkerView[]>("meeting_marker_list", { meetingId });
export const meetingMarkerDelete = (id: number) => invoke<boolean>("meeting_marker_delete", { id });
/** A mark's label changed, or cleared with null; false when there is no such mark (finished note, redesign only). */
export const meetingMarkerSetLabel = (id: number, label: string | null) => invoke<boolean>("meeting_marker_set_label", { id, label });
// A question typed in the in-meeting window: parked in the shell until the main window takes it (once), or cancelled.
export const meetingBarAsk = (meetingId: number, text: string) => invoke<void>("meeting_bar_ask", { meetingId, text });
export const meetingBarTakeAsk = (meetingId: number) => invoke<string | null>("meeting_bar_take_ask", { meetingId });
export const meetingBarCancelAsk = () => invoke<void>("meeting_bar_cancel_ask");
export const meetingAudioLevel = () => invoke<AudioLevel>("meeting_audio_level");
export const todaySchedule = () => invoke<ScheduleView>("today_schedule");

// Skins (core/skins through src-tauri/src/commands/skins.rs). `SkinDto` is typed by hand in skin.ts.
export const skinsList = () => invoke<SkinDto[]>("skins_list");
/** The hidden redesign switch (`redesign.enabled` in settings.json, off unless set by hand); see `redesign.ts`. */
export const redesignEnabledCommand = () => invoke<boolean>("redesign_enabled");
/** Moves when a sync changed the skins folder (Task 15). */
export const skinsRevision = () => invoke<number>("skins_revision");
/** An approved image of one of the user's skins, as raw bytes. */
export const skinImage = (id: string, name: string) => invoke<ArrayBuffer | number[]>("skin_image", { id, name }).then(asBytes);

// Making skins by chat (Task 10; the decisions are in src-tauri/src/skin_maker.rs).
export const skinSources = () => invoke<SkinSources>("skin_sources");
/** One turn on `skinId`. `runId` is the caller's, for `skinTurnStop`. */
export const skinTurn = (skinId: string, request: string, runId: string) =>
  invoke<SkinTurnResult>("skin_turn", { skinId, request, runId });
/** Makes a running turn's late reply change nothing; false when it isn't running. */
export const skinTurnStop = (runId: string) => invoke<boolean>("skin_turn_stop", { runId });
/** The skin's chat as stored (Task 11); empty for a built-in. */
export const skinChatLoad = (skinId: string) => invoke<ChatLine[]>("skin_chat_load", { skinId });
/** A Whimbrel line for the skin's chat, kept when the skin is yours. */
export const skinChatPost = (skinId: string, text: string) => invoke<SkinLine>("skin_chat_post", { skinId, text });
export const skinUndo = (skinId: string) => invoke<SkinLine>("skin_undo", { skinId });
// Rename, delete, export and import (Task 12). The pickers open in the shell; a refusal is a CommandError
// whose message is the line to show.
export const skinRename = (skinId: string, name: string) => invoke<void>("skin_rename", { skinId, name });
export const skinDelete = (skinId: string) => invoke<void>("skin_delete", { skinId });
/** False when the save picker is cancelled. */
export const skinExport = (skinId: string) => invoke<boolean>("skin_export", { skinId });
/** The new skin's id, or null when the picker is cancelled. */
export const skinImport = () => invoke<string | null>("skin_import");
export const skinImageSearch = (query: string) => invoke<ImageCandidateView[]>("skin_image_search", { query });
export const skinImageGenerate = (prompt: string) => invoke<ImageCandidateView>("skin_image_generate", { prompt });
/** A candidate's thumbnail as raw bytes, fetched by the shell (the page may not load remote images). */
export const skinImageThumbnail = (url: string) => invoke<ArrayBuffer | number[]>("skin_image_thumbnail", { url }).then(asBytes);
export const skinImageReject = (url: string) => invoke<void>("skin_image_reject", { url });
/** Logs why a thumbnail that arrived couldn't be shown (the image then leaves the strip). */
export const skinImageUnshown = (reason: string) => invoke<void>("skin_image_unshown", { reason });
export const skinImageApprove = (skinId: string, url: string) => invoke<SkinLine>("skin_image_approve", { skinId, url });
/** Opens a file picker; null when it is cancelled. */
export const skinImageAddFile = (skinId: string) => invoke<SkinLine | null>("skin_image_add_file", { skinId });
export const meetingRecordStartEvent = (eventId: string) => invoke<MeetingRecordingView>("meeting_record_start_event", { eventId });
export const meetingRecordingStatus = () => invoke<MeetingRecordingView>("meeting_recording_status");

// The Meetings tab (M3.3 Task 5).
/** The small local model behind Ask (W-1.3). */
export const localModelStatus = () => invoke<LocalModelStatus>("local_model_status");
export const localModelDownloadStart = () => invoke<string>("local_model_download_start");
export const localModelRemove = () => invoke<LocalModelStatus>("local_model_remove");
/** Newest first: `limit` rows (200 when left out); a non-empty `filter` searches every meeting. */
export const meetingList = (filter: string, limit?: number) => invoke<MeetingRow[]>("meeting_list", { filter, limit });
/** How many meetings the store holds, for the list's "Showing X of Y meetings". */
export const meetingTotal = () => invoke<number>("meeting_total");
/** Import (W-1.6): each opens a picker and resolves null when it's cancelled. */
export const importTranscripts = () => invoke<ImportView | null>("import_transcripts");
export const importMarkdownFolder = () => invoke<ImportView | null>("import_markdown_folder");
export const meetingWorkspace = (id: number) => invoke<WorkspaceView | null>("meeting_workspace", { id });
/** "Re-summarize with Whimbrel" (#805): Whimbrel AI only, no fallback; fails with why. */
export const meetingResummarizeWhimbrel = (id: number, keepEdits: boolean) => invoke<void>("meeting_resummarize_whimbrel", { id, keepEdits });
// Meeting recap (core #815): Copy as text, Markdown or Slack, and Send recap.
export type RecapFormat = "plain" | "markdown" | "slack" | "email";
export const recapText = (summary: RecapSummaryInput, format: RecapFormat) => invoke<string>("recap_text", { summary, format });
export const recapDraft = (meetingId: number, summary: RecapSummaryInput) => invoke<RecapDraft>("recap_draft", { meetingId, summary });
export const recapSend = (meetingId: number, to: string[], subject: string, body: string) =>
  invoke<RecapSentView>("recap_send", { meetingId, to, subject, body });
export const recapMailto = (to: string[], subject: string, body: string) => invoke<RecapMailto>("recap_mailto", { to, subject, body });
export const recapOpenMail = (url: string) => invoke<void>("recap_open_mail", { url });
export const recapSent = (meetingId: number) => invoke<RecapSentView | null>("recap_sent", { meetingId });
export const meetingTranscript = (id: number) => invoke<TranscriptView>("meeting_transcript", { id });
/** The stored transcript lines with ids and numeric times, a page at a time, unmerged (finished note, redesign only). */
export const meetingTranscriptLines = (id: number, offset: number, limit: number) => invoke<TranscriptLinesView>("meeting_transcript_lines", { id, offset, limit });
/** The finished note's lines with the sources that still hold, owner and due (redesign only). */
export const meetingNoteSources = (id: number) => invoke<NoteSourcesView | null>("meeting_note_sources", { id });
/** Speaker names: an invitee or a typed name for one of the meeting's speakers, confirmed. */
export const setMeetingSpeaker = (id: number, speakerId: number, name: string, email: string | null) =>
  invoke<TranscriptView>("set_meeting_speaker", { id, speakerId, name, email });
/** "Yes, that's {name}": confirms a guess as the person it was shown as. */
export const confirmMeetingSpeaker = (id: number, speakerId: number, personKey: string) =>
  invoke<TranscriptView>("confirm_meeting_speaker", { id, speakerId, personKey });
/** People with a remembered voice (Settings → Meetings). */
export const voicePeople = () => invoke<VoicePersonView[]>("voice_people");
export const forgetVoice = (personKey: string) => invoke<VoicePersonView[]>("forget_voice", { personKey });
export const meetingSaveNotes = (id: number, notes: string) => invoke<void>("meeting_save_notes", { id, notes });
export const meetingSaveOverrides = (
  id: number,
  summary: string | null,
  decisions: string[] | null,
  openQuestions: string[] | null,
) => invoke<void>("meeting_save_overrides", { id, summary, decisions, openQuestions });
export const meetingSeedActionItems = (id: number) => invoke<ActionItemView[]>("meeting_seed_action_items", { id });
export const meetingAddActionItem = (id: number, text: string) => invoke<ActionItemView[]>("meeting_add_action_item", { id, text });
export const meetingUpdateActionItem = (id: number, itemId: number, text: string | null, completed: boolean | null) =>
  invoke<ActionItemView[]>("meeting_update_action_item", { id, itemId, text, completed });
export const meetingDeleteActionItem = (id: number, itemId: number) =>
  invoke<ActionItemView[]>("meeting_delete_action_item", { id, itemId });
export const meetingReorderActionItems = (id: number, orderedIds: number[]) =>
  invoke<ActionItemView[]>("meeting_reorder_action_items", { id, orderedIds });
export const meetingResetOverrides = (id: number) => invoke<void>("meeting_reset_overrides", { id });
export const meetingRegenerate = (id: number, keepEdits: boolean) => invoke<void>("meeting_regenerate", { id, keepEdits });
/** Whether "Trim recording…" is offered, and the meeting's length on the transcript clock. */
export const meetingTrimInfo = (id: number) => invoke<TrimInfo>("meeting_trim_info", { id });
/** Keeps the first `keepMs`; false is "Nothing to trim." */
export const meetingTrim = (id: number, keepMs: number) => invoke<boolean>("meeting_trim", { id, keepMs });
/** The per-meeting "Transcript language": stores it (`null` is Automatic) and transcribes the meeting again. */
export const meetingSetSpokenLanguage = (id: number, language: string | null) =>
  invoke<void>("meeting_set_spoken_language", { id, language });
/** Transcribe again from a meeting's "Transcription failed" or "No speech detected" notice (the Mac's #530). */
export const meetingTranscribeAgain = (id: number) => invoke<void>("meeting_transcribe_again", { id });
export const meetingRegenerationConflict = (id: number) => invoke<boolean>("meeting_regeneration_conflict", { id });
export const meetingForget = (id: number) => invoke<void>("meeting_forget", { id });
export const meetingMostRelevant = () => invoke<number | null>("meeting_most_relevant");

// Meeting snapshots: the workspace's Slides strip and Settings → Meetings (plan Task 7).
export const meetingSnapshots = (meetingId: number) => invoke<SnapshotsView>("meeting_snapshots", { meetingId });
export const meetingSnapshotThumbnail = (meetingId: number, id: number) =>
  invoke<Thumbnail>("meeting_snapshot_thumbnail", { meetingId, id });
export const meetingSnapshotDelete = (meetingId: number, id: number) => invoke<void>("meeting_snapshot_delete", { meetingId, id });
export const meetingSetKeepSnapshots = (meetingId: number, keep: boolean) =>
  invoke<void>("meeting_set_keep_snapshots", { meetingId, keep });
export const meetingSnapshotOpen = (meetingId: number, id: number) => invoke<void>("meeting_snapshot_open", { meetingId, id });
export const meetingSnapshotCopy = (meetingId: number, id: number) => invoke<void>("meeting_snapshot_copy", { meetingId, id });
export const meetingSnapshotsShow = (meetingId: number, id: number | null) => invoke<void>("meeting_snapshots_show", { meetingId, id });
export const meetingSnapshotSettings = () => invoke<SnapshotSettings>("meeting_snapshot_settings");
export const setMeetingSnapshotsEnabled = (enabled: boolean) => invoke<SnapshotSettings>("set_meeting_snapshots_enabled", { enabled });
export const chooseMeetingSnapshotsFolder = () => invoke<SnapshotSettings>("choose_meeting_snapshots_folder");
export const resetMeetingSnapshotsFolder = () => invoke<SnapshotSettings>("reset_meeting_snapshots_folder");

// Detection, its alert window, and Settings → Meetings (M3.3 Task 6).
export const meetingSettings = () => invoke<MeetingSettings>("meeting_settings");
export const setMeetingSettings = (settings: MeetingSettings) => invoke<MeetingSettings>("set_meeting_settings", { settings });
/** "Remind me to tell people I'm recording" (W-1.7). */
export const recordingNoticeSettings = () => invoke<RecordingNoticeSettings>("recording_notice_settings");
export const setRecordingNoticeSettings = (settings: RecordingNoticeSettings) =>
  invoke<RecordingNoticeSettings>("set_recording_notice_settings", { settings });
export const alertStatus = () => invoke<AlertView>("alert_status");
export const alertAccept = (openApp: boolean) => invoke<void>("alert_accept", { openApp });
export const alertDismiss = () => invoke<void>("alert_dismiss");
export const questionAlertStatus = () => invoke<QuestionNotice[]>("question_alert_status");
export const questionAlertOpen = (id: string) => invoke<void>("question_alert_open", { id });
export const questionAlertDismiss = () => invoke<void>("question_alert_dismiss");
/** "Still in a meeting?": Keep recording, or Stop (trimmed to the last words once transcribed). */
export const stillHereKeep = () => invoke<void>("still_here_keep");
export const stillHereStop = () => invoke<void>("still_here_stop");
/** The budget alert popup's queued 80% / 100% notices. */
export const budgetAlertStatus = () => invoke<BudgetNoticeView[]>("budget_alert_status");
export const budgetAlertRaise = () => invoke<void>("budget_alert_raise");
export const budgetAlertDismiss = () => invoke<void>("budget_alert_dismiss");
/** The tray popup's next meeting chip: the main window opens on the topic. */
export const popupOpenTopic = (topicId: number) => invoke<void>("popup_open_topic", { topicId });
/** The topic the tray popup asked for, once. */
export const takePendingOpenTopic = () => invoke<number | null>("take_pending_open_topic");
export const takePendingOpenMeeting = () => invoke<OpenMeetingRequest | null>("take_pending_open_meeting");
export const meetingLiveAlerts = (id: number) => invoke<LiveAlertView[]>("meeting_live_alerts", { id });
export const meetingLiveTranscript = (id: number) => invoke<LiveTranscriptView>("meeting_live_transcript", { id });

export const calendarAccounts = () => invoke<CalendarAccounts>("calendar_accounts");
export const calendarConnect = () => invoke<CalendarAccounts>("calendar_connect");
export const calendarReconnect = (email: string) => invoke<CalendarAccounts>("calendar_reconnect", { email });
export const calendarDisconnect = (email: string) => invoke<CalendarAccounts>("calendar_disconnect", { email });
export const gmailAccounts = () => invoke<GmailAccounts>("gmail_accounts");
export const gmailConnect = () => invoke<GmailAccounts>("gmail_connect");
export const gmailReconnect = (email: string) => invoke<GmailAccounts>("gmail_reconnect", { email });
export const gmailDisconnect = (email: string) => invoke<GmailAccounts>("gmail_disconnect", { email });
/** Whether removing this Google account also revokes Whimbrel's access with Google (PR 798). */
export const googleRemovalRevokes = (service: "gmail" | "calendar", email: string) => invoke<boolean>("google_removal_revokes", { service, email });
export const gmailDeleteStoredMail = (email: string) => invoke<GmailAccounts>("gmail_delete_stored_mail", { email });
/** Your own Google Cloud project (Settings → Connectors → Advanced). The secret is write-only. */
export const googleClientStatus = () => invoke<GoogleClientStatus>("google_client_status");
export const googleClientSave = (clientId: string, clientSecret: string) =>
  invoke<GoogleClientStatus>("google_client_save", { clientId, clientSecret });
export const googleClientClear = () => invoke<GoogleClientStatus>("google_client_clear");
export const mcpServerState = () => invoke<McpServerView>("mcp_server_state");
export const mcpSetEnabled = (enabled: boolean) => invoke<McpServerView>("mcp_set_enabled", { enabled });
export const mcpClients = () => invoke<McpClientView[]>("mcp_clients");
export const mcpCreateClient = (name: string, scopes: string[]) => invoke<McpNewClient>("mcp_create_client", { name, scopes });
export const mcpRevokeClient = (id: number) => invoke<McpClientView[]>("mcp_revoke_client", { id });
export const calendarSyncNow = () => invoke<CalendarAccounts>("calendar_sync_now");

export const retentionSettings = () => invoke<RetentionView>("retention_settings");
export const setRetentionSettings = (settings: RetentionSettings) => invoke<RetentionView>("set_retention_settings", { settings });
export const retentionRunNow = () => invoke<RetentionView>("retention_run_now");
export const retentionPreview = () => invoke<RetentionPreview>("retention_preview");

// Settings -> Profile (src-tauri/src/commands/profile.rs).
export const profileGet = () => invoke<ProfileView>("profile_get");
export const profileSetFields = (fields: ProfileFields) => invoke<ProfileView>("profile_set_fields", { fields });
export const profileSetSwitches = (switches: ProfileSwitches) => invoke<ProfileView>("profile_set_switches", { switches });
export const profileAddFact = (text: string) => invoke<ProfileView>("profile_add_fact", { text });
export const profileUpdateFact = (id: string, text: string) => invoke<ProfileView>("profile_update_fact", { id, text });
export const profileRemoveFact = (id: string) => invoke<ProfileView>("profile_remove_fact", { id });
export const profileSuggestions = () => invoke<SuggestionView[]>("profile_suggestions");
export const profileAcceptSuggestion = (id: number) => invoke<ProfileView>("profile_accept_suggestion", { id });
export const profileDismissSuggestion = (id: number) => invoke<void>("profile_dismiss_suggestion", { id });

// Automations -> Routines (src-tauri/src/commands/routines.rs). Each change returns the refreshed list.
export const routinesList = () => invoke<RoutineView[]>("routines_list");
export const routineSetEnabled = (id: number, enabled: boolean) => invoke<RoutineView[]>("routine_set_enabled", { id, enabled });
export const routineSetSchedule = (id: number, schedule: string) => invoke<RoutineView[]>("routine_set_schedule", { id, schedule });
export const routineRuns = (limit?: number) => invoke<RunView[]>("routine_runs", { limit });
export const routinesHost = () => invoke<HostView>("routines_host");
export const routinesSetHost = (id: string) => invoke<HostView>("routines_set_host", { id });
export const statusOverview = () => invoke<StatusView>("status_overview");
// Settings → Storage (plan 2026-11-12-storage-visibility).
export const storageOverview = () => invoke<StorageView>("storage_overview");
/** Holds the store until the rewrite is done (about a minute for a few GB). */
export const storageCompact = () => invoke<CompactView>("storage_compact");
export const storageFreeUpOptions = (days: number) => invoke<FreeUpOptions>("storage_free_up_options", { days });
export const storageDeleteOldRecordings = (days: number) => invoke<number>("storage_delete_old_recordings", { days });
export const storageDeleteOldCaptures = (days: number) => invoke<number>("storage_delete_old_captures", { days });
export const storageCompressRecordings = () => invoke<CompressView>("storage_compress_recordings");
export const diskAlertStatus = () => invoke<LowDisk[]>("disk_alert_status");
export const diskAlertOpen = () => invoke<void>("disk_alert_open");
export const diskAlertDismiss = () => invoke<void>("disk_alert_dismiss");
export const routineSetAutoApprove = (id: number, enabled: boolean) => invoke<RoutineView[]>("routine_set_auto_approve", { id, enabled });
export const routineSetModelPref = (id: number, pref: string) => invoke<RoutineView[]>("routine_set_model_pref", { id, pref });
export const routineModelOptions = () => invoke<ModelOptionView[]>("routine_model_options");
// Run in the cloud (src-tauri/src/commands/routines.rs; the shapes are in automations/cloudRoutineModel.ts).
export const routinesCloud = () => invoke<CloudRoutinesView>("routines_cloud");
export const routineSetCloud = (template: string, on: boolean) => invoke<CloudRoutinesView>("routine_set_cloud", { template, on });
export const routineSetReadyBy = (template: string, readyBy: string) => invoke<CloudRoutinesView>("routine_set_ready_by", { template, readyBy });

// Automations -> Approvals (src-tauri/src/commands/approvals.rs).
export const approvalsList = () => invoke<ApprovalView[]>("approvals_list");
export const approvalsCount = () => invoke<number>("approvals_count");
export const approvalApprove = (runId: number, account?: string) => invoke<ApproveResult>("approval_approve", { runId, account });
export const approvalReject = (runId: number) => invoke<ApprovalView[]>("approval_reject", { runId });

/** Starts answering `query`; resolves once it has started. The answer arrives on `onEvent`. */
export function chatAsk(query: string, conversationId: number | null, onEvent: (event: ChatEvent) => void) {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("chat_ask", { query, conversationId, onEvent: channel });
}
// The hotkey panel (src-tauri/src/hotkey_panel, commands/window_ask.rs).
/** Reads the window the panel was opened over; sends nothing. */
export const hotkeyPanelRead = () => invoke<HotkeyPanelRead>("hotkey_panel_read");
/** Closes the panel, drops what it read and returns to the window it came from. */
export const hotkeyPanelClose = () => invoke<void>("hotkey_panel_close");
/** Settings: the panel's shortcut and whether it is registered. */
export const hotkeyStatus = () => invoke<HotkeyStatus>("hotkey_status");
/** Settings: registers `shortcut` (e.g. "Control+Alt+Space") and saves it only once it registers. Resolves with
 *  the attempt: `registered: false` means the old one is back (re-read `hotkeyStatus` for it). */
export const setHotkeyShortcut = (shortcut: string) => invoke<HotkeyStatus>("set_hotkey_shortcut", { shortcut });
/** Asks `query` about the window read for `requestId`. The answer arrives on `onEvent`; the reply says where it went. */
export function windowAsk(requestId: number, query: string, onEvent: (event: ChatEvent) => void) {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<WindowAskReply>("window_ask", { requestId, query, onEvent: channel });
}
export const chatStop = (runId: string) => invoke<boolean>("chat_stop", { runId });
/** Starts answering `query` about one meeting from its transcript (the meeting Ask bar). Nothing is saved; `history` is the page's answered exchanges. */
export function meetingAsk(meetingId: number, query: string, history: MeetingAskExchange[], onEvent: (event: ChatEvent) => void) {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("meeting_ask", { meetingId, query, history, onEvent: channel });
}
export const chatConversations = () => invoke<ConversationRow[]>("chat_conversations");
export const chatConversation = (id: number) => invoke<ConversationView | null>("chat_conversation", { id });
export const chatRenameConversation = (id: number, title: string) => invoke<ConversationRow[]>("chat_rename_conversation", { id, title });
export const chatStarConversation = (id: number, starred: boolean) => invoke<ConversationRow[]>("chat_star_conversation", { id, starred });
export const chatDeleteConversation = (id: number) => invoke<ConversationRow[]>("chat_delete_conversation", { id });
export const chatStarterPrompts = () => invoke<StarterPromptView[]>("chat_starter_prompts");
export const chatSpend = () => invoke<ChatSpend>("chat_spend");

export const todayOverview = () => invoke<TodayOverview>("today_overview");
/** The next calendar meeting and its Meeting Prep brief, if any. */
export const nextMeetingPrep = () => invoke<NextMeetingView | null>("next_meeting_prep");
/** The brief a Prep chip names, by its key; nothing when it is gone or unreadable. */
export const meetingPrepBrief = (key: string) => invoke<NextMeetingView | null>("meeting_prep_brief", { key });
/** Each recorded meeting's Prep chip key and topic, in one call. */
export const meetingChips = (ids: number[]) => invoke<MeetingChipsView[]>("meeting_chips", { ids });
/** Serves today's cached recap unless `refresh`; otherwise calls a model (slow, costs). */
export const todayRecap = (refresh: boolean) => invoke<TodayRecap>("today_recap", { refresh });

// The launch through the coordinator (X4 Task 11c, src-tauri/src/commands/launch.rs). These work
// before the memory database is open. None of them carries a key or the recovery file's text.
/** Where the launch is, read on every mount (the window may have missed an event). */
export const launchState = () => invoke<LaunchView>("launch_state");
/** An answer to the current screen; false when that screen does not offer it. */
export const launchRespond = (answer: LaunchAnswer) => invoke<boolean>("launch_respond", { answer });
/** The first window rendered: counts the good launch once. */
export const launchFirstWindowRendered = () => invoke<boolean>("launch_first_window_rendered");
/** After a Delete everything: the line the onboarding shows first, once (the shell consumes it); null otherwise. */
export const launchFreshLibraryNotice = () => invoke<string | null>("launch_fresh_library_notice");
/** This launch opened a new, empty store because the library went missing: the app's one banner. */
export const launchLibraryMissing = () => invoke<boolean>("launch_library_missing");
/** Dismiss on that banner: gone for this launch, even after the window is rebuilt. */
export const launchDismissLibraryMissing = () => invoke<void>("launch_dismiss_library_missing");
/** Settings → Capture → Files kept aside: the set-aside `.foreign-N` files beside the memory database. */
export const foreignFilesList = () => invoke<ForeignFilesView>("foreign_files_list");
/** Deletes what core finds now (confirmed in the card). */
export const foreignFilesDelete = (confirmed: number) => invoke<ForeignFilesDeleted>("foreign_files_delete", { confirmed });
/** Recovery: a Windows open dialog; the file's text stays in the shell. */
export const launchRecoveryChooseFile = () => invoke<ChosenFile>("launch_recovery_choose_file");
export const launchRecoveryUnlockFile = (passphrase: string) => invoke<UseKeyResult>("launch_recovery_unlock_file", { passphrase });
export const launchRecoveryUnlockWords = (words: string) => invoke<UseKeyResult>("launch_recovery_unlock_words", { words });
/** The recovery setup (onboarding at launch, or the Settings opt-in). Argon2id: about a second. */
export const setupBegin = (scope: SetupScope, passphrase: string) => invoke<SetupBegun>("setup_begin", { scope, passphrase });
export const setupSaveFile = (scope: SetupScope) => invoke<SaveFileResult>("setup_save_file", { scope });
export const setupVerify = (scope: SetupScope, passphrase: string) => invoke<boolean>("setup_verify", { scope, passphrase });
export const setupStartOver = (scope: SetupScope) => invoke<boolean>("setup_start_over", { scope });
export const setupContinue = (scope: SetupScope) => invoke<boolean>("setup_continue", { scope });
export const setupSavedTo = (scope: SetupScope) => invoke<string | null>("setup_saved_to", { scope });
/** Whether the scope's setup wraps an existing key (Start over then keeps the words); null when not open. */
export const setupReusesKey = (scope: SetupScope) => invoke<boolean | null>("setup_reuses_key", { scope });
/** Whether quitting now would waste words the shell made for a fresh key (what closing the window asks). */
export const setupHasFreshMaterial = (scope: SetupScope) => invoke<boolean>("setup_has_fresh_material", { scope });

// Settings > Capture & Privacy > Database encryption (commands/encryption.rs).
export const encryptionCard = () => invoke<EncryptionCardView>("encryption_card");
/** Runs before any key or words exist; null means the setup may start. */
export const encryptionPrecheckForSetup = () => invoke<SetupRefusal | null>("encryption_precheck_for_setup");
/** Stores the pending key (never replacing one) and sets "migrate at next launch". Never migrates. */
export const encryptionEncrypt = () => invoke<EncryptOutcome>("encryption_encrypt");
export const encryptionNoteAction = (action: NoteAction) => invoke<NoteActionResult>("encryption_note_action", { action });
export const setFastSearch = (enabled: boolean) => invoke<boolean>("set_fast_search", { enabled });
/** The local speed check: timings and the kind of PC only. Progress 0..1 arrives on `onProgress`. */
export function speedCheckRun(onProgress: (overall: number) => void) {
  const channel = new Channel<number>();
  channel.onmessage = onProgress;
  return invoke<SpeedCheckResult>("speed_check_run", { onProgress: channel });
}
export const speedCheckCancel = () => invoke<void>("speed_check_cancel");
export const speedCheckSave = () => invoke<string | null>("speed_check_save");
