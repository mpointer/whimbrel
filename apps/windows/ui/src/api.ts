// Typed wrappers over the shell's Tauri commands (src-tauri/src/commands).

import { Channel, invoke } from "@tauri-apps/api/core";
import type {
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
} from "./generated/types";
import type { SkinDto } from "./skin";

export const appStatus = () => invoke<AppStatus>("app_status");
export const getRoutingSettings = () => invoke<RoutingSettings>("get_routing_settings");
export const setRoutingSettings = (routing: RoutingSettings) => invoke<void>("set_routing_settings", { routing });
export const captureStatus = () => invoke<CaptureStatus>("capture_status");
export const setCaptureEnabled = (enabled: boolean) => invoke<CaptureStatus>("set_capture_enabled", { enabled });

// The tray's quick-glance popup (Task 10).
export const trayStatus = () => invoke<TrayStatus>("tray_status");
export const popupOpenMain = () => invoke<void>("popup_open_main");
export const popupQuit = () => invoke<void>("popup_quit");

// Auto-update (M3.5): the version of a downloaded update, and "Restart now".
export const updateReady = () => invoke<string | null>("update_ready");
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
export const syncDevices = () => invoke<DeviceView[]>("sync_devices");
export const syncSetDeviceName = (name: string) => invoke<DeviceView[]>("sync_set_device_name", { name });
export const syncApproveDevice = (id: string) => invoke<DeviceView[]>("sync_approve_device", { id });
export const syncRevokeDevice = (id: string) => invoke<DeviceView[]>("sync_revoke_device", { id });

export const aiSettings = () => invoke<AiSettings>("ai_settings");
/** A paid provider's monthly budget in dollars; `null` removes it. */
export const setProviderBudget = (provider: Provider, dollars: number | null) =>
  invoke<ProviderBudgetView[]>("set_provider_budget", { provider, dollars });
/** "Open AI settings": the main window shows Settings at AI. */
export const openAiSettings = () => invoke<void>("open_ai_settings");
/** Opens a new email to Whimbrel with the version and Windows build filled in. */
export const sendFeedback = () => invoke<FeedbackResult>("send_feedback");
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
export const topicSuggestions = () => invoke<TopicSuggestionView[]>("topic_suggestions");
export const acceptTopicSuggestion = (topicId: number, name: string) => invoke<TopicSeedView>("accept_topic_suggestion", { topicId, name });
export const dismissTopicSuggestion = (topicId: number) => invoke<void>("dismiss_topic_suggestion", { topicId });
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
export const todaySchedule = () => invoke<ScheduleView>("today_schedule");

// Skins (core/skins through src-tauri/src/commands/skins.rs). `SkinDto` is typed by hand in skin.ts.
export const skinsList = () => invoke<SkinDto[]>("skins_list");
/** Moves when a sync changed the skins folder (Task 15). */
export const skinsRevision = () => invoke<number>("skins_revision");
/** An approved image of one of the user's skins, as raw bytes. */
export const skinImage = (id: string, name: string) => invoke<ArrayBuffer>("skin_image", { id, name });

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
export const skinImageThumbnail = (url: string) => invoke<ArrayBuffer>("skin_image_thumbnail", { url });
export const skinImageReject = (url: string) => invoke<void>("skin_image_reject", { url });
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
export const meetingList = (filter: string) => invoke<MeetingRow[]>("meeting_list", { filter });
/** Import (W-1.6): each opens a picker and resolves null when it's cancelled. */
export const importTranscripts = () => invoke<ImportView | null>("import_transcripts");
export const importMarkdownFolder = () => invoke<ImportView | null>("import_markdown_folder");
export const meetingWorkspace = (id: number) => invoke<WorkspaceView | null>("meeting_workspace", { id });
export const meetingTranscript = (id: number) => invoke<TranscriptView>("meeting_transcript", { id });
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
export const gmailDeleteStoredMail = (email: string) => invoke<GmailAccounts>("gmail_delete_stored_mail", { email });
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
export const routineSetAutoApprove = (id: number, enabled: boolean) => invoke<RoutineView[]>("routine_set_auto_approve", { id, enabled });
export const routineSetModelPref = (id: number, pref: string) => invoke<RoutineView[]>("routine_set_model_pref", { id, pref });
export const routineModelOptions = () => invoke<ModelOptionView[]>("routine_model_options");

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
