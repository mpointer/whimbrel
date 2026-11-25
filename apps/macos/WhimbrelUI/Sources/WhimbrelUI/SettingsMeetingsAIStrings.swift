import Foundation

/// The words of the Meetings section. English defaults, which the gallery shows; the app fills every field from its
/// string catalog. Text that depends on a number or a name is a closure, so a translation can put the words in its own order.
public struct WMeetingsLabels {
    // Recording notice and consent
    public var noticeHeading = "Recording notice"
    /// The label of the line that shows a model file's path under the row that names the file.
    public var pathLocation = "Location"
    public var consentNote = "Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are."
    public var noticeToggle = "Remind me to tell people I'm recording"
    public var noticeTextLabel = "Your own wording"
    public var noticeTextHelp: @Sendable (String) -> String = { "Leave this blank to use the standard reminder: \($0)" }

    // Meetings
    public var meetingsHeading = "Meetings"
    public var autoStart = "Auto-start recording for calendar meetings"
    public var qualifyingNote = "A qualifying calendar meeting has a conferencing link (Zoom/Meet/Teams/Webex) or at least one other attendee. Upcoming meetings and the recording prompt itself are shown in the main window, not here."
    public var leadTime = "Prompt lead time"
    public var minutes: @Sendable (Int) -> String = { "\($0) min" }
    public var heuristic = "Detect meetings with no calendar entry"
    public var heuristicNote = "Prompts when a known meeting app (Zoom, Teams, Slack, Webex, FaceTime) is running with the mic and system audio both active, for ad-hoc calls with nothing on your calendar. Uses the Auto-start setting above too. Doesn't cover a meeting joined from a browser tab (such as Meet) yet."
    public var checkIn = "Ask if I'm still recording after"
    public var checkInNote = "Shows a \"Still recording?\" prompt. With no answer in 5 minutes, the recording stops and is saved. Recordings always stop at 6 hours or when less than 2 GB is free. Applies from the next recording."
    public var awayMode = "Away Mode"
    public var awayNote = "While this is on, Whimbrel joins your Zoom meetings for you, muted with the camera off, records them, and posts a short note in the meeting chat. Keep this Mac unlocked and signed in to Zoom."
    public var awayZoomSetup = "In Zoom > Settings > Audio, turn on \"Automatically join audio by computer\" and \"Always mute my microphone when joining\". In Zoom > Settings > Video, turn on \"Turn off my video when joining\" and turn off \"Always show video preview dialog\"."
    public var awayConsent = "Some places require everyone's consent before a meeting is recorded. The chat note tells people, but it isn't consent. Check the rules where you and the people you meet are."
    public var awayComingSoonNote = "Whimbrel will join the Zoom meetings you can't make, record them and post a short note in the chat."
    public var betaComingSoon = "Beta coming soon"
    public var liveTranscript = "Live transcript during meetings"
    public var liveTranscriptNote = "Transcribes while you record so you can read along and ask questions during the call. Uses extra CPU while recording and pauses in Low Power Mode. The full transcript is still made after the meeting and replaces the live one. Question alerts also use it."
    public var snapshots = "Save slides and shared screens"
    public var snapshotsNote = "Saves one image per slide shown in recorded meetings to the media folder (set in Retention). Needs Screen Recording permission."
    public var screenRecordingOff = "Screen Recording permission is off."
    public var readingSize = "Transcript and summary reading size"
    public var points: @Sendable (Int) -> String = { "\($0) pt" }

    // Question alerts
    public var questionsHeading = "Question alerts"
    public var questionsNote = "Lines that mention you, and the minutes before them, go to your Ask model. Alert emails go only to your own address. While alerts are on, the live transcript keeps running in meetings even when \"Live transcript during meetings\" is off."
    public var liveQuestion = "Alert me when someone asks me something"
    public var addYourNameFirst = "Add your name in Settings, Profile first."
    public var alsoEmail = "Also email me"
    public var otherNames = "Other names people call you"

    // Voices
    public var voicesHeading = "Voices"
    public var rememberVoices = "Remember people's voices"
    public var rememberVoicesNote = "Whimbrel learns a voice when you confirm or fix a name. Voices stay on your devices and in your encrypted sync."
    public var rememberedVoices = "Remembered voices"
    public var rememberedVoicesButton = "Remembered voices\u{2026}"
    public var noVoices = "No remembered voices yet."
    public var deleteVoice = "Delete"
    public var deleteVoiceQuestion: @Sendable (String) -> String = { "Delete the remembered voice of \($0)?" }
    public var done = "Done"

    // Always treat as meetings
    public var calendarsHeading = "Always treat as meetings"
    public var calendarsNote = "Some work Exchange/CalDAV accounts don't sync a conferencing link or attendee list at all, so those events never qualify above. Opt a calendar in here and every non-all-day event on it counts as a meeting."
    public var noCalendars = "No calendars found. Grant Calendar access first."

