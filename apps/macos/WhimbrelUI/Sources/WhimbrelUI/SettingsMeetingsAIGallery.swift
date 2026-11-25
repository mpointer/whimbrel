import SwiftUI

// Meetings, AI and Topics from sample models, for `WhimbrelUIGallery --settings`. Made-up content only: no real key, no
// real name. A saved key is drawn as the part draws it, a fixed mask. The sheets and confirms are frames the app presents
// in its own sheet, so they are drawn here as their frames, on a page-coloured panel.

@MainActor
extension WSettingsWindowGallery {
    static let anchorsMeetings = WMeetingsAnchors(
        recordingNotice: "meetings.recordingNotice", autoStart: "meetings.autoStart", liveTranscript: "meetings.liveTranscript",
        heuristic: "meetings.heuristic", snapshots: "meetings.snapshots", voices: "meetings.voices", transcription: "meetings.transcription",
        multilingual: "meetings.multilingual")
    static let anchorsAI = WAIAnchors(
        localOnly: "ai.localOnly", whimbrelAI: "ai.whimbrelAI", anthropicKey: "ai.anthropicKey", openaiKey: "ai.openaiKey", googleKey: "ai.googleKey",
        openrouterKey: "ai.openrouterKey", ollama: "ai.ollama", provider: "ai.provider", model: "ai.model", dailyCap: "ai.dailyCap", spend: "ai.spend",
        usage: "ai.usage", typesafe: "ai.typesafe", prompts: "ai.prompts")
    static let anchorsTopics = WTopicsAnchors(list: "topics.list", add: "topics.add", names: "topics.names")

    // MARK: Meetings

    static func meetings(
        noticeOn: Bool = true, hasNames: Bool = true, away: WAwayModel = .available(enabled: false), calendars: Bool = true,
        download: WModelDownloadState = .none, voices: [WVoicePerson] = [], modelMissing: Bool = false, screenRecording: Bool = true
    ) -> WMeetingsModel {
        WMeetingsModel(
            noticeEnabled: noticeOn, noticeCustomText: "", noticeDefaultText: "I'm recording this meeting with Whimbrel, on my own computer, for my notes.",
            autoStart: true, leadMinutes: 5, heuristic: false, checkInHours: 2,
            checkInOptions: [WChoice(id: "0", title: "Off"), WChoice(id: "1", title: "1 hour"), WChoice(id: "2", title: "2 hours"), WChoice(id: "3", title: "3 hours"), WChoice(id: "4", title: "4 hours")],
            away: away, liveTranscript: true, snapshots: true, screenRecordingGranted: screenRecording, readingSize: 13,
            hasNames: hasNames, liveQuestion: hasNames, liveQuestionEmail: false, otherNames: "Sam, Sammy", rememberVoices: true, voices: voices,
            calendars: calendars ? [
                WCalendarChoice(id: "work", title: "Work", isOn: true), WCalendarChoice(id: "home", title: "Home", isOn: false),
                WCalendarChoice(id: "club", title: "Rowing club", isOn: false),
            ] : [],
            transcription: WTranscriptionModel(
                qualityOptions: [WChoice(id: "automatic", title: "Automatic"), WChoice(id: "better", title: "Better"), WChoice(id: "fast", title: "Fast")],
                quality: download == .none ? "automatic" : "better",
                qualityDetail: download == .none ? "The best engine this computer runs well." : "The most accurate model, a download of about 574 MB. Slower on older computers.",
                download: download, downloadText: "Downloading the better transcription model\u{2026}",
                modelFile: WPathModel(label: "Model file", path: "/Users/sam/Library/Application Support/Example/models/ggml-base.en.bin", exists: !modelMissing),
                multilingual: false, statusMessage: nil,
                segmentation: WPathModel(label: "Segmentation model", path: "/Users/sam/Library/Application Support/Example/models/segmentation-3.0.onnx", exists: true),
                embedding: WPathModel(label: "Speaker-embedding model", path: "/Users/sam/Library/Application Support/Example/models/embedding.onnx", exists: true),
                diarizationStatus: nil),
            anchors: anchorsMeetings)
    }

