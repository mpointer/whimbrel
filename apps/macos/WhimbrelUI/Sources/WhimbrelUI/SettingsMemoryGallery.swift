import SwiftUI

// Memory & Privacy from sample models, for `WhimbrelUIGallery --settings`. Made-up content only. The sheets and confirms are
// frames the app presents in its own sheet, so they are drawn here as their frames, on a page-coloured panel.

@MainActor
extension WSettingsWindowGallery {
    static let anchorsMemory = WMemoryPrivacyAnchors(
        yourData: "capture.yourData", export: "capture.export", delete: "capture.delete", obsidian: "capture.obsidian",
        encryption: "capture.encryption", screenCaptures: "capture.screenCaptures", excludeApps: "capture.excludeApps",
        excludeSites: "capture.excludeSites", hotkey: "capture.hotkey", retention: "capture.retention", media: "capture.media",
        storage: "capture.storage")

    private static func retentionRows(screenForever: Bool = true) -> [WRetentionRowModel] {
        [
            WRetentionRowModel(id: "screenText", kind: .screenText, label: "Screen text", days: 30, keepForever: screenForever, foreverTitle: "Keep forever", asksBeforeShortening: true),
            WRetentionRowModel(id: "meetings", kind: .meetings, label: "Meetings and recordings", days: 90, keepForever: false, foreverTitle: "Keep forever", asksBeforeShortening: true),
            WRetentionRowModel(id: "conversations", kind: .conversations, label: "Conversations", days: 30, keepForever: true, foreverTitle: "Keep forever", asksBeforeShortening: true),
        ]
    }

    static func memoryPrivacy(
        memory: WMemoryState = .on, reason: String? = nil, screenForever: Bool = true, encryption: WLibraryEncryptionModel? = nil
    ) -> WMemoryPrivacyModel {
        WMemoryPrivacyModel(
            memory: memory, memoryReason: reason, showScreenCaptures: false,
            apps: [
                WExcludedApp(id: "com.example.notes", name: "Notes", excluded: false),
                WExcludedApp(id: "com.example.vault", name: "Password Vault", excluded: true),
                WExcludedApp(id: "com.example.mail", name: "Mail", excluded: false),
            ],
            sites: ["bank.example", "health.example"],
            localOnlyLine: "Local-only mode: Off",
            counts: [
                WSettingsValueItem(label: "Meetings", value: "128"), WSettingsValueItem(label: "Transcript lines", value: "20,418"),
                WSettingsValueItem(label: "Screen captures", value: "3,904"), WSettingsValueItem(label: "Slides", value: "212"),
                WSettingsValueItem(label: "Emails", value: "1,577"), WSettingsValueItem(label: "Conversations", value: "46"),
            ],
            dataFolderValue: "1.8 GB",
            mediaFolderValue: "/Users/sam/Pictures/Whimbrel, 3.4 GB",
            outbound: [
                WOutboundRow(id: "providers", title: "AI providers", state: "Keys for Anthropic.", when: "When you ask a question, summarize a meeting, or run a routine with a hosted model.", sees: "The provider whose key you added receives the text needed for that request, under its own terms.", active: true),
                WOutboundRow(id: "sync", title: "Sync relay", state: "Off.", when: "Continuously, while sync is on.", sees: "The relay stores encrypted data it cannot read.", active: false),
            ],
            vault: WVaultModel(enabled: true, path: "/Users/sam/Documents/Vault", lastUpdated: "Last updated 3 minutes ago"),
            delete: WDeleteEverythingModel(
                body: "This removes the database and the data folder (1.8 GB), and this Mac's recordings and slide images in /Users/sam/Pictures/Whimbrel (3.4 GB), then restarts Whimbrel as new. Settings, keys, connected Google accounts and downloaded models stay.",
                offersSyncAccount: true,
                syncNoteIfDeleted: "The encrypted copy on the sync server is deleted and every device is signed out of sync. Your other devices keep their own data.",
                syncNoteIfKept: "Sync is on. This Mac will download everything again from your other devices unless you also delete your sync account."),
            storageTotal: "Whimbrel uses 5.2 GB on this Mac.",
            storageParts: "Database 1.8 GB · Recordings and images 3.4 GB · Speech models 0.9 GB · Capture queue 0 KB · Other 0.1 GB",
            lowDisk: [],
            retention: retentionRows(screenForever: screenForever),
            media: WMediaModel(
                folderPath: "/Users/sam/Pictures/Whimbrel", canReset: false,
                compress: WRetentionRowModel(id: "compress", kind: nil, label: "Compress audio after", days: 30, keepForever: false, foreverTitle: "Keep full quality", asksBeforeShortening: true, effect: .recompressesAudio),
                deleteAudio: WRetentionRowModel(id: "deleteAudio", kind: .deleteAudio, label: "Delete audio after", days: 365, keepForever: true, foreverTitle: "Keep forever", asksBeforeShortening: true),
                shrink: WRetentionRowModel(id: "shrink", kind: nil, label: "Shrink images after", days: 90, keepForever: true, foreverTitle: "Keep full quality", asksBeforeShortening: true, effect: .shrinksImages),
                usage: "Audio: 2.9 GB full quality, 0.3 GB compressed. Images: 0.2 GB.",
                preview: "Nothing due now. Up to 25 meetings per run; runs at launch, daily and after each meeting. Includes existing recordings."),
            cleanups: [
                WCleanupModel(id: "orphans", title: "Orphaned recording files", notice: "Recording files with no matching meeting record at all, for example from a crashed or interrupted session.", scanTitle: "Scan for Orphaned Files", status: "No orphaned files found."),
                WCleanupModel(id: "duplicates", title: "Duplicate meetings", notice: "Finds and removes leftover duplicates of a peer's meeting. Never affects this device's own meetings.", scanTitle: "Scan for Duplicate Meetings", status: "2 duplicate sets found, 2 extra copies.", confirm: .init(title: "Delete Duplicate Meetings?", message: "This will permanently delete 2 duplicate meetings. The most recently applied copy in each group is kept. This cannot be undone.", button: "Delete 2 Duplicates")),
                WCleanupModel(id: "noAudio", title: "Meetings without audio", notice: "Meeting entries that never captured any audio. Still-recording meetings are never included.", scanTitle: "Scan for Meetings Without Audio"),
            ],
            encryption: encryption,
            hotkey: WMemoryHotkeyModel(keys: ["\u{2303}", "\u{2325}", "Space"]),
            anchors: anchorsMemory)
    }

