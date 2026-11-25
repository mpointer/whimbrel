// Extra answers for the screenshot harness (screens.shots.ts), layered over e2e/fakeBridge.ts, which CI uses and
// which stays as it is. Everything here is fictional: the user is Alex Rivera; colleagues are Sam Okafor, Priya
// Shah and Dana Whitfield; companies are Northwind and Acme. Shapes follow src/generated/types.ts.
//
// `answers(now)` are plain values (the page gets them as JSON). `helpers(now)` is plain data too, put on the page
// as `globalThis.__fx`; `dynamic` are functions of the call's arguments that read it. The page evaluates their
// source, so they must not use anything from this module.
const MIN = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

const THIS_PC = { deviceId: "PC1", label: "Alex's laptop", thisDevice: true };
const MAC = { deviceId: "MAC1", label: "Alex's MacBook", thisDevice: false };

const localDay = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

const meetingRow = (now: number, id: number, hoursAgo: number, title: string, extra: Record<string, unknown> = {}) => ({
  actionItemCount: 3,
  attendeeCount: 4,
  device: THIS_PC,
  endUnixMs: now - hoursAgo * HOUR + 42 * MIN,
  hasSummary: true,
  id,
  status: "done",
  title,
  trigger: "calendar",
  tsUnixMs: now - hoursAgo * HOUR,
  ...extra,
});

/** The meetings the Meetings tab lists: one this morning, then earlier days. */
export const meetings = (now: number) => [
  meetingRow(now, 1, 3, "Northwind renewal check-in"),
  meetingRow(now, 2, 27, "Weekly product sync", { actionItemCount: 2, attendeeCount: 6 }),
  meetingRow(now, 3, 52, "Acme onboarding call", { trigger: "detected", attendeeCount: 3 }),
  meetingRow(now, 4, 76, "Design review with Priya", { actionItemCount: 1, attendeeCount: 2, device: MAC }),
  meetingRow(now, 5, 150, "Hiring sync: senior engineer", { trigger: "manual", actionItemCount: 0, attendeeCount: 3 }),
];

const ATTENDEES = ["Sam Okafor", "Priya Shah", "Dana Whitfield"];

const workspace = (now: number, id: number) => ({
  actionItems: [
    { completed: false, id: 11, source: "generated", text: "Alex to send Northwind the revised renewal quote by Friday" },
    { completed: false, id: 12, source: "generated", text: "Sam to confirm the pilot seat count with Northwind's IT lead" },
    { completed: true, id: 13, source: "generated", text: "Dana to share last quarter's usage report" },
  ],
  attendees: ATTENDEES,
  canChooseLanguage: true,
  conferenceLink: "https://meet.example.test/northwind-renewal",
  decisions: ["Offer Northwind a 12-month term with a 5% volume discount", "Keep the pilot team at 25 seats until the security review closes"],
  decisionsAreEdited: false,
  elapsedSeconds: null,
  hasTranscript: true,
  openQuestions: ["Does Northwind need SSO on day one?", "Who signs for Acme's side of the data agreement?"],
  openQuestionsAreEdited: false,
  personalNotes: "Dana prefers email over calls. Ask Sam about the Q1 budget before sending the quote.",
  phase: "after",
  processingStage: null,
  readOnly: false,
  row: meetings(now).find((m) => m.id === id) ?? meetings(now)[0],
  spokenLanguage: null,
  summary:
    "Alex, Sam, Priya and Dana reviewed the Northwind renewal. Northwind is happy with the pilot and wants to extend it to 60 seats, but is waiting on its security review. The group agreed to offer a 12-month term with a 5% volume discount and to keep the pilot at 25 seats until the review closes. Alex will send the revised quote by Friday; Sam will confirm seat counts with Northwind's IT lead.",
  summaryFailure: null,
  summaryFallback: null,
  summaryIsEdited: false,
  transcription: null,
  trigger: "calendar",
});

const topicRef = (id: number, name: string) => ({ key: `seed-${id}`, name, topicId: id });
const NORTHWIND = topicRef(1, "Northwind");
const ACME = topicRef(2, "Acme");

const step = (now: number, key: string, text: string, direction: string, status: string, extra: Record<string, unknown> = {}) => ({
  askedAtUnixMs: now - 2 * DAY,
  closedBy: null,
  confirmed: true,
  direction,
  dueDate: null,
  fromScreen: false,
  key,
  note: null,
  open: { id: 1, kind: "meeting" },
  sourceKind: "meeting",
  sourceTitle: "Northwind renewal check-in",
  status,
  text,
  ...extra,
});

const topicPage = (now: number, topicId: number) => ({
  done: [step(now, "s9", "Share the usage report with Dana", "you_owe", "done", { closedBy: "You" })],
  kind: topicId === 1 ? "customer" : "project",
  lead: { kind: "step", text: "Send the revised renewal quote", dueDate: localDay(now + 2 * DAY) },
  nextMeeting: { attendeesByOrg: [{ org: "Northwind", count: 2 }, { org: "Contoso", count: 1 }], me: 1, startUnixMs: now + 5 * HOUR, title: "Northwind security review" },
  older: [step(now, "s8", "Intro Sam to Northwind's IT lead", "you_owe", "open", { askedAtUnixMs: now - 40 * DAY })],
  owedToYou: [step(now, "s3", "Dana to send the signed data agreement", "owed_to_you", "open", { dueDate: localDay(now + 4 * DAY) })],
  recent: [
    { kind: "meeting", open: { id: 1, kind: "meeting" }, title: "Northwind renewal check-in", tsUnixMs: now - 3 * HOUR },
    { kind: "meeting", open: { id: 2, kind: "meeting" }, title: "Weekly product sync", tsUnixMs: now - 27 * HOUR },
    { kind: "capture", open: { id: 101, kind: "capture" }, title: "Northwind pricing sheet - Excel", tsUnixMs: now - 30 * HOUR },
  ],
  recentTotal: 3,
  role: "Account owner for the renewal",
  roleEdited: false,
  topic: topicRef(topicId, ["", "Northwind", "Acme", "Q1 planning", "Mobile app launch", "Hiring"][topicId] || "Topic"),
  youOwe: [
    step(now, "s1", "Send Northwind the revised renewal quote", "you_owe", "open", { dueDate: localDay(now + 2 * DAY) }),
    step(now, "s2", "Book the security review follow-up", "you_owe", "open"),
  ],
});

const prepBrief = (now: number) => ({
  attendees: [
    { note: "Sam leads the Northwind account and owns the seat-count question.", sources: ["m1"], who: "Sam Okafor" },
    { note: "Priya is reviewing the pilot's security answers.", sources: ["m1"], who: "Priya Shah" },
  ],
  generatedAtUnixMs: now - 30 * MIN,
  openItems: [{ sources: ["m1"], text: "Alex still owes Northwind the revised renewal quote." }],
  openQuestions: [{ sources: ["m1"], text: "Does Northwind need SSO on day one?" }],
  sources: [{ id: "m1", meetingId: 1, title: "Northwind renewal check-in" }],
  summary: "Last time the group agreed to a 12-month term and a 5% volume discount. Northwind's security review is the one thing left open.",
  summarySources: ["m1"],
});

function aiSettings() {
  return {
    budgets: [{ cents: 500, provider: "anthropic" }],
    imageSearchKey: null,
    keys: [{ masked: "sk-ant-…a1b2", provider: "anthropic", syncEnabled: true }],
    modelWarnings: [],
    ollamaWarning: null,
    prompts: [],
    providers: [
      { cheapModel: "llama3", dataTerms: null, id: "ollama", keyPageUrl: null, keyPrefix: null, name: "Ollama (on this PC)", policyUrl: null },
      { cheapModel: "claude-haiku", dataTerms: { checked: "2026-09-01", text: "Anthropic does not train on API data by default." }, id: "anthropic", keyPageUrl: "https://console.example.test/keys", keyPrefix: "sk-ant-", name: "Anthropic", policyUrl: "https://example.test/policy" },
      { cheapModel: "gpt-mini", dataTerms: null, id: "openai", keyPageUrl: "https://platform.example.test/keys", keyPrefix: "sk-", name: "OpenAI", policyUrl: null },
      { cheapModel: "gemini-flash", dataTerms: null, id: "google", keyPageUrl: "https://aistudio.example.test/keys", keyPrefix: null, name: "Google AI", policyUrl: null },
    ],
    routing: {
      anthropicModel: "claude-sonnet",
      cheapFirst: true,
      globalDailyCapCents: null,
      googleAiBillingConfirmed: false,
      googleModel: "",
      localOnly: false,
      ollamaModel: "llama3",
      openaiModel: "",
      openrouterModel: "",
      preferredProvider: "anthropic",
      remoteOllamaGoogleData: false,
      useLocalModel: false,
      userDailyCapCents: 200,
      whimbrelModel: "",
    },
    spend: { globalCents: 37, userCents: 37 },
    summaryInstructions: "",
    usage: { byProvider: [{ cents: 212, provider: "anthropic" }], last30DaysCents: 212, monthByProvider: [{ cents: 412, provider: "anthropic" }], monthToDateCents: 412, todayCents: 37 },
  };
}

const capture = (id: number, app: string, title: string, snippet: string, source: string, ts: number, device = THIS_PC) => ({ app, device, id, snippet, source, title, tsUnixMs: ts });

