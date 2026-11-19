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
    { completed: false, id: 11, source: "summary", text: "Alex to send Northwind the revised renewal quote by Friday" },
    { completed: false, id: 12, source: "summary", text: "Sam to confirm the pilot seat count with Northwind's IT lead" },
    { completed: true, id: 13, source: "summary", text: "Dana to share last quarter's usage report" },
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
    redesign_enabled: false,
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
    capture_status: { enabled: true, excluded: 41, lastApp: "Microsoft Excel", lastError: null, lastStoredAtMs: now - 4 * MIN, ocrRuns: 120, paused: null, running: true, stored: 1482, unchanged: 9120 },
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
    tray_status: { captureEnabled: true, line: "Capturing · 1,482 today", recordingPaused: false, recordingStartedUnixMs: null },
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
    source: "whisper",
    turns: [
      { guess: false, label: "Alex Rivera", personKey: null, speaker: "Me", speakerId: 0, text: "Thanks everyone for making time. I want to close out the renewal today if we can.", timestamp: "0:04" },
      { guess: false, label: "Dana Whitfield", personKey: "p-dana", speaker: "Dana Whitfield", speakerId: 1, text: "We are happy with the pilot. We would like to go to sixty seats, but our security review is still open.", timestamp: "0:19" },
      { guess: false, label: "Sam Okafor", personKey: "p-sam", speaker: "Sam Okafor", speakerId: 2, text: "Then let's keep the pilot at twenty-five seats until it closes, and offer a twelve-month term with a five percent volume discount.", timestamp: "0:41" },
      { guess: true, label: "Priya Shah", personKey: "p-priya", speaker: "Speaker 3", speakerId: 3, text: "I can turn around the security answers by Thursday if Dana sends the questionnaire today.", timestamp: "1:08" },
      { guess: false, label: "Alex Rivera", personKey: null, speaker: "Me", speakerId: 0, text: "Great. I will send the revised quote by Friday and Sam will confirm the seat count with your IT lead.", timestamp: "1:30" },
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