    static func encryptionModel(_ state: WLibraryEncryptionModel.State) -> WLibraryEncryptionModel {
        switch state {
        case .encrypted:
            return WLibraryEncryptionModel(
                state: .encrypted, stateText: "Encrypted",
                body: [
                    "Your library on this Mac is encrypted with a key kept in your Mac Keychain. A copy of the library file is unreadable without it. Backups made before you turned this on, such as Time Machine, still hold a readable copy.",
                    "Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files.",
                    "Encryption does not protect against malware running as you, or someone using your Mac while it is unlocked.",
                ],
                phase: .idle(notice: nil, canStart: false), fastSearch: false, offersSpeedCheck: true)
        case .notEncrypted:
            return WLibraryEncryptionModel(
                state: .notEncrypted, stateText: "Not encrypted",
                body: [
                    "Your library is an ordinary file on this Mac. Encrypting it makes a copy of the file unreadable without a key kept in your Mac Keychain.",
                    "Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files.",
                    "Encryption does not protect against malware running as you, or someone using your Mac while it is unlocked.",
                ],
                phase: .idle(notice: nil, canStart: true), fastSearch: nil, offersSpeedCheck: true)
        case .needsAttention:
            return WLibraryEncryptionModel(
                state: .needsAttention, stateText: "Needs attention",
                body: ["The key in your Keychain does not open your library. Use your recovery key to open it."],
                phase: .idle(notice: nil, canStart: false),
                notes: [WLibraryEncryptionModel.Note(id: 0, lines: ["Whimbrel could not read your Keychain when it started, so encryption cannot start yet. Your library opened as usual."])])
        }
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

    private static func memoryPane(_ m: WMemoryPrivacyModel, highlighted: String? = nil) -> AnyView {
        AnyView(WMemoryPrivacySettingsView(model: m, actions: WMemoryPrivacyActions(), highlighted: highlighted))
    }

    static func memoryPrivacyCases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ mode: WGalleryMode, _ scale: CGFloat = 1, width: CGFloat = 900, height: CGFloat, _ view: AnyView) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: height, view: view))
        }
        let labels = WMemoryPrivacyLabels()
        let variants: [(String, WGalleryMode, CGFloat)] = [("light", .light, 1), ("dark", .dark, 1), ("high-contrast", .highContrastLight, 1), ("light-2.0", .light, 2)]
        for (suffix, mode, scale) in variants {
            let big: CGFloat = scale > 1 ? 1.9 : 1
            add("settings-memory-privacy-\(suffix)", mode, scale, width: scale > 1 ? 1400 : 900, height: 3800 * big,
                window("capture", pane: memoryPane(memoryPrivacy(encryption: encryptionModel(.encrypted)))))
        }
        // The Memory row in each state (the top of the section).
        add("settings-memory-on-light", .light, height: 600, window("capture", pane: memoryPane(memoryPrivacy())))
        add("settings-memory-paused-light", .light, height: 600, window("capture", pane: memoryPane(memoryPrivacy(memory: .pausedUntil("3:10 PM")))))
        add("settings-memory-off-light", .light, height: 600, window("capture", pane: memoryPane(memoryPrivacy(memory: .off))))
        add("settings-memory-not-capturing-light", .light, height: 600, window("capture", pane: memoryPane(memoryPrivacy(
            memory: .onNotCapturing, reason: "Whimbrel can't read text on your screen. Allow Accessibility in System Settings."))))
        add("settings-memory-not-capturing-dark", .dark, height: 600, window("capture", pane: memoryPane(memoryPrivacy(
            memory: .onNotCapturing, reason: "Whimbrel can't read text on your screen. Allow Accessibility in System Settings."))))
        add("settings-memory-highlight-excluded-light", .light, height: 1100, window("capture", pane: memoryPane(memoryPrivacy(), highlighted: "capture.excludeApps")))

        // Retention: Keep forever on, off and waiting, and the two confirms.
        add("settings-retention-keep-forever-light", .light, height: 560, AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                ForEach(retentionRows()) { row in WRetentionRowView(row: row, labels: labels, actions: WMemoryPrivacyActions()) }
            }
        }))
        add("settings-retention-choosing-light", .light, height: 560, AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                WRetentionRowView(row: retentionRows()[0], labels: labels, actions: WMemoryPrivacyActions(), initialChoosing: true)
                WRetentionRowView(row: retentionRows()[1], labels: labels, actions: WMemoryPrivacyActions(), initialDraft: 30)
            }
        }))
        add("settings-retention-choosing-dark", .dark, height: 560, AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                WRetentionRowView(row: retentionRows()[0], labels: labels, actions: WMemoryPrivacyActions(), initialChoosing: true)
                WRetentionRowView(row: retentionRows()[1], labels: labels, actions: WMemoryPrivacyActions(), initialDraft: 30)
            }
        }))
        add("settings-retention-choosing-light-2.0", .light, 2, width: 1400, height: 1500, AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                WRetentionRowView(row: retentionRows()[0], labels: labels, actions: WMemoryPrivacyActions(), initialChoosing: true)
                WRetentionRowView(row: retentionRows()[1], labels: labels, actions: WMemoryPrivacyActions(), initialDraft: 30)
            }
        }))
        add("settings-retention-invalid-light", .light, height: 360, AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                WRetentionRowView(
                    row: WRetentionRowModel(id: "meetings", kind: .meetings, label: "Meetings and recordings", days: 99999, keepForever: false, foreverTitle: "Keep forever", asksBeforeShortening: true),
                    labels: labels, actions: WMemoryPrivacyActions())
            }
        }))
        let meetingsAsk = WRetentionAsk(effect: .deletes, kind: .meetings, name: "Meetings and recordings", days: 30, replacing: .days(90))
        let screenAsk = WRetentionAsk(effect: .deletes, kind: .screenText, name: "Screen text", days: 30, replacing: .forever, count: 3904)
        let compressAsk = WRetentionAsk(effect: .recompressesAudio, kind: nil, name: "Compress audio after", days: 7, replacing: .days(30))
        let shrinkAsk = WRetentionAsk(effect: .shrinksImages, kind: nil, name: "Shrink images after", days: 30, replacing: .forever)
        add("settings-retention-confirm-shorter-light", .light, height: 360, AnyView(Panel {
            WConfirm(
                title: labels.askTitle(meetingsAsk), message: labels.askMessage(meetingsAsk), confirmTitle: labels.askConfirm(meetingsAsk),
                onConfirm: {}, onCancel: {})
        }))
        add("settings-retention-confirm-compress-light", .light, height: 360, AnyView(Panel {
            WConfirm(
                title: labels.askTitle(compressAsk), message: labels.askMessage(compressAsk), confirmTitle: labels.askConfirm(compressAsk),
                onConfirm: {}, onCancel: {})
        }))
        add("settings-retention-confirm-shrink-dark", .dark, height: 360, AnyView(Panel {
            WConfirm(
                title: labels.askTitle(shrinkAsk), message: labels.askMessage(shrinkAsk), confirmTitle: labels.askConfirm(shrinkAsk),
                onConfirm: {}, onCancel: {})
        }))
        add("settings-retention-confirm-from-forever-dark", .dark, height: 360, AnyView(Panel {
            WConfirm(
                title: labels.askTitle(screenAsk), message: labels.askMessage(screenAsk), confirmTitle: labels.askConfirm(screenAsk),
                onConfirm: {}, onCancel: {})
        }))
        // Every period is Keep forever: the stepper is off and says why, Apply Retention Now is off and says why, and Clean Up is off
        // before a scan. Dark and light, so the disabled text can be read in both.
        let disabledCard = AnyView(Panel {
            WSettingsCard(heading: "Keeping data") {
                WRetentionRowView(row: retentionRows()[0], labels: labels, actions: WMemoryPrivacyActions())
                WSettingsRow(
                    label: labels.applyNow, description: labels.applyNowHelp, buttonTitle: labels.applyNow, isEnabled: false, disabledReason: labels.applyNowNothing) {}
                WSettingsRow(label: "Orphaned recording files", description: "Recording files with no matching meeting record. " + labels.cleanUpNeedsScan) {
                    WFlowLayout(spacing: 8, lineSpacing: 6) {
                        WButton("Scan for Orphaned Files", kind: .secondary) {}
                        WButton(labels.cleanUp, kind: .secondary, reason: labels.cleanUpNeedsScan) {}.disabled(true)
                    }
                }
            }
        })
        add("settings-retention-disabled-dark", .dark, height: 520, disabledCard)
        add("settings-retention-disabled-high-contrast", .highContrastLight, height: 520, disabledCard)
        add("settings-retention-apply-confirm-light", .light, height: 360, AnyView(Panel {
            WConfirm(
                title: labels.applyConfirmTitle,
                message: labels.applyConfirmMessage([WRetentionApplyPlan.Limit(kind: .meetings, days: 90)]), confirmTitle: labels.applyConfirmButton,
                onConfirm: {}, onCancel: {})
        }))

        // Sheets.
        add("settings-memory-delete-sheet-light", .light, height: 700, AnyView(Panel {
            WDeleteEverythingSheet(model: memoryPrivacy().delete, labels: labels, typed: .constant(""), alsoSync: .constant(false), onCancel: {}, onDelete: {})
        }))
        add("settings-memory-delete-sheet-dark", .dark, height: 700, AnyView(Panel {
            WDeleteEverythingSheet(model: memoryPrivacy().delete, labels: labels, typed: .constant(""), alsoSync: .constant(true), onCancel: {}, onDelete: {})
        }))
        add("settings-memory-free-up-space-light", .light, height: 620, AnyView(Panel {
            WFreeUpSpaceSheet(
                model: WFreeUpModel(dayChoices: [30, 90, 180], days: 90, options: [
                    WFreeUpOption(id: "compact", title: "Compact database", detail: "Frees 120 MB inside the database file.", buttonTitle: "Compact"),
                    WFreeUpOption(id: "audio", title: "Delete old recordings", detail: "Recordings older than 90 days: 1.4 GB. Transcripts, notes and summaries stay.", buttonTitle: "Delete…", confirmQuestion: "Delete the audio of meetings older than 90 days? This can't be undone."),
                ]),
                labels: labels, actions: WFreeUpActions())
        }))

        // Library encryption in its three states, and the card in the window at the narrow size.
        for (name, state) in [("encrypted", WLibraryEncryptionModel.State.encrypted), ("not-encrypted", .notEncrypted), ("needs-attention", .needsAttention)] {
            add("settings-library-encryption-\(name)-light", .light, height: 760, AnyView(Panel {
                WLibraryEncryptionCard(model: encryptionModel(state), actions: WLibraryEncryptionActions(), labels: labels)
            }))
        }
        add("settings-library-encryption-not-encrypted-dark", .dark, height: 760, AnyView(Panel {
            WLibraryEncryptionCard(model: encryptionModel(.notEncrypted), actions: WLibraryEncryptionActions(), labels: labels)
        }))
        add("settings-library-encryption-offer-export-light", .light, height: 560, AnyView(Panel {
            WLibraryEncryptionCard(
                model: { var m = encryptionModel(.notEncrypted); m.phase = .offeringExport(exportOffered: false); return m }(),
                actions: WLibraryEncryptionActions(), labels: labels)
        }))
        add("settings-memory-privacy-narrow-light", .light, width: 520, height: 5200, window("capture", pane: memoryPane(memoryPrivacy(encryption: encryptionModel(.notEncrypted)))))
        add("settings-memory-privacy-narrow-dark", .dark, width: 520, height: 1000, window("capture", pane: memoryPane(memoryPrivacy(memory: .pausedUntil("3:10 PM")))))
        return out
    }
}