function libraryRecent(now: number) {
  return [
    { kind: "meeting", ...meetingRow(now, 1, 3, "Northwind renewal check-in") },
    { kind: "capture", ...capture(101, "Microsoft Excel", "Northwind pricing sheet.xlsx - Excel", "Northwind pricing sheet: 60 seats, 12-month term, 5% volume discount, total 41,040", "accessibility", now - 25 * MIN) },
    { kind: "capture", ...capture(102, "Outlook", "Re: Northwind renewal quote - Outlook", "Re: Northwind renewal quote. Hi Alex, could you confirm whether the discount applies to the pilot seats?", "accessibility", now - 2 * HOUR) },
    { kind: "episode", app: "Google Chrome", appHidden: false, device: THIS_PC, endTsUnixMs: now - 4 * HOUR + 18 * MIN, hidden: false, id: 201, itemCount: 23, status: "done", summary: "Read Northwind's public security page and compared its SSO options with the pilot setup.", title: "Researching Northwind's SSO options", tsUnixMs: now - 4 * HOUR },
    { kind: "capture", ...capture(103, "Slack", "#northwind-renewal - Slack", "Sam: pilot seat count is 25 until security signs off, I'll confirm with their IT lead", "ocr", now - 26 * HOUR, MAC) },
    { kind: "meeting", ...meetingRow(now, 2, 27, "Weekly product sync", { actionItemCount: 2, attendeeCount: 6 }) },
    { kind: "transcript", id: 301, meetingId: 3, meetingTitle: "Acme onboarding call", snippet: "...we can have the data agreement signed by the end of next week if Dana sends it today...", speaker: "Priya Shah", startCs: 125_000 },
    { kind: "episode", app: "Microsoft Word", appHidden: false, device: THIS_PC, endTsUnixMs: now - 52 * HOUR + 40 * MIN, hidden: false, id: 202, itemCount: 12, status: "done", summary: "Drafted the Acme onboarding checklist.", title: "Acme onboarding checklist", tsUnixMs: now - 52 * HOUR },
  ];
}

function routines(now: number) {
  const run = (id: number, routineId: number, name: string, summary: string, outcome: string, ago: number) => ({
    id, originDeviceId: "PC1", originLabel: "Alex's laptop", outcome, ranAtUnixMs: now - ago, routineId, routineName: name, summary,
  });
  return [
    { autoApprove: false, callsModel: true, enabled: true, id: 1, kind: "dailyBrief", lastRun: run(41, 1, "Daily brief", "Yesterday: the Northwind renewal and the Acme onboarding.", "success", 7 * HOUR), modelPref: "", name: "Daily brief", schedule: "07:30", unavailable: null, writes: false },
    { autoApprove: false, callsModel: true, enabled: true, id: 2, kind: "eodRecap", lastRun: run(42, 2, "End-of-day recap", "Draft email to Dana Whitfield waiting for approval.", "pending", 40 * MIN), modelPref: "", name: "End-of-day recap", schedule: "17:30", unavailable: null, writes: true },
    { autoApprove: false, callsModel: true, enabled: false, id: 3, kind: "weeklyDigest", lastRun: null, modelPref: "", name: "Weekly digest", schedule: "16:00", unavailable: null, writes: true },
    { autoApprove: false, callsModel: false, enabled: true, id: 4, kind: "focusTime", lastRun: run(43, 4, "Focus time", "Block two hours on Thursday afternoon.", "pending", 3 * HOUR), modelPref: "", name: "Focus time", schedule: "09:00", unavailable: null, writes: true },
    { autoApprove: false, callsModel: true, enabled: false, id: 5, kind: "replyPriorities", lastRun: null, modelPref: "", name: "Reply priorities", schedule: "", unavailable: "Needs a connected Gmail account.", writes: false },
    { autoApprove: true, callsModel: true, enabled: true, id: 6, kind: "meetingPrep", lastRun: run(44, 6, "Meeting prep", "Brief ready for Northwind security review.", "success", 25 * MIN), modelPref: "", name: "Meeting prep", schedule: "", unavailable: null, writes: false },
  ];
}

function runs(now: number) {
  const mk = (id: number, routineId: number, name: string, summary: string, outcome: string, ago: number) => ({ id, originDeviceId: "PC1", originLabel: "Alex's laptop", outcome, ranAtUnixMs: now - ago, routineId, routineName: name, summary });
  return [
    mk(44, 6, "Meeting prep", "Brief ready for Northwind security review.", "success", 25 * MIN),
    mk(42, 2, "End-of-day recap", "Draft email to Dana Whitfield waiting for approval.", "pending", 40 * MIN),
    mk(43, 4, "Focus time", "Block two hours on Thursday afternoon.", "pending", 3 * HOUR),
    mk(41, 1, "Daily brief", "Yesterday: the Northwind renewal and the Acme onboarding.", "success", 7 * HOUR),
    mk(40, 2, "End-of-day recap", "Recap sent to alex.rivera@contoso.example.test.", "success", 31 * HOUR),
    mk(39, 1, "Daily brief", "The model could not be reached.", "failed", 31 * HOUR),
  ];
}