    // Transcription
    public var transcriptionHeading = "Transcription"
    public var quality = "Quality"
    public var downloadFailed = "The better transcription model did not download."
    public var tryAgain = "Try again"
    public var modelFileNote = "Whisper.cpp GGML model used to transcribe captured meeting audio. A file you choose here is used in place of the Quality setting."
    public var chooseModelFile = "Choose Model File\u{2026}"
    public var multilingual = "I have meetings in other languages"
    public var multilingualNote = "Whimbrel detects each meeting's language and transcribes it as spoken, using a larger multilingual model. Off, every meeting is transcribed as English."
    public var diarizationHeading = "Speaker Diarization"
    public var diarizationNote = "Real per-speaker labels for the system-audio channel. Whimbrel can fetch these two models automatically via the first-run \"Set Up Models\" prompt, or see core/diarize/README.md to fetch them manually. If either is missing, transcripts fall back to a single \"Others\" label, exactly as before this feature existed."
    public var chooseSegmentation = "Choose Segmentation Model\u{2026}"
    public var chooseEmbedding = "Choose Speaker-Embedding Model\u{2026}"
    public var fileMissing: @Sendable (String) -> String = { "\($0): the file is not there." }

    public init() {}
}

/// The words of the AI section.
public struct WAILabels {
    // Local-only first
    public var heading = "AI"
    public var intro = "Configure hosted LLM providers for Ask/Digest. Keys are stored in the macOS Keychain, never in a plaintext file."
    public var localOnly = "Local-only mode"
    public var localOnlyNote = "When on, every conversation uses Ollama only, regardless of the provider picked in Ask or Digest. Hosted providers are never called."
    public var smallLocalModel = "Small local model"
    public var removeLocalModel = "Remove the Small Local Model"

    // Where AI runs
    public var onRampHeading = "Pick where AI runs"
    public var onRampIntro = "Whimbrel works without any AI model: capture, transcripts and search still work. Chat, summaries and routines need one."
    public var installOllama = "Install Ollama"
    public var freeGeminiKey = "Get a free Gemini key"
    public var onRampNote = "Ollama is free and runs on this Mac, so nothing leaves it unless you point Whimbrel at another computer. Gemini's free tier is free, and Google may use prompts to improve its products. Or paste any Anthropic, OpenAI or Google key below."

    // Whimbrel AI
    public var whimbrelAI = "Whimbrel AI"
    public var addCredit = "Add credit"
    public var signInToUse = "Sign in to sync to use Whimbrel AI"
    public var refreshModels = "Refresh Models"

    // Providers
    public var monthlyBudget = "Monthly budget"
    public var dollar = "$"
    public var none = "None"
    public var overBudget = "At or over the monthly budget."
    public var syncToDevices = "Sync to devices"
    public var saveKeyFirst = "Save a key first."
    public var googleBilling = "My Google AI Studio project has billing on"
    public var googleBillingNote = "Gmail and Calendar text goes to Gemini only when its project has billing on, because Google may train on the free tier."
    public var openRouterModel = "OpenRouter model"
    public var openRouterPlaceholder = "vendor/model or vendor/model:free"
    public var notPicked = "Not picked"
    public var noModelsLoaded = "No models loaded. Save a key above, then choose Refresh Models."
    public var includesOtherComputers = "Includes your other computers."
    public var remove = "Remove"
    public var removeKeyTitle: @Sendable (String) -> String = { "Remove your \($0) key?" }
    public var removeKeyMessage = "Anything that uses it stops working until you save a key again."

    // Ollama
    public var ollama = "Ollama"
    public var ollamaNote = "Local server, no API key needed. It runs on this Mac. If you point Whimbrel at an Ollama server on another computer, your prompts go to that server."
    public var useLocalModel = "Use local model"
    public var remoteGoogleData = "Allow Gmail and Calendar text with a cloud or remote Ollama model"
    public var remoteGoogleDataNote = "Off keeps Gmail and Calendar text on this computer when Ollama runs elsewhere."

    // Small tasks, image search
    public var cheapFirst = "Use the cheapest model for small tasks"
    public var cheapFirstNote = "Conversation titles, topic names and profile suggestions go to the local model first, then to the cheapest model you have a key for. Chat, summaries and digests keep using your default provider. Each routine picks its own model in Routines."
    public var imageSearchNote = "Lets the skin chat in General settings find images on the web, with strict safe-search. Searches count toward the daily spending limit. The key stays on this Mac. Local-only mode blocks it."