    private static func meetingsPane(_ m: WMeetingsModel, highlighted: String? = nil) -> AnyView {
        AnyView(WMeetingsSettingsView(model: m, actions: WMeetingsActions(), highlighted: highlighted))
    }

    // MARK: AI

    private static func provider(
        _ id: String, _ name: String, _ keyTitle: String, anchor: String, saved: Bool = false, status: WSettingsStatus? = nil, month: Bool = false,
        budget: Double? = nil, models: WModelListStatus? = nil, billing: Bool? = nil
    ) -> WProviderModel {
        // A saved key whose last check was a problem is a refused key: the chip says so and no model count is shown.
        var refused = false
        if saved, case .problem? = status { refused = true }
        return WProviderModel(
            id: id, name: name,
            links: [WProviderLink(title: "Get an API key from \(name)", url: "https://example.com/\(id)/keys"), WProviderLink(title: "Data policy", url: "https://example.com/\(id)/policy")],
            key: WKeyRowModel(title: keyTitle, isSaved: saved, placeholder: saved ? "sk-\u{2026}" : "", status: status, anchor: anchor, refused: refused),
            keyNote: id == "openrouter" ? "One key for any model on OpenRouter's catalogue, free ones included. Pick the model below; until you do, OpenRouter is left out of every chain, because their catalogue changes weekly and there is no fallback model to substitute." : nil,
            costLine: "Small tasks use example-small. Most people spend under a dollar a month at this setting.",
            termsLine: id == "google" ? "The free tier may use prompts to improve Google products. The paid tier does not. (Checked 2026-11-01.)" : nil,
            monthText: month ? "This month: $12.40 of $25.00 (50%)" : nil, monthFraction: month ? 0.5 : nil, budgetDollars: budget,
            syncOn: false, models: refused ? nil : models, billingConfirmed: billing)
    }