/** Everything that is the same on every screen. */
export const answers = (now: number): Record<string, unknown> => {
  const rows = meetings(now);
  const startOfToday = new Date(now).setHours(0, 0, 0, 0);
  const todayMeetings = rows.filter((m) => m.tsUnixMs >= startOfToday);
  return {
    // --- who and what ------------------------------------------------------------------------------------------
    profile_get: {
      about: "Operations lead at a small software company. I run customer renewals and the weekly product sync.",
      composedChars: 190,
      enabled: true,
      facts: [
        { id: "f1", sourceMeetingId: 2, text: "Prefers short written summaries to long calls" },
        { id: "f2", sourceMeetingId: null, text: "Works Pacific time" },
      ],
      maxChars: 1200,
      maxFactChars: 160,
      maxFacts: 20,
      name: "Alex Rivera",
      organization: "Contoso Software",
      preview: "Alex Rivera, operations lead at Contoso Software. Runs customer renewals and the weekly product sync.",
      role: "Operations lead",
      suggestEnabled: true,
      suggestionsActive: true,
    },
    profile_suggestions: [
      { createdAtUnixMs: now - 5 * HOUR, id: 1, sourceMeetingId: 1, sourceMeetingTitle: "Northwind renewal check-in", text: "Owns the Northwind renewal" },
      { createdAtUnixMs: now - 30 * HOUR, id: 2, sourceMeetingId: 2, sourceMeetingTitle: "Weekly product sync", text: "Chairs the weekly product sync" },
    ],
    app_status: { dataDir: "C:\\Users\\alex\\AppData\\Local\\Whimbrel", deviceId: "PC1", embeddingModelLoaded: true, startupErrors: [], storeOpen: true, storeQueueWait: null, version: "0.9.80" },
    update_ready: null,
    update_check_status: { state: "upToDate", version: "0.9.80" },
    running_emulated: false,
    launch_library_missing: false,
    general_settings: { autostart: true, autostartAvailable: true },
    take_pending_open_topic: null,
    take_pending_open_meeting: null,
    model_status: { dir: "C:\\Users\\alex\\AppData\\Local\\Whimbrel\\models", installed: true, loaded: true },
    // The meeting models are installed, so the sidebar footer has no download line.
    meeting_models_status: { dir: "C:\\Users\\alex\\AppData\\Local\\Whimbrel\\models", installed: true, missing: [], multilingual: false },
    transcription_quality_status: { offer: false, quality: "automatic", turboInstalled: true, turboSpeed: 6.2 },
    local_model_status: { canOffer: true, installed: false },
    approvals_count: 2,

    // --- Home, redesigned (library_day_blocks, library_capture_apps): window titles and app names only -----------------
    library_day_blocks: [
      {
        start_unix_ms: now - 140 * MIN,
        end_unix_ms: now - 118 * MIN,
        apps: [
          { app_bundle_id: "inbox.exe", items: ["Inbox, 11 messages"], more_items: 0 },
          { app_bundle_id: "slack.exe", items: ["#general"], more_items: 0 },
        ],
        more_apps: 0,
      },
      {
        start_unix_ms: now - 95 * MIN,
        end_unix_ms: now - 52 * MIN,
        apps: [
          { app_bundle_id: "excel.exe", items: ["Pricing v3.xlsx"], more_items: 0 },
          { app_bundle_id: "slack.exe", items: ["#pricing"], more_items: 0 },
          { app_bundle_id: "outlook.exe", items: ["2 messages with Northwind"], more_items: 0 },
        ],
        more_apps: 0,
      },
      {
        start_unix_ms: now - 31 * MIN,
        end_unix_ms: now - 8 * MIN,
        apps: [
          { app_bundle_id: "excel.exe", items: ["Pricing v3.xlsx", "Seat counts.xlsx", "Quote draft.xlsx"], more_items: 2 },
          { app_bundle_id: "MSEdge", items: ["Northwind security overview"], more_items: 0 },
        ],
        more_apps: 1,
      },
    ],
    library_capture_apps: [
      { app_bundle_id: "excel.exe", captures: 412 },
      { app_bundle_id: "slack.exe", captures: 288 },
      { app_bundle_id: "MSEdge", captures: 251 },
      { app_bundle_id: "outlook.exe", captures: 140 },
    ],

    // --- Today -----------------------------------------------------------------------------------------------------
    meeting_most_relevant: 1,
    meeting_list: rows,
    today_overview: {
      day: localDay(now),
      devices: [
        { device: THIS_PC, captures: 1482, lastTsUnixMs: now - 4 * MIN },
        { device: MAC, captures: 312, lastTsUnixMs: now - 95 * MIN },
      ],
      openLoops: [
        { id: 1, subject: "Re: Northwind renewal quote", status: "waiting_on_me", lastMessageFrom: "Dana Whitfield <dana@northwind.example.test>", lastActivityTsUnixMs: now - 2 * HOUR, summary: "Dana asks for the revised quote and whether the discount applies to the pilot seats." },
        { id: 2, subject: "Acme data agreement", status: "waiting_on_them", lastMessageFrom: "Alex Rivera", lastActivityTsUnixMs: now - 26 * HOUR, summary: null },
        { id: 3, subject: "Q1 planning agenda", status: "waiting_on_me", lastMessageFrom: "Sam Okafor <sam@contoso.example.test>", lastActivityTsUnixMs: now - 50 * HOUR, summary: null },
      ],
      recap: {
        cached: true,
        day: localDay(now),
        emptyDay: false,
        openLoops: ["Send Northwind the revised renewal quote", "Reply to Sam about the Q1 planning agenda"],
        summary:
          "You spent the morning on the Northwind renewal: the pricing sheet, the draft quote and a check-in call with Sam, Priya and Dana. Late morning went to the Acme onboarding checklist.",
      },
      briefRecap: null,
      latestMeeting: rows[0],
    },
    today_schedule: {
      events: [
        { actionItemCount: 0, attendees: ["Sam Okafor", "Priya Shah"], endUnixMs: now + 39 * MIN, eventId: "e1", hasSummary: false, prepKey: "prep-1", recordedMeetingId: null, startUnixMs: now + 9 * MIN, title: "Northwind security review" },
        { actionItemCount: 0, attendees: [], endUnixMs: now + 120 * MIN, eventId: "e2", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + 90 * MIN, title: "Focus time" },
        { actionItemCount: 2, attendees: ["Priya Shah"], endUnixMs: now + 240 * MIN, eventId: "e3", hasSummary: true, prepKey: null, recordedMeetingId: 4, startUnixMs: now + 180 * MIN, title: "Design review with Priya" },
        { actionItemCount: 0, attendees: ["Dana Whitfield", "Sam Okafor"], endUnixMs: now + 330 * MIN, eventId: "e4", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + 300 * MIN, title: "Acme kickoff prep" },
      ],
      earlierToday: todayMeetings,
      recording: false,
    },
    next_meeting_prep: { brief: prepBrief(now), eventId: "e1", startUnixMs: now + 9 * MIN, title: "Northwind security review" },
    today_upcoming: {
      days: [
        {
          date: localDay(now + DAY),
          tomorrow: true,
          events: [
            { actionItemCount: 0, attendees: ["Sam Okafor"], endUnixMs: now + DAY + 5 * HOUR, eventId: "u1", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + DAY + 4 * HOUR, title: "Weekly product sync" },
            { actionItemCount: 0, attendees: ["Dana Whitfield"], endUnixMs: now + DAY + 9 * HOUR, eventId: "u2", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + DAY + 8 * HOUR, title: "Acme onboarding follow-up" },
          ],
        },
        {
          date: localDay(now + 2 * DAY),
          tomorrow: false,
          events: [{ actionItemCount: 0, attendees: ["Priya Shah", "Sam Okafor"], endUnixMs: now + 2 * DAY + 3 * HOUR, eventId: "u3", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + 2 * DAY + 2 * HOUR, title: "Roadmap review" }],
        },
      ],
      syncError: null,
    },
    today_recap: { cached: true, day: localDay(now), emptyDay: false, openLoops: [], summary: "You spent the morning on the Northwind renewal." },
    ai_settings: aiSettings(),
    whimbrel_ai_status: { balanceMicros: null, low: false, signedIn: false },

    // --- Meetings ------------------------------------------------------------------------------------------------------
    meeting_recording_status: { degraded: null, failure: null, meetingId: null, paused: false, pausedTotalMs: 0, processing: null, resuming: false, startedUnixMs: null, state: "idle" },
    meeting_trim_info: { lengthMs: 42 * MIN, offered: true },
    meeting_seed_action_items: workspace(now, 1).actionItems,
    meeting_snapshots: { keep: false, snapshots: [] },
    meeting_live_alerts: [],
    meeting_live_transcript: { enabled: true, lines: [] },
    meeting_regeneration_conflict: false,
    recording_notice_settings: { enabled: true, text: "" },
    meeting_settings: { autoStart: false, hasNames: true, heuristicDetection: true, leadMinutes: 1, liveQuestionAlerts: true, liveTranscript: true, otherNames: "Alexandra, Lex", prepLeadMinutes: 10, rememberVoices: true },
    voice_people: [
      { confirmedCount: 14, name: "Sam Okafor", personKey: "p-sam" },
      { confirmedCount: 9, name: "Priya Shah", personKey: "p-priya" },
    ],
    meeting_snapshot_settings: { enabled: true, folder: "C:\\Users\\alex\\Pictures\\Whimbrel slides", isDefault: true },
    calendar_accounts: { accessIssues: [], blocked: null, emails: ["alex.rivera@contoso.example.test"], lastSyncedUnixMs: now - 12 * MIN, nextMeeting: "Northwind security review", nextStartUnixMs: now + 9 * MIN, signInExpired: [], syncError: null, upcoming: 6 },
    gmail_accounts: { accounts: [{ email: "alex.rivera@contoso.example.test", lastReadUnixMs: now - 8 * MIN, needsReconnect: false, readError: null, sendOnly: false, signInExpired: false }], blocked: null },
    google_client_status: { clientId: null },

    // --- Library ---------------------------------------------------------------------------------------------------------
    library_settings: { hiddenEpisodeApps: ["Spotify"], showScreenCaptures: true },
    library_recent: libraryRecent(now),
    library_episodes: libraryRecent(now).filter((r) => r.kind === "episode"),
    chat_conversations: [
      { id: 1, starred: true, title: "What did Northwind ask for?", updatedAtUnixMs: now - 2 * HOUR },
      { id: 2, starred: false, title: "Open items for the Acme onboarding", updatedAtUnixMs: now - 28 * HOUR },
      { id: 3, starred: false, title: "Summarise last week's product sync", updatedAtUnixMs: now - 5 * DAY },
    ],
    chat_starter_prompts: [
      { id: "p1", query: "What did I work on yesterday?", title: "What did I work on yesterday?" },
      { id: "p2", query: "What do I owe people right now?", title: "What do I owe people right now?" },
      { id: "p3", query: "Summarise my last meeting", title: "Summarise my last meeting" },
    ],
    chat_spend: { capCents: 500, userCents: 37 },
    get_routing_settings: aiSettings().routing,
    // Find (screen 5): where an ask would be answered. A hosted stream can fall back, so it names no provider.
    chat_route_preview: ROUTE_HOSTED,
    // The one-time "Use Whimbrel AI?" question: nothing to ask in the harness (it was unanswered, and the prompt logged a TypeError).
    plus_ai_prompt: { state: "done" },

    // --- Topics ---------------------------------------------------------------------------------------------------------
    topics_page_list: [
      { kind: "customer", nextMeetingUnixMs: now + 9 * MIN, openMine: 2, topic: NORTHWIND },
      { kind: "customer", nextMeetingUnixMs: null, openMine: 1, topic: ACME },
      { kind: "project", nextMeetingUnixMs: now + DAY + 4 * HOUR, openMine: 0, topic: topicRef(3, "Q1 planning") },
      { kind: "product", nextMeetingUnixMs: null, openMine: 0, topic: topicRef(4, "Mobile app launch") },
      { kind: "subject", nextMeetingUnixMs: null, openMine: 0, topic: topicRef(5, "Hiring") },
    ],
    client_review_flag: { ok: true, data: null },
    client_control_get: { ok: true, data: { exclude_hosted: false, keep_months: 24 } },
    client_pending_devices: { pending_devices: 0, topic_rows_held_devices: 0 },
    topic_suggestions: [
      { itemCount: 7, name: "Fabrikam", topicId: 11 },
      { itemCount: 4, name: "Security review", topicId: 12 },
    ],
    topic_seeds: [
      { aliases: ["Northwind Traders"], itemCount: 38, key: "seed-1", kind: "customer", name: "Northwind" },
      { aliases: [], itemCount: 21, key: "seed-2", kind: "customer", name: "Acme" },
      { aliases: ["Q1"], itemCount: 9, key: "seed-3", kind: "project", name: "Q1 planning" },
    ],
    topic_sort_state: { enabled: true, key: "ts-…9f3a", keyRejected: false, localOnly: false, overBudget: false, started: true },
    topic_sort_estimate: { indexing: false, sources: 214, tokens: 180_000, usd: 0.42 },
    topic_naming_status: { placeholderCount: 2 },
    topic_names: { names: ["Northwind", "Acme", "Q1 planning"], suggested: ["Fabrikam"] },
    topic_steps_state: { enabled: true, hasKey: true, keyRejected: false, localOnly: false, noModel: false, overBudget: false, started: true },
    topic_steps_estimate: { indexing: false, sources: 214, tokens: 90_000, usd: 0.21 },

    // --- Automations -----------------------------------------------------------------------------------------------------
    routines_list: routines(now),
    routine_runs: runs(now),
    routines_host: { caption: null, options: [{ id: "PC1", label: "Alex's laptop" }, { id: "MAC1", label: "Alex's MacBook" }], primaryDeviceId: "PC1", selected: "" },
    routine_model_options: [
      { label: "Same as Ask", value: "" },
      { label: "Claude Haiku (cheapest)", value: "anthropic:claude-haiku" },
    ],
    // Nothing asks for it any more (the switch is on for everyone); the answer is kept true so no screen can depend on a false one.
    redesign_enabled: true,
    routines_cloud: { cloud_ui_enabled: false, devices: [], has_synced_key: false, local_only: false, routines: [], runner_last_run_ms: null, sync_ready: true },
    approvals_list: [
      {
        account: "alex.rivera@contoso.example.test",
        accounts: ["alex.rivera@contoso.example.test"],
        action: { body: "Hi Dana,\n\nHere is the revised renewal quote we discussed. It carries the 12-month term and the 5% volume discount.\n\nBest,\nAlex", kind: "sendEmail", subject: "Revised Northwind renewal quote", to: "dana@northwind.example.test" },
        blocked: null,
        ranAtUnixMs: now - 40 * MIN,
        routineName: "End-of-day recap",
        runId: 501,
        summary: "Draft email to Dana Whitfield with the revised renewal quote.",
      },
      {
        account: null,
        accounts: ["alex.rivera@contoso.example.test"],
        action: { endUnixMs: now + 2 * DAY + 4 * HOUR, kind: "createCalendarEvent", startUnixMs: now + 2 * DAY + 2 * HOUR, title: "Focus time: renewal quote" },
        blocked: null,
        ranAtUnixMs: now - 3 * HOUR,
        routineName: "Focus time",
        runId: 502,
        summary: "Block two hours on Thursday afternoon.",
      },
    ],

    // --- Settings ---------------------------------------------------------------------------------------------------------
    status_overview: { audioBytes: 2_400_000_000, imageBytes: 610_000_000, outboxOldestMs: null, outboxPending: 0 },
    sync_status: {
      contentError: null,
      contentLastSyncedMs: now - 3 * MIN,
      deviceId: "PC1",
      deviceLimit: false,
      kindBackfillPercent: null,
      message: null,
      plan: { billing: true, interval: "month", plan: "Whimbrel Cloud", renews: true, untilUnixMs: now + 18 * DAY },
      planUnknown: false,
      settingsError: null,
      settingsLastSyncedMs: now - 3 * MIN,
      state: "ready",
    },
    sync_devices: [
      { comparisonCode: null, enrolledAtUnixMs: now - 90 * DAY, id: "d1", name: "Alex's laptop", platform: "windows", publicKey: "pk-pc1", status: "approved", thisDevice: true },
      { comparisonCode: null, enrolledAtUnixMs: now - 60 * DAY, id: "d2", name: "Alex's MacBook", platform: "macos", publicKey: "pk-mac1", status: "approved", thisDevice: false },
      { comparisonCode: "4821", enrolledAtUnixMs: now - 20 * MIN, id: "d3", name: "Alex's tablet", platform: "windows", publicKey: "pk-tab1", status: "pending", thisDevice: false },
    ],
    retention_settings: { acknowledged: true, lastRun: { capturesDeleted: 214, conversationsDeleted: 0, finishedUnixMs: now - 9 * HOUR, meetingsDeleted: 0, orphanBytes: 0, orphansDeleted: 0 }, settings: { captureDays: 90, conversationDays: null, meetingDays: null } },
    retention_preview: { captures: 0, conversations: 0, meetings: 0, orphanBytes: 0, orphans: 0 },
    capture_status: { enabled: true, excluded: 41, lastApp: "Microsoft Excel", lastError: null, lastStoredAtMs: now - 4 * MIN, ocrProblem: null, ocrProblemAtUnixMs: null, ocrRuns: 120, paused: null, running: true, stored: 1482, unchanged: 9120 },
    capture_settings: {
      excluded: ["1Password", "KeePassXC"],
      ocr: { available: ["en-US", "es-US", "fr-FR"], covered: true, profile: ["en-US"] },
      runningApps: [
        { appId: "excel", exe: "EXCEL.EXE", title: "Microsoft Excel" },
        { appId: "chrome", exe: "chrome.exe", title: "Google Chrome" },
        { appId: "outlook", exe: "OUTLOOK.EXE", title: "Outlook" },
      ],
    },
    hotkey_status: { error: null, registered: true, shortcut: "Control+Alt+Space" },
    storage_overview: { days: [30, 60, 90, 180, 365], dbFreeBytes: 40_000_000, defaultDays: 90, footprint: { database: 1_100_000_000, models: 3_300_000_000, other: 90_000_000, recordings: 2_400_000_000, total: 6_890_000_000 }, lowDisks: [] },
    storage_free_up_options: { capturesBytes: 180_000_000, capturesCount: 4210, compactBytes: 40_000_000, compressBytes: 700_000_000, days: 90, recordingsBytes: 1_300_000_000 },
    encryption_card: { badge: "encrypted", cleanupRunning: false, fastSearch: { available: true, enabled: true }, notes: [], pendingNextLaunch: false, plainBackupPresent: false, showsStoreTools: true },
    foreign_files_list: { bytes: 0, count: 0 },
    vault_settings: { enabled: true, failed: false, lastUpdatedUnixMs: now - 20 * MIN, path: "C:\\Users\\alex\\Documents\\Whimbrel vault" },
    delete_everything_preview: { dataBytes: 6_890_000_000, dataFolder: "C:\\Users\\alex\\AppData\\Local\\Whimbrel", endsMeeting: null, googleAccounts: ["alex.rivera@contoso.example.test"], snapshotsBytes: 120_000_000, snapshotsFolder: "C:\\Users\\alex\\Pictures\\Whimbrel slides", snapshotsRefused: null, storeOpen: true, syncOn: true, syncSignedIn: true },
    mcp_server_state: { address: "http://127.0.0.1:47811/mcp", enabled: true, error: null, status: "running" },
    mcp_clients: [
      { createdUnixMs: now - 20 * DAY, id: 1, name: "Claude Desktop", revokedUnixMs: null, scopes: ["meetings:read", "captures:read"] },
      { createdUnixMs: now - 45 * DAY, id: 2, name: "Old test client", revokedUnixMs: now - 30 * DAY, scopes: ["meetings:read"] },
    ],
    key_check: { message: "The key works.", models: ["claude-sonnet", "claude-haiku"], status: "valid" },
    usage_rows: [
      { estimatedCostCents: 3, inputTokens: 5200, local: false, model: "claude-haiku", outputTokens: 480, provider: "anthropic", task: "Meeting summary", tsUnixMs: now - 3 * HOUR },
      { estimatedCostCents: 9, inputTokens: 21000, local: false, model: "claude-sonnet", outputTokens: 900, provider: "anthropic", task: "Ask", tsUnixMs: now - 5 * HOUR },
      { estimatedCostCents: 0, inputTokens: 3000, local: true, model: "llama3", outputTokens: 300, provider: "ollama", task: "Daily recap", tsUnixMs: now - 20 * HOUR },
    ],
    provider_models: ["claude-sonnet", "claude-haiku"],
    skin_sources: { chat: true, generate: false, provider: "anthropic", search: false },
    skin_chat_load: [],
    skins_revision: 1,

    // --- the small windows --------------------------------------------------------------------------------------------------
    tray_status: { captureEnabled: true, line: "Capturing · 1,482 today", recordingPaused: false, recordingStartedUnixMs: null, syncError: false, syncText: "Synced 10:20 AM", googleSignInExpired: false },
    alert_status: {
      autoStart: false,
      countdownSeconds: 30,
      meeting: { appId: "zoom", attendees: ["Sam Okafor", "Priya Shah"], id: "det-1", promptedUnixMs: now, rootPid: 4242, source: "Zoom", startUnixMs: now, title: "Northwind security review" },
    },
    question_alert_status: [
      { body: "\"Does Northwind need SSO on day one?\"", id: "q1", meetingId: 1, origin: "live", sourceMeetingId: 1, subtitle: "Sam Okafor asked, 4 minutes ago", title: "Someone asked you a question" },
    ],
    budget_alert_status: [{ budgetCents: 500, month: new Date(now).toISOString().slice(0, 7), provider: "anthropic", spentCents: 412, threshold: 80 }],
    disk_alert_status: [{ freeBytes: 4_800_000_000, volume: "C:" }],
  };
};