    // TypeSafe
    public var typesafeHeading = "Reply priorities (TypeSafe)"
    public var saveTypesafeFirst = "Save a TypeSafe key first."
    public var rankEmail = "Rank email by reply priority"
    public var rankEmailNote = "Off by default. Gmail threads wait until TypeSafe confirms it doesn't train on them. Then, for each thread waiting on you, Whimbrel sends TypeSafe only the redacted subject, the sender's name and domain, and the latest message cut to 1,000 characters. The key and this setting stay on this Mac. Local-only mode blocks it."
    public var typesafeRejected = "TypeSafe rejected this key. Check it and save it again."
    public var sortTopics = "Sort items into topics with TypeSafe"
    public var sortTopicsNote = "Sends short excerpts of emails, meetings, calendar events and screen activity to TypeSafe to choose a topic. Gmail and Google Calendar items wait until TypeSafe confirms it doesn't train on them."
    public var titleCheck = "Check calendar meeting titles"
    public var titleCheckNote = "Off by default. When on, after a calendar meeting is summarized Whimbrel sends the event title, attendee names and the summary text to TypeSafe to check that the title matches what was discussed. Uses the key above. Never runs in Local-only mode."

    // Defaults and limits
    public var defaultsHeading = "Defaults and limits"
    public var defaultProvider = "Default provider"
    public var defaultModel = "Default model"
    public var ollamaModelPlaceholder = "e.g. gemma4:12b"
    public var userCap = "Daily cap per identity (cents)"
    public var globalCap = "Daily cap, global (cents)"
    public var defaultWord = "default"
    public var capNote = "Leave blank to use the conservative built-in default; set to 0 to explicitly disable that cap."
    public var spendToday = "Spend today"
    public var refresh = "Refresh"

    // Model usage
    public var usageHeading = "Model usage"
    public var noCalls = "No model calls yet."
    public var usageRange = "Time range"
    public var showLast7 = "Show last 7 days"
    public var showLast30 = "Show last 30 days"

    // Prompts
    public var promptsHeading = "Prompts"
    public var promptsNote = "Tune the instructions folded into each surface's prompt. Each surface is independent and syncs across your devices."
    public var promptSurface = "Surface"
    public var resetPromptLabel = "Standard instructions"
    public var resetPrompt = "Reset to Default"
    public var promptIsDefault = "This surface already uses the standard instructions."

    public init() {}
}

/// The words of the Topics section.
public struct WTopicsLabels {
    public var heading = "Topics"
    public var intro = "Name the customers, products and projects you care about. Whimbrel groups your meetings, emails and notes under them."
    public var yourTopics = "Your topics"
    public var noTopics = "No topics yet. Add one, or accept a suggestion below."
    /// The line for an empty list when there is nothing in Suggested either, so it does not point at a list with nothing in it.
    public var noTopicsAndNoSuggestions = "No topics yet. Add one."
    public func noTopicsText(hasSuggestions: Bool) -> String { hasSuggestions ? noTopics : noTopicsAndNoSuggestions }
    public var addTopicLabel = "Your own topic"
    public var addTopic = "Add topic"
    public var editTopic = "Edit topic"
    public var edit = "Edit"
    public var editNamed: @Sendable (String) -> String = { "Edit \($0)" }
    public var mergeInto = "Merge into\u{2026}"
    public var mergeNamed: @Sendable (String) -> String = { "Merge \($0) into another topic" }
    public var mergeSentence: @Sendable (String) -> String = { "Merge \($0) into" }
    public var merge = "Merge"
    public var delete = "Delete"
    public var deleteNamed: @Sendable (String) -> String = { "Delete \($0)" }
    public var deletePointer = "To delete a client and what Whimbrel holds on them, open the client's page and choose Delete this client."
    public var close = "Close"
    public var cancel = "Cancel"
    public var save = "Save"
    public var name = "Name"
    public var aliases = "Aliases, separated by commas"
    public var kind = "Kind"
    public var nameRequired = "Enter a name to save."
    public var suggested = "Suggested"
    public var suggestedEmpty = "When a theme shows up across several of your meetings, emails and notes, Whimbrel suggests it here. Nothing is added until you press Add."
    public var placeholderNote = "Some suggested topics have placeholder names until an AI model names them."
    public var add = "Add"
    public var dismiss = "Dismiss"
    public var namesHeading = "Names I go by"
    public var namesNote = "Whimbrel uses these to tell steps you owe from steps owed to you."
    public var namesList = "Your names"
    public var noNames = "No names yet."
    public var addNameLabel = "Add a name"
    public var addName = "Add a name"
    public var suggestedNames = "Suggested names"
    public var addChip: @Sendable (String) -> String = { "+ \($0)" }
    public var remove = "Remove"
    public var removeNamed: @Sendable (String) -> String = { "Remove \($0)" }
    public var findSteps = "Find steps with TypeSafe"
    public var findStepsNote = "Sends short excerpts of topic emails, meeting action items and Teams screens to TypeSafe to find steps you owe and are owed. Off: your local model does this."

    public init() {}
}