    static func ai(
        keys: Set<String> = [], localOnly: Bool = false, keyStatus: [String: WSettingsStatus] = [:], onRamp: Bool = false,
        localModel: WLocalModelCard = .notInstalled, signedIn: Bool = true, usageEmpty: Bool = false
    ) -> WAIModel {
        let listed = WModelListStatus(text: "12 models available")
        func has(_ id: String) -> Bool { keys.contains(id) }
        return WAIModel(
            localOnly: localOnly,
            onRamp: onRamp ? WOnRampModel(installOllamaURL: "https://example.com/ollama", geminiKeyURL: "https://example.com/gemini") : nil,
            localModel: localModel,
            whimbrel: signedIn
                ? WWhimbrelAIModel(
                    signedIn: true, allowanceLine: "Plus: $6.40 of this month's AI allowance left, resets December 1, 2026", balanceLine: "Prepaid credit: $4.20 left",
                    lowWarning: nil, packs: [WCreditPack(id: "10", title: "Add $10"), WCreditPack(id: "25", title: "Add $25")], models: listed,
                    terms: "Whimbrel AI sends the text of your request to the provider that answers it.")
                : WWhimbrelAIModel(signedIn: false),
            providers: [
                provider("anthropic", "Anthropic", "Anthropic API Key", anchor: "ai.anthropicKey", saved: has("anthropic"), status: keyStatus["anthropic"], month: has("anthropic"), budget: has("anthropic") ? 25 : nil, models: has("anthropic") ? listed : nil),
                provider("openai", "OpenAI", "OpenAI API Key", anchor: "ai.openaiKey", saved: has("openai"), status: keyStatus["openai"], models: has("openai") ? listed : nil),
                provider("google", "Google", "Google API Key", anchor: "ai.googleKey", saved: has("google"), status: keyStatus["google"], models: has("google") ? listed : nil, billing: false),
                provider("openrouter", "OpenRouter", "OpenRouter API Key", anchor: "ai.openrouterKey", saved: has("openrouter"), status: keyStatus["openrouter"], models: has("openrouter") ? listed : nil),
            ],
            includesOtherComputers: !usageEmpty,
            openRouterModel: .picker(options: ["example/model-a", "example/model-b:free"], selection: ""),
            ollama: WOllamaModel(
                useLocalModel: true,
                useLocalCaption: localOnly ? "Local-only mode is on, so the local model is always used." : "Off skips Ollama entirely. Turn it off on a Mac that does not run Ollama, so calls stop trying to reach it first.",
                models: WModelListStatus(text: "3 models available"), missingChoice: nil, remoteGoogleData: false),
            cheapFirst: true,
            imageSearchKey: WKeyRowModel(title: "Image Search Key (Brave Search)", isSaved: false),
            typeSafe: WTypeSafeModel(
                key: WKeyRowModel(title: "TypeSafe API Key", isSaved: has("typesafe"), placeholder: has("typesafe") ? "sk-\u{2026}" : ""),
                hasKey: has("typesafe"), rankEmail: false, rankRejected: false, sortTopics: has("typesafe"),
                sortNotes: has("typesafe") ? [WSettingNote(id: "start", text: "About 420 items, about $0.02. Start?", buttonTitle: "Start")] : [], titleCheck: false),
            routing: WRoutingModel(
                providers: [WChoice(id: "ollama", title: "Ollama"), WChoice(id: "anthropic", title: "Anthropic"), WChoice(id: "openai", title: "OpenAI"), WChoice(id: "google", title: "Google"), WChoice(id: "openrouter", title: "OpenRouter"), WChoice(id: "whimbrel", title: "Whimbrel AI")],
                provider: "anthropic", model: .picker(options: ["example-large", "example-small"], selection: "example-large"),
                unavailableNotes: [], userCapCents: nil, globalCapCents: 0, spendText: "12\u{00A2} (this identity) / 40\u{00A2} (global)"),
            usage: WUsageModel(
                totalsLine: usageEmpty ? nil : "Today $0.12, last 30 days $9.80, this month $5.40. Local calls are not metered.",
                days: usageEmpty ? [] : [
                    WUsageDayModel(day: "Nov 23, 2026", rows: [
                        WUsageRowModel(id: "1", time: "9:41 AM", task: "ask", model: "anthropic example-large", tokens: "1200 in, 340 out", cost: "$0.04"),
                        WUsageRowModel(id: "2", time: "9:12 AM", task: "title", model: "ollama example-small", tokens: "", cost: "local"),
                    ]),
                    WUsageDayModel(day: "Nov 22, 2026", rows: [
                        WUsageRowModel(id: "3", time: "4:03 PM", task: "digest", model: "anthropic example-large", tokens: "5200 in, 800 out", cost: "$0.12"),
                    ]),
                ], showsAll: false),
            prompts: WPromptsModel(
                surfaces: [WChoice(id: "ask", title: "Ask"), WChoice(id: "meetingSummary", title: "Meeting summary"), WChoice(id: "dailyDigest", title: "Daily digest")],
                selected: "ask", text: "Answer in short paragraphs. Put the decision first.", help: "Added to every answer in Ask."),
            anchors: anchorsAI)
    }

    private static func aiPane(_ m: WAIModel, highlighted: String? = nil, removing: String? = nil) -> AnyView {
        AnyView(WAISettingsView(model: m, actions: WAIActions(), highlighted: highlighted, initiallyRemoving: removing))
    }

    // MARK: Topics