/** Plain data the dynamic answers read (the page has it as `globalThis.__fx`). */
export const helpers = (now: number) => ({
  find: findData(now),
  conv: convData(now),
  northwind: NORTHWIND,
  acme: ACME,
  prep: { brief: prepBrief(now), eventId: "e1", startUnixMs: now + 9 * MIN, title: "Northwind security review" },
  workspaces: Object.fromEntries([1, 2, 3, 4, 5].map((id) => [id, workspace(now, id)])),
  topicPages: Object.fromEntries([1, 2, 3, 4, 5].map((id) => [id, topicPage(now, id)])),
  captures: {
    101: { ...capture(101, "Microsoft Excel", "Northwind pricing sheet.xlsx - Excel", "", "accessibility", now - 25 * MIN), url: null, text: "Northwind pricing sheet\n\nSeats: 60\nTerm: 12 months\nVolume discount: 5%\nList price per seat: 12.00\nTotal: 41,040.00\n\nNotes: pilot seats (25) billed at the discounted rate only if Northwind signs by month end." },
    102: { ...capture(102, "Outlook", "Re: Northwind renewal quote - Outlook", "", "accessibility", now - 2 * HOUR), url: null, text: "Re: Northwind renewal quote\n\nHi Alex,\n\nThanks for the call this morning. Could you confirm whether the discount applies to the pilot seats? Priya is reviewing the security answers and we expect to hear back this week.\n\nDana" },
    103: { ...capture(103, "Slack", "#northwind-renewal - Slack", "", "ocr", now - 26 * HOUR, MAC), url: null, text: "Sam Okafor 10:12\npilot seat count is 25 until security signs off, I'll confirm with their IT lead" },
  } as Record<number, unknown>,
  episodes: Object.fromEntries(libraryRecent(now).filter((r) => r.kind === "episode").map((r) => [(r as { id: number }).id, r])),
  searchRows: [
    { kind: "transcript", id: 301, meetingId: 3, meetingTitle: "Acme onboarding call", snippet: "...we can have the data agreement signed by the end of next week if Dana sends it today...", speaker: "Priya Shah", startCs: 125_000 },
    { kind: "capture", ...capture(102, "Outlook", "Re: Northwind renewal quote - Outlook", "Re: Northwind renewal quote. Hi Alex, could you confirm whether the discount applies to the pilot seats?", "accessibility", now - 2 * HOUR) },
  ],
  conversations: {
    1: {
      exchanges: [
        {
          answer:
            "Northwind asked for three things in the renewal check-in:\n\n- A **12-month term** with a **5% volume discount** for 60 seats.\n- The pilot to stay at **25 seats** until their security review closes.\n- A revised quote from you by **Friday**.\n\nDana also asked by email whether the discount applies to the pilot seats; that is still open.",
          citations: [
            { item: { id: 1, kind: "transcript" }, label: "Northwind renewal check-in", open: { id: 1, kind: "meeting" }, tsUnixMs: now - 3 * HOUR },
            { item: { id: 102, kind: "capture" }, label: "Re: Northwind renewal quote - Outlook", open: { id: 102, kind: "capture" }, tsUnixMs: now - 2 * HOUR },
            { item: { id: 101, kind: "capture" }, label: "Northwind pricing sheet.xlsx - Excel", open: { id: 101, kind: "capture" }, tsUnixMs: now - 25 * MIN },
          ],
          createdAtUnixMs: now - 2 * HOUR,
          model: "claude-sonnet",
          query: "What did Northwind ask for?",
        },
      ],
      id: 1,
      starred: true,
      title: "What did Northwind ask for?",
    },
  } as Record<number, unknown>,
  transcript: {
    canName: true,
    invitees: [{ email: "sam@contoso.example.test", name: "Sam Okafor" }, { email: "priya@contoso.example.test", name: "Priya Shah" }, { email: "dana@northwind.example.test", name: "Dana Whitfield" }],
    more: false,
    source: "file",
    turns: [
      { guess: false, label: "Alex Rivera", personKey: null, speaker: "Me", speakerId: null, text: "Thanks everyone for making time. I want to close out the renewal today if we can.", timestamp: "00:00:04" },
      { guess: false, label: "Dana Whitfield", personKey: "p-dana", speaker: "Dana Whitfield", speakerId: 1, text: "We are happy with the pilot. We would like to go to sixty seats, but our security review is still open.", timestamp: "00:00:19" },
      { guess: false, label: "Sam Okafor", personKey: "p-sam", speaker: "Sam Okafor", speakerId: 2, text: "Then let's keep the pilot at twenty-five seats until it closes, and offer a twelve-month term with a five percent volume discount.", timestamp: "00:00:41" },
      { guess: true, label: "Priya Shah", personKey: "p-priya", speaker: "Speaker 3", speakerId: 3, text: "I can turn around the security answers by Thursday if Dana sends the questionnaire today.", timestamp: "00:01:08" },
      { guess: false, label: "Alex Rivera", personKey: null, speaker: "Me", speakerId: null, text: "Great. I will send the revised quote by Friday and Sam will confirm the seat count with your IT lead.", timestamp: "00:01:30" },
    ],
  },
});

