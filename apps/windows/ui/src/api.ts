// Typed wrappers over the shell's Tauri commands (src-tauri/src/commands).

import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AiSettings,
  AppStatus,
  CaptureDetail,
  CaptureSettings,
  CaptureStatus,
  ChatEvent,
  ChatSpend,
  ConversationRow,
  ConversationView,
  StarterPromptView,
  SuggestionView,
  TodayOverview,
  TodayRecap,
  DeviceView,
  DownloadView,
  EpisodeRow,
  LibraryRow,
  LibrarySettings,
  MeetingDetail,
  MeetingModelsStatus,
  MeetingRecordingView,
  MeetingRow,
  ModelStatus,
  ActionItemView,
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
  RunView,
  RetentionSettings,
  RetentionView,
  MeetingSettings,
  NextMeetingView,
  TranscriptView,
  WorkspaceView,
  PromptSurface,
  Provider,
  ProviderKey,
  RoutingSettings,
  ScheduleView,
  SyncView,
  TrayStatus,
  GeneralSettings,
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

export const generalSettings = () => invoke<GeneralSettings>("general_settings");
export const setAutostart = (enabled: boolean) => invoke<GeneralSettings>("set_autostart", { enabled });

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
export const syncDevices = () => invoke<DeviceView[]>("sync_devices");
export const syncSetDeviceName = (name: string) => invoke<DeviceView[]>("sync_set_device_name", { name });
export const syncApproveDevice = (id: string) => invoke<DeviceView[]>("sync_approve_device", { id });
export const syncRevokeDevice = (id: string) => invoke<DeviceView[]>("sync_revoke_device", { id });

export const aiSettings = () => invoke<AiSettings>("ai_settings");
export const setApiKey = (provider: Provider, key: string | null) => invoke<ProviderKey[]>("set_api_key", { provider, key });
export const setPrompt = (surface: PromptSurface, text: string) => invoke<void>("set_prompt", { surface, text });
export const setSummaryInstructions = (text: string) => invoke<void>("set_summary_instructions", { text });
export const providerModels = (provider: Provider) => invoke<string[]>("provider_models", { provider });

export const captureSettings = () => invoke<CaptureSettings>("capture_settings");
export const setExcludedApps = (excluded: string[]) => invoke<string[]>("set_excluded_apps", { excluded });

export const modelStatus = () => invoke<ModelStatus>("model_status");
export const modelDownloadStart = () => invoke<string>("model_download_start");
export const modelDownloadPoll = (jobId: string) => invoke<DownloadView>("model_download_poll", { jobId });
export const modelDownloadCancel = (jobId: string) => invoke<boolean>("model_download_cancel", { jobId });
export const restartApp = () => invoke<void>("restart_app");
export const meetingModelsStatus = () => invoke<MeetingModelsStatus>("meeting_models_status");
export const meetingModelsDownloadStart = () => invoke<string>("meeting_models_download_start");

export const meetingRecordStart = () => invoke<MeetingRecordingView>("meeting_record_start");
export const meetingRecordStop = () => invoke<MeetingRecordingView>("meeting_record_stop");
export const todaySchedule = () => invoke<ScheduleView>("today_schedule");

// Skins (core/skins through src-tauri/src/commands/skins.rs). `SkinDto` is typed by hand in skin.ts.
export const skinsList = () => invoke<SkinDto[]>("skins_list");
/** An approved image of one of the user's skins, as raw bytes. */
export const skinImage = (id: string, name: string) => invoke<ArrayBuffer>("skin_image", { id, name });
export const meetingRecordStartEvent = (eventId: string) => invoke<MeetingRecordingView>("meeting_record_start_event", { eventId });
export const meetingRecordingStatus = () => invoke<MeetingRecordingView>("meeting_recording_status");

// The Meetings tab (M3.3 Task 5).
export const meetingList = (filter: string) => invoke<MeetingRow[]>("meeting_list", { filter });
export const meetingWorkspace = (id: number) => invoke<WorkspaceView | null>("meeting_workspace", { id });
export const meetingTranscript = (id: number) => invoke<TranscriptView>("meeting_transcript", { id });
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
export const meetingRegenerationConflict = (id: number) => invoke<boolean>("meeting_regeneration_conflict", { id });
export const meetingForget = (id: number) => invoke<void>("meeting_forget", { id });
export const meetingMostRelevant = () => invoke<number | null>("meeting_most_relevant");

// Detection, its alert window, and Settings → Meetings (M3.3 Task 6).
export const meetingSettings = () => invoke<MeetingSettings>("meeting_settings");
export const setMeetingSettings = (settings: MeetingSettings) => invoke<MeetingSettings>("set_meeting_settings", { settings });
export const alertStatus = () => invoke<AlertView>("alert_status");
export const alertAccept = (openApp: boolean) => invoke<void>("alert_accept", { openApp });
export const alertDismiss = () => invoke<void>("alert_dismiss");

export const calendarAccounts = () => invoke<CalendarAccounts>("calendar_accounts");
export const calendarConnect = () => invoke<CalendarAccounts>("calendar_connect");
export const calendarDisconnect = (email: string) => invoke<CalendarAccounts>("calendar_disconnect", { email });
export const gmailAccounts = () => invoke<GmailAccounts>("gmail_accounts");
export const gmailConnect = () => invoke<GmailAccounts>("gmail_connect");
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
export const routineSetAutoApprove = (id: number, enabled: boolean) => invoke<RoutineView[]>("routine_set_auto_approve", { id, enabled });

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
/** Serves today's cached recap unless `refresh`; otherwise calls a model (slow, costs). */
export const todayRecap = (refresh: boolean) => invoke<TodayRecap>("today_recap", { refresh });