    static func topics(empty: Bool = false, steps: Bool = true) -> WTopicsModel {
        WTopicsModel(
            seeds: empty ? [] : [
                WTopicSeedModel(id: "harbor", name: "Harbor redesign", aliases: ["Harbor", "HR"], kind: "project", kindLabel: "Project", itemsText: "42 items"),
                WTopicSeedModel(id: "northwind", name: "Northwind Studio", aliases: [], kind: "customer", kindLabel: "Customer", itemsText: "17 items"),
                WTopicSeedModel(id: "notes", name: "Weekly planning", aliases: ["Planning"], kind: nil, kindLabel: nil, itemsText: "9 items"),
            ],
            kindChoices: [WChoice(id: "", title: "None"), WChoice(id: "customer", title: "Customer"), WChoice(id: "product", title: "Product"), WChoice(id: "project", title: "Project"), WChoice(id: "subject", title: "Subject")],
            suggestions: empty ? [] : [WTopicSuggestionModel(id: "101", name: "Rowing club", itemsText: "6 items"), WTopicSuggestionModel(id: "102", name: "Tax paperwork", itemsText: "4 items")],
            showsPlaceholderNote: !empty, names: ["Sam Rivera", "Sam"], nameChips: ["Sammy"],
            stepsOn: steps, stepsDisabledReason: steps ? nil : "Save a TypeSafe key first.",
            stepsNotes: steps ? [WSettingNote(id: "start", text: "About 120 items, about $0.01. Start?", buttonTitle: "Start")] : [],
            anchors: anchorsTopics)
    }

    private static func topicsPane(
        _ m: WTopicsModel, highlighted: String? = nil, editor: WTopicSeedModel?? = nil, merging: String? = nil, deleting: String? = nil
    ) -> AnyView {
        AnyView(WTopicsSettingsView(model: m, actions: WTopicsActions(), highlighted: highlighted, initialEditor: editor, initialMerging: merging, initialDeleting: deleting))
    }

    private struct Panel<Content: View>: View {
        @Environment(\.wTheme) private var theme
        let content: Content
        init(@ViewBuilder content: () -> Content) { self.content = content() }
        var body: some View {
            VStack(alignment: .leading, spacing: 24) { content }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(theme.backgroundPage)
        }
    }

    /// Each picture's height, fitted to its content when the pictures were last made.
    static let fittedHeights: [String: CGFloat] = [
        "settings-ai-budget-invalid-dark": 162,
        "settings-ai-budget-invalid-light": 217,
        "settings-ai-dark": 4043,
        "settings-ai-high-contrast": 4059,
        "settings-ai-highlight-cap-light": 4004,
        "settings-ai-key-error-light": 3969,
        "settings-ai-key-saved-dark": 4027,
        "settings-ai-key-saved-light": 4027,
        "settings-ai-light-2.0": 6757,
        "settings-ai-light": 4043,
        "settings-ai-local-only-dark": 3889,
        "settings-ai-local-only-light": 3937,
        "settings-ai-narrow-light": 4388,
        "settings-ai-no-keys-light": 3819,
        "settings-ai-remove-confirm-dark": 203,
        "settings-ai-remove-confirm-light": 203,
        "settings-ai-usage-light": 248,
        "settings-ai-usage-narrow-dark": 312,
        "settings-meetings-dark": 2340,
        "settings-meetings-download-failed-dark": 2457,
        "settings-meetings-download-light": 2414,
        "settings-meetings-high-contrast": 2343,
        "settings-meetings-highlight-voices-light": 2340,
        "settings-meetings-light-2.0": 3831,
        "settings-meetings-light": 2340,
        "settings-meetings-narrow-light": 2761,
        "settings-meetings-no-names-light": 2073,
        "settings-meetings-notice-off-light": 576,
        "settings-meetings-voices-sheet-empty-dark": 204,
        "settings-meetings-voices-sheet-light": 321,
        "settings-topics-dark": 1069,
        "settings-topics-delete-pointer-light": 1128,
        "settings-topics-editor-light": 372,
        "settings-topics-editor-new-dark": 399,
        "settings-topics-empty-light": 736,
        "settings-topics-high-contrast": 1072,
        "settings-topics-highlight-names-light": 1069,
        "settings-topics-light-2.0": 1798,
        "settings-topics-light": 1069,
        "settings-topics-merge-light": 1105,
        "settings-topics-narrow-light": 1282,
    ]