/** Answers that depend on what the page asks. They read `globalThis.__fx` (`helpers` above). */
export const dynamic: Record<string, (args: any) => unknown> = {
  // A level that moves, so the meter draws a waveform (the page's Math.random is not pinned).
  meeting_audio_level: () => ({ mic: 0.08 + Math.random() * 0.25, system: 0.15 + Math.random() * 0.55 }),
  meeting_workspace: (a) => (globalThis as any).__fx.workspaces[a.id] ?? null,
  topic_page: (a) => (globalThis as any).__fx.topicPages[a.topicId] ?? null,
  topic_lookup: (a) => {
    const fx = (globalThis as any).__fx;
    return a.items.map((i: any) => (i.kind === "meeting" ? (i.id === 1 ? fx.northwind : i.id === 3 ? fx.acme : null) : /northwind/i.test(i.title) ? fx.northwind : /acme/i.test(i.title) ? fx.acme : null));
  },
  meeting_chips: (a) => {
    const fx = (globalThis as any).__fx;
    return a.ids.map((id: number) => ({ meetingId: id, prepKey: id === 1 ? "prep-1" : null, topic: id === 1 ? fx.northwind : id === 3 ? fx.acme : null }));
  },
  meeting_prep_brief: () => (globalThis as any).__fx.prep,
  library_capture: (a) => (globalThis as any).__fx.captures[a.id] ?? null,
  library_episode: (a) => (globalThis as any).__fx.episodes[a.id] ?? null,
  library_meeting: (a) => (globalThis as any).__fx.workspaces[a.id]?.row ?? null,
  library_search: () => (globalThis as any).__fx.searchRows,
  // The redesigned Home's filtered search answers captures only.
  library_search_filtered: () => (globalThis as any).__fx.searchRows.filter((r: any) => r.kind === "capture"),
  chat_conversation: (a) => (globalThis as any).__fx.conversations[a.id] ?? null,
  // Find (screen 5): results under the page's filters, and the saved conversations with their cited sources.
  library_find: (a) => {
    const fx = (globalThis as any).__fx.find;
    const f = a.filter ?? {};
    const q = String(a.query ?? "").trim().toLowerCase();
    let rows: any[] = fx.results.slice();
    if (q.includes("zzz")) rows = [];
    if (f.startUnixMs != null) rows = rows.filter((r) => r.tsUnixMs >= f.startUnixMs && r.tsUnixMs < f.endUnixMs);
    if (f.app) rows = rows.filter((r) => r.kind === "capture" && r.app === f.app);
    let mentions: any[] = [];
    if (f.person) {
      rows = rows.filter((r) => r.kind === "transcript" || r.kind === "snapshot" || r.kind === "thread");
      mentions = fx.mentions;
    }
    // No words: what was saved in the span, listed by time (today's blocks, or an older week a reopened conversation kept), still under the app.
    if (q === "" && f.startUnixMs != null) {
      rows = fx.today.concat(fx.results).filter((r: any) => r.tsUnixMs >= f.startUnixMs && r.tsUnixMs < f.endUnixMs);
      if (f.app) rows = rows.filter((r: any) => r.kind === "capture" && r.app === f.app);
    }
    if (a.order === "rank") rows = rows.slice().sort((x, y) => x.rank - y.rank);
    const counts: Record<string, number> = {};
    for (const r of rows) counts[r.kind] = (counts[r.kind] ?? 0) + 1;
    if (mentions.length) counts.mentions = mentions.length;
    return {
      results: rows.slice(a.offset, a.offset + a.limit),
      mentions: a.offset === 0 ? mentions : [],
      total: rows.length,
      mentionsTotal: mentions.length,
      counts,
      semanticUsed: fx.semantic,
      semanticAvailable: fx.semantic,
      more: false,
      capped: false,
    };
  },
  chat_conversation_sourced: (a) => (globalThis as any).__fx.find.conversations[a.id] ?? null,
  // Conversations (screen 6): a conversation's page, the topic page's list and a conversation's stored scope.
  conversation_page: (a) => (globalThis as any).__fx.conv.pages[a.id] ?? null,
  conversation_scope: (a) => (globalThis as any).__fx.conv.pages[a.id]?.scope ?? { kind: "everything" },
  topic_conversations_list: (a) => (globalThis as any).__fx.conv.byTopic[a.topicId] ?? [],
  chat_conversation_find_filter: (a) => (globalThis as any).__fx.find.kept[a.id] ?? null,
  meeting_transcript: () => (globalThis as any).__fx.transcript,
};

// ---- scenario patches (passed to open() for one screen) ---------------------------------------------------------------

/** A meeting being recorded right now: the Meetings tab's recording card, the workspace's live view and live transcript. */
export const recordingPatch = (now: number): Record<string, unknown> => {
  const start = now - 7 * MIN - 12_000;
  const row = meetingRow(now, 6, 0, "Weekly product sync", { endUnixMs: null, hasSummary: false, status: "recording", tsUnixMs: start, actionItemCount: 0, attendeeCount: 3 });
  return {
    meeting_recording_status: { degraded: null, failure: null, meetingId: 6, paused: false, pausedTotalMs: 0, processing: null, resuming: false, startedUnixMs: start, state: "recording" },
    meeting_list: [row, ...meetings(now).slice(0, 4)],
    meeting_most_relevant: 6,
    meeting_chips: [],
    meeting_workspace: {
      actionItems: [], attendees: ATTENDEES, canChooseLanguage: false, conferenceLink: null, decisions: [], decisionsAreEdited: false, elapsedSeconds: 432,
      hasTranscript: false, openQuestions: [], openQuestionsAreEdited: false, personalNotes: "Ask Sam about the Q1 budget.", phase: "during", processingStage: null,
      readOnly: false, row, spokenLanguage: null, summary: null, summaryFailure: null, summaryFallback: null, summaryIsEdited: false, transcription: null, trigger: "calendar",
    },
    meeting_live_transcript: {
      enabled: true,
      lines: [
        { id: 1, speaker: "Sam Okafor", startCs: 1_200, text: "Okay, let's start with the Northwind renewal, since Dana joins at the half hour." },
        { id: 2, speaker: "Me", startCs: 3_400, text: "The quote is drafted. I want to confirm the seat count before it goes out." },
        { id: 3, speaker: "Priya Shah", startCs: 6_100, text: "The pilot is at twenty-five seats, and security still has two open questions." },
        { id: 4, speaker: "Sam Okafor", startCs: 9_800, text: "Then we hold the quote until Thursday. Alex, can you own the follow-up with Dana?" },
      ],
    },
    meeting_live_alerts: [
      { askedAt: "4:05", asker: "Sam Okafor", createdAtUnixMs: now - 3 * MIN, draft: null, evidence: [{ label: "Northwind pricing sheet.xlsx", tsUnixMs: now - 25 * MIN }], id: 1, quote: "Alex, can you own the follow-up with Dana?", restatement: "Sam asked you to own the follow-up with Dana.", sourcesElsewhere: false },
    ],
    today_schedule: { events: [], earlierToday: [row], recording: true },
  };
};

/** The live meeting screen (screen 3): the recording from `recordingPatch`, 14 minutes in, in the state a picture needs.
 *  `paused`: paused for the last 2 minutes. `resuming`: resume asked for, audio not back yet. `problem`: a stream gave
 *  up. `processing`: stopped, the summary being written. `starting`: the row is still being written. `readOnly`: another
 *  device's meeting. `noTranscript`: live transcript off. */
export type LiveKind = "marks" | "recording" | "paused" | "resuming" | "problem" | "processing" | "starting" | "readOnly" | "noTranscript";

export const livePatch = (now: number, kind: LiveKind = "recording"): Record<string, unknown> => {
  const base = recordingPatch(now);
  const start = now - 14 * MIN - 8_000;
  const row = { ...(base.meeting_list as any[])[0], tsUnixMs: start, title: "Pricing review", attendeeCount: 3 };
  const status = (over: Record<string, unknown>) => ({ degraded: null, failure: null, meetingId: 6, paused: false, pausedSinceUnixMs: null, pausedTotalMs: 0, processing: null, resuming: false, startedUnixMs: start, state: "recording", ...over });
  const workspace = { ...(base.meeting_workspace as any), row, attendees: ["Sam Okafor", "Priya Shah"], elapsedSeconds: 848, personalNotes: "sam wants usage tier\nannual discount 15 or 20?\npriya: check churn on starter first\nsam: start at 15, then" };
  const out: Record<string, unknown> = { ...base, meeting_marker_list: [], meeting_list: [row, ...(base.meeting_list as any[]).slice(1)], meeting_workspace: workspace, meeting_recording_status: status({}) };
  out.today_schedule = { events: [{ actionItemCount: 0, attendees: ["Dana Whitfield", "Sam Okafor"], endUnixMs: now + 3.5 * 3_600_000, eventId: "evt-design", hasSummary: false, prepKey: null, recordedMeetingId: null, startUnixMs: now + 3 * 3_600_000, title: "Design sync" }], earlierToday: [row], recording: true };
  if (kind === "paused") out.meeting_recording_status = status({ paused: true, pausedSinceUnixMs: now - 2 * MIN, pausedTotalMs: 2 * MIN });
  if (kind === "resuming") out.meeting_recording_status = status({ resuming: true, pausedTotalMs: 2 * MIN });
  if (kind === "problem") out.meeting_recording_status = status({ degraded: "The microphone isn't delivering audio, so this meeting records system audio only." });
  if (kind === "processing") {
    out.meeting_recording_status = { ...status({ meetingId: null, startedUnixMs: null, state: "idle" }), processing: { label: "Writing the summary", meetingId: 6, stage: "summarizing" } };
  }
  if (kind === "starting") out.meeting_recording_status = status({ meetingId: null, startedUnixMs: null, state: "starting" });
  if (kind === "readOnly") {
    out.meeting_recording_status = status({ meetingId: null, startedUnixMs: null, state: "idle" });
    out.meeting_workspace = { ...workspace, readOnly: true };
  }
  if (kind === "marks") out.meeting_marker_list = [{ atCs: 24_800, createdUnixMs: now - 10 * MIN, id: 1, label: "priya: check churn on starter first", meetingId: 6 }, { atCs: 40_100, createdUnixMs: now - 7 * MIN, id: 2, label: null, meetingId: 6 }];
  if (kind === "noTranscript") out.meeting_live_transcript = { enabled: false, lines: [] };
  if (kind !== "noTranscript") {
    out.meeting_live_transcript = {
      enabled: true,
      lines: [
        { id: 1, speaker: "Priya Shah", startCs: 72_000, text: "So a usage-based tier, on top of seats?" },
        { id: 2, speaker: "Sam Okafor", startCs: 75_300, text: "Yes. Small base, then pay for what you use. Starter would like it most." },
        { id: 3, speaker: "Me", startCs: 78_000, text: "Do we know what that does to churn on Starter?" },
        { id: 4, speaker: "Sam Okafor", startCs: 81_000, text: "On the annual discount, I would start at 15 and see..." },
      ],
    };
    out.meeting_live_alerts = [];
  }
  return out;
};


// ---- the finished note (screen 4): the Northwind renewal call, made up -------------------------------------------------

/** What a picture of the finished note needs. `withSources`: the margin's numbered sources (the default). `noSources`: an old
 *  meeting, plain lines. `processing`: the summary being written. `readOnly`: another device's meeting. `noSummary`: a
 *  transcript only. `failed`: the summary did not finish. */
export type NoteKind = "withSources" | "noSources" | "processing" | "readOnly" | "noSummary" | "failed" | "noTranscript";

type NoteLine = { id: number; startCs: number; endCs: number; speaker: string; label: string; speakerId: number | null; personKey: string | null; guess?: boolean; text: string };
const ME = { speaker: "Me", label: "Alex Rivera", speakerId: null, personKey: null };
const JO = { speaker: "Speaker 2", label: "Jo (Northwind)", speakerId: 1, personKey: "p-jo" };
const DANA = { speaker: "Speaker 3", label: "Dana Whitfield", speakerId: 2, personKey: "p-dana" };
const SAM = { speaker: "Speaker 4", label: "Sam Okafor", speakerId: 3, personKey: "p-sam" };
const NOTE_LINES: NoteLine[] = [
  { id: 101, startCs: 1_200, endCs: 6_800, ...ME, text: "Thanks for making time, everyone. I want to cover the renewal terms first, then seats, then the security questionnaire." },
  { id: 102, startCs: 37_200, endCs: 40_400, ...JO, text: "If the price holds, we can do two years." },
  { id: 103, startCs: 41_000, endCs: 44_600, ...ME, text: "Good. Two years at the current price works for us." },
  { id: 104, startCs: 58_000, endCs: 63_000, ...DANA, text: "We expect fifteen more seats by Q2. I will confirm the date." },
  { id: 105, startCs: 66_000, endCs: 71_000, ...SAM, guess: true, text: "Then we are at fifty-five. I will note it against the renewal." },
  { id: 106, startCs: 90_000, endCs: 94_500, ...ME, text: "Jo, anything on your side before we get to security?" },
  { id: 107, startCs: 110_500, endCs: 114_800, ...JO, text: "There are still a few open items on the questionnaire." },
  { id: 108, startCs: 120_000, endCs: 126_000, ...DANA, text: "I can pick those up. Who sent them to us in the first place?" },
  { id: 109, startCs: 140_000, endCs: 145_000, ...ME, text: "That was Priya. I will ask her for the original file." },
  { id: 110, startCs: 173_000, endCs: 176_000, ...SAM, text: "I will get the quote to you by Thursday." },
  { id: 111, startCs: 177_100, endCs: 181_000, ...DANA, text: "I will answer the open items by Friday." },
  { id: 112, startCs: 186_200, endCs: 189_000, ...ME, text: "I will check the seat price before we send it." },
  { id: 113, startCs: 190_000, endCs: 192_000, ...JO, text: "Thank you all. Talk next week." },
];

const NOTE_ROW = (now: number, extra: Record<string, unknown> = {}) =>
  meetingRow(now, 7, 26, "Northwind renewal call", { actionItemCount: 3, attendeeCount: 4, endUnixMs: now - 26 * HOUR + 32 * MIN, ...extra });

/** The finished note for `kind`, as answers for `open()`. Times are centiseconds into the call; the sources point at lines above. */
export const notePatch = (now: number, kind: NoteKind = "withSources"): Record<string, unknown> => {
  const row = NOTE_ROW(now, kind === "readOnly" ? { device: MAC } : {});
  const src = (n: number) => {
    const l = NOTE_LINES.find((x) => x.id === n)!;
    return { segmentId: l.id, speaker: l.label, startCs: l.startCs, text: l.text };
  };
  const decisions = ["Northwind will renew for two years if the price stays the same.", "Seats grow from 40 to 55 in Q2. Dana will confirm the start date."];
  const questions = ["Northwind's security questionnaire still has open items, and the renewal date is close."];
  const items = [
    { completed: false, id: 21, source: "generated", text: "Send the two-year renewal quote to Jo." },
    { completed: false, id: 22, source: "generated", text: "Answer the open items on the questionnaire." },
    { completed: false, id: 23, source: "generated", text: "Check that 55 seats still fits the current price." },
  ];
  const written = kind !== "processing" && kind !== "noSummary" && kind !== "failed" && kind !== "noTranscript";
  const workspace = {
    actionItems: written ? items : [],
    attendees: ["Sam Okafor", "Dana Whitfield", "Jo (Northwind)", "Alex Rivera"],
    canChooseLanguage: true,
    conferenceLink: null,
    decisions: written ? decisions : [],
    decisionsAreEdited: false,
    elapsedSeconds: null,
    hasTranscript: kind !== "noTranscript",
    openQuestions: written ? questions : [],
    openQuestionsAreEdited: false,
    personalNotes: "jo ok with 2 yr term if price holds\nsecurity questionnaire?? who owns it\nseats 40 to 55 in Q2",
    phase: "after",
    processingStage: kind === "processing" ? "Writing the summary" : null,
    readOnly: kind === "readOnly",
    row,
    spokenLanguage: null,
    summary: written
      ? "Alex, Sam and Dana spoke with Jo from Northwind about renewing. Northwind will renew for two years if the price stays the same, and wants to grow from 40 to 55 seats in Q2. The security questionnaire still has open items."
      : null,
    summaryFailure: kind === "failed" ? "The model did not answer in time." : null,
    summaryFallback: null,
    summaryIsEdited: false,
    transcription: null,
    trigger: "calendar",
  };
  const sources =
    kind === "withSources" || kind === "readOnly"
      ? {
          decisions: [{ text: decisions[0], source: src(102) }, { text: decisions[1], source: src(104) }],
          openQuestions: [{ text: questions[0], source: src(107) }],
          actionItems: [
            { itemId: 21, text: items[0].text, owner: "Sam Okafor", due: "Thu, Nov 19", source: src(110) },
            { itemId: 22, text: items[1].text, owner: "Dana Whitfield", due: "Fri, Nov 20", source: src(111) },
            { itemId: 23, text: items[2].text, owner: "Alex Rivera", due: "Tue, Nov 24", source: src(112) },
          ],
        }
      : null;
  return {
    meeting_list: [row, ...meetings(now).slice(0, 4)],
    meeting_chips: [],
    meeting_workspace: workspace,
    meeting_seed_action_items: workspace.actionItems,
    meeting_note_sources: sources,
    meeting_trim_info: { lengthMs: 32 * MIN, offered: true },
    meeting_marker_list: [{ atCs: 61_000, createdUnixMs: now - 26 * HOUR + 11 * MIN, id: 31, label: "security questionnaire?? who owns it", meetingId: 7 }, { atCs: 147_000, createdUnixMs: now - 26 * HOUR + 25 * MIN, id: 32, label: null, meetingId: 7 }],
    meeting_transcript_lines: {
      canName: true,
      invitees: [{ email: "sam@contoso.example.test", name: "Sam Okafor" }, { email: "dana@northwind.example.test", name: "Dana Whitfield" }],
      total: kind === "noTranscript" ? 0 : NOTE_LINES.length,
      lines: (kind === "noTranscript" ? [] : NOTE_LINES).map((l) => ({ ...l, endCs: kind === "noSources" ? null : l.endCs, guess: l.guess === true })),
    },
    today_schedule: { events: [], earlierToday: [], recording: false },
  };
};