    // MARK: Cases

    static func meetingsAITopicsCases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ mode: WGalleryMode, _ scale: CGFloat = 1, width: CGFloat = 900, height: CGFloat, _ view: AnyView) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: fittedHeights[id] ?? height, view: view))
        }
        let variants: [(String, WGalleryMode, CGFloat)] = [("light", .light, 1), ("dark", .dark, 1), ("high-contrast", .highContrastLight, 1), ("light-2.0", .light, 2)]
        for (suffix, mode, scale) in variants {
            let big: CGFloat = scale > 1 ? 1.9 : 1
            let wide: CGFloat = scale > 1 ? 1400 : 900
            add("settings-meetings-\(suffix)", mode, scale, width: wide, height: 2500 * big, window("meetings", pane: meetingsPane(meetings(voices: []))))
            add("settings-ai-\(suffix)", mode, scale, width: wide, height: 4300 * big, window("ai", pane: aiPane(ai(keys: ["anthropic", "typesafe"], onRamp: false, localModel: .installed))))
            add("settings-topics-\(suffix)", mode, scale, width: wide, height: 1900 * big, window("topics", pane: topicsPane(topics())))
        }
        // Meetings states.
        add("settings-meetings-notice-off-light", .light, height: 560, window("meetings", pane: meetingsPane(meetings(noticeOn: false))))
        add("settings-meetings-no-names-light", .light, height: 2200, window("meetings", pane: meetingsPane(meetings(hasNames: false, away: .comingSoon, calendars: false, screenRecording: false))))
        add("settings-meetings-download-light", .light, height: 2500, window("meetings", pane: meetingsPane(meetings(download: .downloading(fraction: 0.42)))))
        add("settings-meetings-download-failed-dark", .dark, height: 2500, window("meetings", pane: meetingsPane(meetings(download: .failed, modelMissing: true))))
        add("settings-meetings-highlight-voices-light", .light, height: 2500, window("meetings", pane: meetingsPane(meetings(), highlighted: "meetings.voices")))
        add("settings-meetings-voices-sheet-light", .light, height: 460, AnyView(Panel {
            WRememberedVoicesSheet(
                model: meetings(voices: [WVoicePerson(id: "a", name: "Alex Chen"), WVoicePerson(id: "b", name: "Priya Nair"), WVoicePerson(id: "c", name: "Jordan Lee")]),
                actions: WMeetingsActions(), onDone: {})
        }))
        add("settings-meetings-voices-sheet-empty-dark", .dark, height: 320, AnyView(Panel {
            WRememberedVoicesSheet(model: meetings(), actions: WMeetingsActions(), onDone: {})
        }))
        add("settings-meetings-narrow-light", .light, width: 520, height: 2900, window("meetings", pane: meetingsPane(meetings())))

        // AI states.
        add("settings-ai-no-keys-light", .light, height: 4200, window("ai", pane: aiPane(ai(onRamp: true, signedIn: false, usageEmpty: true))))
        add("settings-ai-key-saved-light", .light, height: 4200, window("ai", pane: aiPane(ai(keys: ["anthropic"], keyStatus: ["anthropic": .info("Saved. Checking the key\u{2026}")]))))
        add("settings-ai-key-saved-dark", .dark, height: 4200, window("ai", pane: aiPane(ai(keys: ["anthropic"], keyStatus: ["anthropic": .info("12 models available. The key works.")]))))
        add("settings-ai-key-error-light", .light, height: 4200, window("ai", pane: aiPane(ai(keys: ["openai"], keyStatus: ["openai": .problem("That key was refused by the provider. Check it and try again.")]))))
        add("settings-ai-local-only-light", .light, height: 4200, window("ai", pane: aiPane(ai(localOnly: true, localModel: .installed))))
        add("settings-ai-local-only-dark", .dark, height: 4200, window("ai", pane: aiPane(ai(localOnly: true))))
        add("settings-ai-highlight-cap-light", .light, height: 4300, window("ai", pane: aiPane(ai(keys: ["anthropic"]), highlighted: "ai.dailyCap")))
        add("settings-ai-remove-confirm-light", .light, height: 360, AnyView(Panel {
            WConfirm(
                title: "Remove your Anthropic key?", message: "Anything that uses it stops working until you save a key again.", confirmTitle: "Remove",
                onConfirm: {}, onCancel: {})
        }))
        add("settings-ai-remove-confirm-dark", .dark, height: 360, AnyView(Panel {
            WConfirm(
                title: "Remove your Anthropic key?", message: "Anything that uses it stops working until you save a key again.", confirmTitle: "Remove",
                onConfirm: {}, onCancel: {})
        }))
        add("settings-ai-budget-invalid-light", .light, height: 420, AnyView(Panel {
            WSettingsCard(heading: "Anthropic") {
                WSettingsNumberFieldRow(
                    label: "Monthly budget", value: 25, rule: WBudgetRule.rule(maxCents: 100_000_000), prefix: "$", placeholder: "None", fieldWidth: 110,
                    initialText: "12.345", onCommit: { _ in })
                WSettingsNumberFieldRow(
                    label: "Daily cap per identity (cents)", value: nil, rule: WCapRule.rule, placeholder: "default", initialText: "-4", onCommit: { _ in })
            }
        }))
        add("settings-ai-budget-invalid-dark", .dark, height: 420, AnyView(Panel {
            WSettingsCard(heading: "Anthropic") {
                WSettingsNumberFieldRow(
                    label: "Monthly budget", value: 25, rule: WBudgetRule.rule(maxCents: 100_000_000), prefix: "$", placeholder: "None", fieldWidth: 110,
                    initialText: "abc", onCommit: { _ in })
            }
        }))
        add("settings-ai-usage-light", .light, height: 560, AnyView(Panel {
            WSettingsCard(heading: "Model usage", description: ai().usage.totalsLine) {
                WUsageTable(days: ai().usage.days, emptyText: "No model calls yet.", label: "Model usage")
            }
        }))
        add("settings-ai-usage-narrow-dark", .dark, width: 520, height: 600, AnyView(Panel {
            WSettingsCard(heading: "Model usage", description: ai().usage.totalsLine) {
                WUsageTable(days: ai().usage.days, emptyText: "No model calls yet.", label: "Model usage")
            }
        }))
        add("settings-ai-narrow-light", .light, width: 520, height: 4600, window("ai", pane: aiPane(ai(keys: ["anthropic"]))))

        // Topics states.
        add("settings-topics-empty-light", .light, height: 1000, window("topics", pane: topicsPane(topics(empty: true, steps: false))))
        add("settings-topics-highlight-names-light", .light, height: 1900, window("topics", pane: topicsPane(topics(), highlighted: "topics.names")))
        add("settings-topics-editor-light", .light, height: 520, AnyView(Panel {
            WTopicEditorSheet(
                target: topics().seeds[0], labels: WTopicsLabels(), kinds: topics().kindChoices, onSave: { _, _, _ in }, onCancel: {})
        }))
        add("settings-topics-editor-new-dark", .dark, height: 520, AnyView(Panel {
            WTopicEditorSheet(target: nil, labels: WTopicsLabels(), kinds: topics().kindChoices, onSave: { _, _, _ in }, onCancel: {})
        }))
        add("settings-topics-merge-light", .light, height: 1900, window("topics", pane: topicsPane(topics(), merging: "harbor")))
        add("settings-topics-delete-pointer-light", .light, height: 1900, window("topics", pane: topicsPane(topics(), deleting: "northwind")))
        add("settings-topics-narrow-light", .light, width: 520, height: 3000, window("topics", pane: topicsPane(topics(), merging: "notes")))
        return out
    }
}