// ---- Find and the sourced Ask (screen 5) -----------------------------------------------------------------------------------

/** A fixed day and time (the pictures are taken on Wed 18 Nov 2026), so the made-up week never moves. */
const at = (day: number, hour: number, minute: number) => Date.UTC(2026, 10, day, hour, minute);

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const found = (kind: string, id: number, over: Record<string, unknown>): any => ({ kind, found: true, id, title: "", ...over });

const FIND_LINE = found("transcript", 4101, {
  meetingId: 8,
  meetingStartUnixMs: at(10, 15, 0),
  meetingTitle: "Pricing review",
  speaker: "Sam Okafor",
  startCs: 87_200,
  text: "I would add a usage tier. Seats alone do not fit how Starter customers work.",
  title: "Pricing review",
  tsUnixMs: at(10, 15, 14),
});
const FIND_SLACK = found("capture", 5002, {
  app: "slack.exe",
  device: THIS_PC,
  text: "# pricing\nSam Okafor 4:10 PM\nBefore we pick numbers for a usage tier, can someone pull churn on Starter?\nPriya Shah 4:14 PM\nOn it. I will have it Monday.\nSam Okafor 4:21 PM\nThanks. We can look at it Tuesday.",
  title: "Thread in #pricing",
  tsUnixMs: at(12, 16, 10),
});
const FIND_EXCEL = found("capture", 5003, {
  app: "excel.exe",
  device: THIS_PC,
  text: "Pricing v3.xlsx\nPlan  Seats  Rate  Price\nStarter  5  $12  $60\nTeam  25  $10  $250\nBusiness  100  $8  $800\nUsage tier\nRate cell for Usage tier is empty.",
  title: "Window: Pricing v3.xlsx",
  tsUnixMs: at(13, 11, 2),
});
const FIND_THREAD = found("thread", 8801, {
  sender: "Priya Shah",
  text: "Re: Starter churn numbers. Sam, the Starter churn for Q3 is attached. I read it as 4.1% a month, but I would check it against billing before we quote it.",
  title: "Re: Starter churn numbers",
  tsUnixMs: at(12, 9, 30),
});
const FIND_WORD = found("capture", 5004, {
  app: "winword.exe",
  device: THIS_PC,
  text: "Starter plan notes\nSeat pricing works for teams that grow in steps. Starter customers add and drop people month to month, so a usage tier could fit them better.",
  title: "Starter plan notes.docx - Word",
  tsUnixMs: at(11, 14, 5),
});
const FIND_SLIDE = found("snapshot", 6201, {
  meetingId: 8,
  meetingStartUnixMs: at(10, 15, 0),
  meetingTitle: "Pricing review",
  text: "Pricing options: seats only, seats plus usage, usage only. Churn on Starter by plan.",
  title: "Pricing review",
  tsUnixMs: at(10, 15, 0),
});
const FIND_EPISODE = found("episode", 201, {
  app: "chrome.exe",
  device: THIS_PC,
  text: "Read three public pricing pages and compared how each charges for usage.",
  title: "Pricing research in Chrome",
  tsUnixMs: at(10, 9, 40),
});
const FIND_RUN = found("routine_run", 7, { text: "Weekly digest: pricing came up in two meetings and one thread.", title: "Weekly digest", tsUnixMs: at(9, 18, 0) });

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const withRank = (rows: any[]): any[] => rows.map((r, i) => ({ ...r, rank: i }));

function findData(now: number) {
  // Newest first, as the page draws them; `rank` is the order "best matches first" uses.
  const results = withRank([FIND_EXCEL, FIND_SLACK, FIND_THREAD, FIND_LINE, FIND_WORD, FIND_SLIDE, FIND_EPISODE, FIND_RUN]).sort((a, b) => (b.tsUnixMs as number) - (a.tsUnixMs as number));
  const ranked = [FIND_LINE, FIND_SLACK, FIND_EXCEL, FIND_THREAD, FIND_WORD, FIND_SLIDE, FIND_EPISODE, FIND_RUN];
  for (const r of results) (r as { rank: number }).rank = ranked.findIndex((x) => x.id === r.id);
  const mentions = [
    found("capture", 5101, { app: "outlook.exe", device: THIS_PC, text: "Draft to Priya: Sam Okafor asked for the Starter churn numbers before we set the usage tier.", title: "Draft - Outlook", tsUnixMs: at(11, 17, 20) }),
    found("capture", 5102, { app: "chrome.exe", device: MAC, text: "Notes for Monday: ask Sam Okafor whether Starter customers would pay for a usage tier.", title: "Monday notes - Chrome", tsUnixMs: at(9, 16, 45) }),
  ];
  const sources = [
    { n: 1, card: FIND_LINE },
    { n: 2, card: FIND_SLACK },
    { n: 3, card: FIND_EXCEL },
    { n: 4, card: FIND_THREAD },
    { n: 5, card: FIND_WORD },
  ];
  const answer =
    "Sam wants a usage-based tier added alongside seat pricing, and he thinks Starter customers would like it most. [1] Before anyone sets numbers, he asked for churn data on Starter. [2] His draft in Pricing v3 has a Usage tier row, but the rate is still blank. [3] I did not find a decision on the annual discount.";
  const gone = found("capture", 5999, { found: false });
  const legacy = {
    answer:
      "Northwind asked for three things in the renewal check-in:\n\n- A **12-month term** with a **5% volume discount** for 60 seats.\n- The pilot to stay at **25 seats** until their security review closes.\n- A revised quote from you by **Friday**.",
    citations: [
      { item: { id: 1, kind: "transcript" }, label: "Northwind renewal check-in", open: { id: 1, kind: "meeting" }, tsUnixMs: null },
      { item: { id: 102, kind: "capture" }, label: "Re: Northwind renewal quote - Outlook", open: { id: 102, kind: "capture" }, tsUnixMs: at(18, 8, 20) },
    ],
    createdAtUnixMs: at(18, 8, 22),
    legacy: true,
    model: "claude-sonnet",
    query: "What did Northwind ask for?",
    sources: [],
  };
  // What the day's memory blocks hold, listed by time when a block's Open filters by its span (no words).
  const today = withRank([
    found("capture", 5203, { app: "outlook.exe", device: THIS_PC, text: "Re: Northwind renewal quote. Hi Alex, could you confirm whether the discount applies to the pilot seats?", title: "Re: Northwind renewal quote - Outlook", tsUnixMs: now - 60 * MIN }),
    found("capture", 5202, { app: "slack.exe", device: THIS_PC, text: "# pricing\nPriya Shah: starter churn is back, I will post it in the doc", title: "#pricing - Slack", tsUnixMs: now - 70 * MIN }),
    found("capture", 5201, { app: "excel.exe", device: THIS_PC, text: "Pricing v3.xlsx\nStarter  5  $12  $60\nTeam  25  $10  $250\nUsage tier", title: "Pricing v3.xlsx - Excel", tsUnixMs: now - 88 * MIN }),
  ]);
  return {
    results,
    today,
    mentions,
    sources,
    answer,
    semantic: true,
    // The filters a Find conversation kept (Dates and Apps, never a person): conversation 1 was asked over Excel, Nov 9 to Nov 15.
    kept: {
      1: { start_unix_ms: at(9, 0, 0), end_unix_ms: at(16, 0, 0), app_bundle_id: "excel.exe", people_left_out: false },
    } as Record<number, unknown>,
    conversations: {
      1: {
        exchanges: [
          {
            answer,
            answeredLocally: true,
            citations: [],
            createdAtUnixMs: at(17, 9, 40),
            leftOut: 0,
            legacy: false,
            model: "gemma4:12b",
            provider: "ollama",
            query: "What did Sam say about pricing last week?",
            sources: [{ n: 1, card: FIND_LINE }, { n: 2, card: FIND_SLACK }, { n: 3, card: FIND_EXCEL }],
          },
        ],
        id: 1,
        starred: true,
        title: "What did Sam say about pricing last week?",
      },
      2: { exchanges: [legacy], id: 2, starred: false, title: "What did Northwind ask for?" },
      3: {
        exchanges: [
          {
            answer: "Priya owns the data agreement. [1] The signing date was still open. [2]",
            answeredLocally: false,
            citations: [],
            createdAtUnixMs: at(16, 13, 0),
            leftOut: 1,
            legacy: false,
            model: "claude-sonnet",
            provider: "anthropic",
            query: "Who owns the Acme data agreement?",
            sources: [{ n: 1, card: FIND_THREAD }, { n: 2, card: gone }],
          },
        ],
        id: 3,
        starred: false,
        title: "Who owns the Acme data agreement?",
      },
    } as Record<number, unknown>,
  };
}

const ROUTE_HOSTED = { destination: "hosted", local: false, mayFallBack: true, model: "claude-sonnet", provider: "anthropic", reason: "preferred_provider", requireLocal: false };
const ROUTE_LOCAL = { destination: "local", local: true, mayFallBack: false, model: "gemma4:12b", provider: "ollama", reason: "preferred_is_local", requireLocal: true };

export type FindKind = "default" | "local" | "setup" | "noSearchModel";

/** What a picture of Find needs: the conversations the panel lists, where an ask would be answered, and (for `setup`) an AI setup with no key. */
export const findPatch = (now: number, kind: FindKind = "default"): Record<string, unknown> => {
  const out: Record<string, unknown> = {
    chat_conversations: [
      { id: 1, starred: true, title: "What did Sam say about pricing last week?", updatedAtUnixMs: at(17, 9, 40) },
      { id: 2, starred: false, title: "What did Northwind ask for?", updatedAtUnixMs: at(18, 8, 22) },
      { id: 3, starred: false, title: "Who owns the Acme data agreement?", updatedAtUnixMs: at(16, 13, 0) },
    ],
    // The people the People chip offers: the attendees of today's and the coming events.
    today_schedule: {
      events: [{ eventId: "e1", title: "Pricing sync", startUnixMs: now + 9 * MIN, endUnixMs: now + 39 * MIN, attendees: ["Sam Okafor", "Priya Shah", "Dana Whitfield"], recordedMeetingId: null }],
      earlierToday: [],
      recording: false,
    },
    meeting_list: [],
    library_capture_apps: [
      { app_bundle_id: "excel.exe", captures: 214 },
      { app_bundle_id: "slack.exe", captures: 160 },
      { app_bundle_id: "outlook.exe", captures: 98 },
    ],
  };
  if (kind === "local") out.chat_route_preview = ROUTE_LOCAL;
  if (kind === "noSearchModel") out.library_find = { results: findData(now).results, mentions: [], total: 8, mentionsTotal: 0, counts: {}, semanticUsed: false, semanticAvailable: false, more: false, capped: false };
  if (kind === "setup") {
    const base = answers(now).ai_settings as Record<string, unknown>;
    out.ai_settings = { ...base, keys: [] };
    out.chat_route_preview = { destination: "refused", local: false, mayFallBack: false, model: null, provider: null, reason: "no_provider", requireLocal: false };
    out.local_model_status = { canOffer: true, installed: false };
  }
  return out;
};

/** The events one scripted ask sends, each after `at` milliseconds (the page's fake `chat_ask_sourced` plays them). `answer`: a streamed local
 *  answer that finishes. `stream`: the same, held part-way. `failed`: the ask refuses because the answer may not leave this PC. */
export type AskScript = "answer" | "stream" | "failed" | "rich";

/** An answer with a list, inline code, a fenced block, adjacent marks and a number that is not a source: only the marks on text become marks. */
const RICH_ANSWER =
  "Sam's draft has three parts. [1]\n\n- A **Usage** tier beside seat pricing. [2]\n- A churn check on Starter first. [1][3]\n- A blank rate cell, `C14`, still to fill. Read `rates[1]` as a placeholder, not a source.\n\n```\nUsage tier: rate = (tbd)\nplaceholder[1] = null\n```\n\nI did not find a decision on the annual discount (see [9]).";

export const askScript = (kind: AskScript): { at: number; event: Record<string, unknown> }[] => {
  const f = findData(0);
  const text = kind === "rich" ? RICH_ANSWER : f.answer;
  const words = text.split(" ");
  const started = { kind: "started", runId: "run-1", provider: "ollama", model: "gemma4:12b", context: [] };
  const sourced = { kind: "sourced", sources: f.sources, provider: "ollama", answeredLocally: true };
  if (kind === "failed") return [{ at: 60, event: { kind: "failed", error: { kind: "provider_failure", reason: { kind: "local_model_missing" } }, partial: false } }];
  const cut = kind === "stream" ? 22 : words.length;
  // As on a real PC, a model thinks before its first word, longer than the page might wait after the shell's call returns (which it does at
  // once). An answer that finishes must arrive well after that, so a page that ends the question when the call returns is caught here.
  const first = kind === "stream" ? 120 : 1800;
  const steps: { at: number; event: Record<string, unknown> }[] = [
    { at: 40, event: started },
    { at: 70, event: sourced },
    ...words.slice(0, cut).map((w, i) => ({ at: first + i * 12, event: { kind: "delta", text: (i === 0 ? "" : " ") + w } })),
  ];
  if (kind === "stream") return steps;
  const end = first + cut * 12 + 40;
  return [
    ...steps,
    { at: end, event: { kind: "cited", cited: [1, 2, 3], answeredLocally: true, provider: "ollama", leftOut: 0 } },
    { at: end + 10, event: { kind: "completed", answer: text, citations: [{ kind: "transcript", id: 4101 }, { kind: "capture", id: 5002 }, { kind: "capture", id: 5003 }], conversationId: 9, narrowed: false, leftOut: null } },
    { at: end + 60, event: { kind: "titled", conversationId: 9, title: "What did Sam say about pricing last week?" } },
  ];
};


// ---- Conversations (screen 6) ------------------------------------------------------------------------------------------------

const CONV_TOPIC_ANSWER =
  "Three things are open with Northwind. [1] The revised quote is yours to send by Friday. [2] Dana still owes the signed data agreement. [1] The security review has no date yet. [3]";

function convData(now: number) {
  const f = findData(now);
  const everything = f.conversations[1] as { exchanges: Record<string, unknown>[]; title: string };
  const exchange = (query: string, answer: string, sources: unknown[], over: Record<string, unknown> = {}) => ({
    answer,
    answeredLocally: true,
    citations: [],
    createdAtUnixMs: at(17, 9, 40),
    leftOut: 0,
    legacy: false,
    model: "gemma4:12b",
    provider: "ollama",
    query,
    sources,
    cited: (sources as { n: number }[]).map((x) => x.n),
    ...over,
  });
  const base = { localOnly: [] as boolean[], citedTopics: [] as unknown[], topics: [NORTHWIND, ACME, topicRef(3, "Q1 planning")], topic: null as unknown, topicSetByUser: false };
  const pages: Record<number, unknown> = {
    // About everything, its sources mixed: no topic, so "Add to topic" stands in the chip's place.
    1: {
      ...base,
      conversation: { id: 1, starred: true, title: everything.title, exchanges: [{ ...everything.exchanges[0], cited: [1, 2, 3] }] },
      localOnly: [false],
      scope: { kind: "everything" },
      citedTopics: [NORTHWIND],
    },
    // About one topic: joined from the start, with a follow-up asked inside it.
    4: {
      ...base,
      conversation: {
        id: 4,
        starred: true,
        title: "What is still open with Northwind?",
        exchanges: [
          exchange("What is still open with Northwind?", CONV_TOPIC_ANSWER, [{ n: 1, card: FIND_THREAD }, { n: 2, card: FIND_SLACK }, { n: 3, card: FIND_LINE }]),
          exchange("Who owns the security review?", "Priya Shah owns the answers, and Sam will confirm the seat count with their IT lead. [1]", [{ n: 1, card: FIND_LINE }], { createdAtUnixMs: at(17, 9, 52) }),
        ],
      },
      localOnly: [false, false],
      scope: { kind: "topic", id: 1 },
      scopeTopicName: "Northwind",
      topic: NORTHWIND,
      citedTopics: [NORTHWIND, ACME],
    },
    // About one meeting: kept from its note, keeps the meeting's scope.
    5: {
      ...base,
      conversation: {
        id: 5,
        starred: false,
        title: "What was decided in the pricing review?",
        exchanges: [exchange("What was decided in the pricing review?", "Sam argued for a usage tier beside seat pricing. [1] No rate was set; Priya will bring Starter churn on Monday. [1]", [{ n: 1, card: FIND_LINE }], { answeredLocally: false, model: "claude-sonnet", provider: "anthropic" })],
      },
      localOnly: [false],
      scope: { kind: "meeting", id: 8 },
      scopeMeetingTitle: "Pricing review",
      scopeMeetingStartUnixMs: at(10, 15, 0),
    },
  };
  const row = (id: number, title: string, starred: boolean, updated: number, topic: unknown, line: string | null) => ({ id, title, starred, updatedAtUnixMs: updated, topic, latestAnswerLine: line });
  return {
    pages,
    byTopic: {
      1: [
        { id: 4, starred: true, title: "What is still open with Northwind?", updatedAtUnixMs: at(18, 8, 40) },
        { id: 6, starred: false, title: "Did Northwind agree to 60 seats?", updatedAtUnixMs: at(14, 10, 5) },
      ],
    } as Record<number, unknown>,
    rows: [
      row(4, "What is still open with Northwind?", true, at(18, 8, 40), NORTHWIND, "Priya Shah owns the answers, and Sam will confirm the seat count with their IT lead."),
      row(1, everything.title, true, at(17, 9, 40), null, "Sam wants a usage-based tier added alongside seat pricing, and he thinks Starter customers would like it most."),
      row(5, "What was decided in the pricing review?", false, at(16, 15, 30), null, "Sam argued for a usage tier beside seat pricing."),
      row(6, "Did Northwind agree to 60 seats?", false, at(14, 10, 5), NORTHWIND, "They want sixty seats once their security review closes."),
      row(7, "When is the Acme data agreement due?", false, at(11, 13, 20), ACME, null),
    ],
  };
}

export type ConvKind = "default" | "empty";

/** What the Conversations section's pictures need: the list, a conversation's stored scope and where a keep or a topic Ask goes. */
export const convPatch = (now: number, kind: ConvKind = "default"): Record<string, unknown> => ({
  conversations_list: kind === "empty" ? [] : convData(now).rows,
  conversation_save_exchange: { conversationId: 4, topic: NORTHWIND, citedTopics: [NORTHWIND] },
  conversation_set_topic: { topic: ACME, setByUser: true },
  chat_route_preview: ROUTE_LOCAL,
});
