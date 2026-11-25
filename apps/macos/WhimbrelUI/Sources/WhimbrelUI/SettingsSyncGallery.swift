import SwiftUI

// Sync & Devices, Connectors and Automations from sample models, for `WhimbrelUIGallery --settings`. Made-up content only:
// made-up tokens, keys, addresses and device names. A saved secret is drawn as the part draws it, a fixed mask. Sheets and
// confirms are frames the app presents in its own sheet, so they are drawn here as their frames on a page-coloured panel.

@MainActor
extension WSettingsWindowGallery {
    static let anchorsSync = WSyncAnchors(
        overview: "devices.overview", syncNow: "devices.syncNow", name: "devices.name", phone: "devices.phone", deleteAccount: "devices.deleteAccount")
    static let anchorsConnectors = WConnectorsAnchors(
        calendar: "connectors.calendar", gmail: "connectors.gmail", outlook: "connectors.outlook", ownClient: "connectors.ownClient", mcp: "connectors.mcp")

    struct SyncPanel<Content: View>: View {
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

    // MARK: Sync & Devices

    static func qrSample() -> Image {
        // A made-up pattern, not a real code: the app draws the real one.
        let size = 21
        var rows: [[Bool]] = []
        for y in 0..<size { rows.append((0..<size).map { x in (x * 7 + y * 13 + x * y) % 3 == 0 || (x < 7 && y < 7 && (x == 0 || y == 0 || x == 6 || y == 6)) }) }
        let scale = 4
        let space = CGColorSpaceCreateDeviceGray()
        var bytes = [UInt8](repeating: 255, count: size * scale * size * scale)
        for y in 0..<(size * scale) { for x in 0..<(size * scale) where rows[y / scale][x / scale] { bytes[y * size * scale + x] = 0 } }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        let cg = CGImage(
            width: size * scale, height: size * scale, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: size * scale, space: space,
            bitmapInfo: CGBitmapInfo(rawValue: 0), provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        return Image(decorative: cg, scale: 1)
    }

    static func syncDevices(collision: Bool = false, loading: Bool = false) -> [WDeviceRow] {
        if loading { return [] }
        return [
            WDeviceRow.make(
                id: "d1", name: "Sam's MacBook Air", status: "approved", statusText: "Approved", isRunner: false, keyDiffersFromThisDevice: false, code: nil,
                collides: false, canRemoveRunner: false),
            WDeviceRow.make(
                id: "d2", name: "Studio iMac", status: "approved", statusText: "Approved", isRunner: false, keyDiffersFromThisDevice: true, code: nil,
                collides: false, canRemoveRunner: false),
            WDeviceRow.make(
                id: "d3", name: "Mac mini", status: "pending", statusText: "Pending", isRunner: false, keyDiffersFromThisDevice: true, code: "482 913",
                collides: collision, canRemoveRunner: false),
            WDeviceRow.make(
                id: "d4", name: "Unnamed device", status: "pending", statusText: "Pending", isRunner: false, keyDiffersFromThisDevice: true,
                code: collision ? "482 913" : nil, collides: collision, canRemoveRunner: false),
            WDeviceRow.make(
                id: "d5", name: "Whimbrel Cloud", status: "approved", statusText: "Approved", isRunner: true, keyDiffersFromThisDevice: false, code: nil,
                collides: false, canRemoveRunner: true, lastRan: "Last ran Nov 22, 2026 at 6:58 AM"),
        ]
    }

    static func sync(
        phase: WSyncPhase = .ready, phone: WPhoneCardModel? = nil, collision: Bool = false, loading: Bool = false,
        limit: WLimitModel? = nil, runner: Bool = true, stale: Bool = false, syncNowMessage: String? = nil, syncNowError: Bool = false, pending: Bool = true,
        confirmingPending: Bool = false
    ) -> WSyncModel {
        var l = WSyncLabels()
        l.removeRunnerMessage = "Whimbrel Cloud loses its access to your sync at once. Routines set to run in the cloud then run on your computer until you turn Run in the cloud off."
        l.removeDeviceMessage = "This device will be removed from the list of devices that still need to update."
        l.provisionInstructions = "Add these to Whimbrel Cloud's secret store. The provision value carries your sync key: paste it only there. It is shown once."
        let notes: [WSyncNote] = phase == .ready
            ? [WSyncNote(id: "s", text: "Settings last synced 2 minutes ago"), WSyncNote(id: "c", text: "Content last synced 1 minute ago"), WSyncNote(id: "i", text: "Indexing history: 42%")]
            : []
        return WSyncModel(
            labels: l, phase: phase, phone: phone ?? .beta(url: "https://example.com/mobile-beta/", qr: qrSample()),
            deviceName: "Sam's MacBook Air", actionError: nil, devices: phase == .ready ? syncDevices(collision: collision, loading: loading) : [],
            showsAddRunner: runner, limit: limit, deleteMessage: "This deletes the encrypted copy on the sync server and signs out every device. Nothing on your devices is deleted. Your Whimbrel AI credit of $4.20 is forfeited.",
            staleLine: stale ? "Sync hasn't worked since Nov 21, 2026 at 3:14 PM. Whimbrel keeps trying." : nil,
            syncNow: WSyncNowModel(isEnabled: phase == .ready, message: syncNowMessage, messageIsError: syncNowError), notes: notes,
            pending: phase == .ready && pending
                ? WPendingClientsModel(
                    line: "1 device still needs to update before it applies client deletes.", devices: [.init(id: "d9", name: "Old Mac Pro")],
                    confirmingName: confirmingPending ? "Old Mac Pro" : nil)
                : nil,
            anchors: anchorsSync)
    }

    static func limitScreen(joined: Bool = false, purchase: WPlanSettingsModel.Purchase = .plus) -> WLimitModel {
        WLimitModel(
            message: "The free plan includes one computer. Sam's MacBook Air is using it.", purchase: purchase,
            canSwap: true,
            waitlist: [
                WWaitlistOption(id: "devices", label: "More devices"), WWaitlistOption(id: "routines", label: "Routines that run while my computer is closed"),
                WWaitlistOption(id: "phone", label: "Notifications on my phone"), WWaitlistOption(id: "backup", label: "Backups"),
            ], joined: joined)
    }

    private static func syncPane(
        _ m: WSyncModel, highlighted: String? = nil, revoking: String? = nil, addingRunner: Bool = false, deleting: Bool = false
    ) -> AnyView {
        AnyView(WSyncDevicesSettingsView(
            model: m, actions: WSyncActions(), highlighted: highlighted,
            waiting: AnyView(Slot(title: "Waiting for approval on another device", text: "Open Whimbrel on a device that is already signed in, go to Settings, Sync & Devices, and approve this one. It will be picked up automatically within a few seconds.")),
            initiallyRevoking: revoking.flatMap { id in m.devices.first { $0.id == id } }, initiallyAddingRunner: addingRunner, initiallyDeleting: deleting))
    }

    // MARK: Connectors

    static func connectors(
        accounts: Bool = true, advanced: Bool = false, ownClient: WOwnClientModel? = nil, token: [WShownValue]? = nil, addState: WAddAccountState = .idle,
        clients: Bool = true, serverOn: Bool = true, formError: String? = nil
    ) -> WConnectorsModel {
        func reconnectAsk(_ what: String) -> WAskModel {
            WAskModel(title: "Remove this Google account?", message: "This removes this account's \(what) from this Mac and revokes Whimbrel's access to the account with Google.")
        }
        let cal: [WAccountRow] = accounts ? [
            WAccountRow(email: "sam.rivera@example.com", isDefault: true, detail: "14 events found in the next 7 days.", showsMakeDefault: true, removeQuestion: reconnectAsk("Calendar events")),
            WAccountRow(
                email: "studio@example.org", isDefault: false, detail: "Connected.",
                reconnect: "Reconnect needed. Click Add Google Account and sign in as this address again.", showsMakeDefault: true,
                removeQuestion: reconnectAsk("Calendar events")),
        ] : []
        let gm: [WAccountRow] = accounts ? [
            WAccountRow(
                email: "sam.rivera@example.com", isDefault: true, detail: "Sending only. Reading email needs your own Google project (Advanced).",
                showsMakeDefault: false, removeQuestion: reconnectAsk("Gmail data")),
        ] : []
        let ol: [WAccountRow] = accounts ? [
            WAccountRow(email: "sam@northwind.example", isDefault: true, detail: "6 events found in the next 7 days.", showsMakeDefault: false, removeQuestion: nil),
        ] : []
        return WConnectorsModel(
            labels: connectorLabels(),
            calendar: WConnectorModel(heading: "Google Calendar", accounts: cal, addTitle: "Add Google Account", addState: addState, anchor: "connectors.calendar"),
            gmail: WConnectorModel(heading: "Gmail", accounts: gm, addTitle: "Add Google Account", addState: .idle, anchor: "connectors.gmail"),
            outlook: WConnectorModel(heading: "Outlook 365", accounts: ol, addTitle: "Add Microsoft Account", addState: .idle, anchor: "connectors.outlook"),
            ownClient: ownClient ?? WOwnClientModel(isSaved: false, clientId: ""),
            mcp: WMcpModel(
                serverOn: serverOn, serverStatus: serverOn ? "Running at http://127.0.0.1:4319/mcp" : "Not running.", name: "Claude Desktop",
                scopes: [
                    WMcpScopeRow(id: "memory", label: "Memory", isOn: true), WMcpScopeRow(id: "meetings", label: "Meetings", isOn: true),
                    WMcpScopeRow(id: "transcripts", label: "Transcripts", isOn: false),
                    WMcpScopeRow(
                        id: "google_data", label: "Gmail and Google Calendar",
                        detail: "Also give it your Gmail and Google Calendar items, and the titles, people and summaries of meetings from Google Calendar. Without this, those are left out of everything above.",
                        isOn: false),
                ],
                canAdd: true, error: formError,
                clients: clients ? [
                    WMcpClientRow(id: "1", name: "Claude Desktop", detail: "Memory, Meetings, Write \u{00B7} created Nov 3, 2026"),
                    WMcpClientRow(id: "2", name: "Notes script", detail: "Transcripts \u{00B7} created Oct 12, 2026", revokedText: "Revoked Nov 5, 2026"),
                ] : []),
            shownToken: token, anchors: anchorsConnectors)
    }

    static func connectorLabels() -> WConnectorsLabels {
        var l = WConnectorsLabels()
        l.ownClientSavedNote = "Saved. Replace it to change the client, or remove it."
        l.saveClientNote = "Both fields are needed. Saving signs Google accounts connected with the other client out of it."
        l.removeClientMessage = "Gmail reading stops. Google accounts connected through this client show Reconnect needed, and you sign in to each again."
        return l
    }

    private static func connectorsPane(
        _ m: WConnectorsModel, highlighted: String? = nil, advanced: Bool = false, removingClient: Bool = false, revoking: String? = nil
    ) -> AnyView {
        AnyView(WConnectorsSettingsView(
            model: m, actions: WConnectorsActions(), highlighted: highlighted, initiallyAdvanced: advanced, initiallyRemovingClient: removingClient,
            initiallyRevoking: revoking.flatMap { id in m.mcp.clients.first { $0.id == id } }))
    }

    private static let sampleToken = WShownValue(id: "token", name: "", value: "whm_mcp_7f3a91c4e2b8d05a6c19f4e7a2b3d8c05e61", isSecret: true)

    // MARK: Automations

    static func automations(cloud: Bool = true, runner: Bool = true, devicesChosen: Bool = false) -> WAutomationsModel {
        let readyBy = Calendar(identifier: .gregorian).date(from: DateComponents(year: 2026, month: 11, day: 23, hour: 8, minute: 0)) ?? Date(timeIntervalSince1970: 0)
        let models = [
            WChoice(id: "default", title: "Default (Anthropic)"), WChoice(id: "cheapest", title: "Cheapest model you have"),
            WChoice(id: "ollama/example-small", title: "Ollama (example-small)"), WChoice(id: "anthropic/example-large", title: "Anthropic (example-large)"),
        ]
        let cloudOn = WRoutineCloud(
            isOn: true, toggleCopy: "Whimbrel Cloud holds your sync key. While this runs it can read your meetings, transcripts and client settings. It keeps nothing after the run.",
            disclosure: "Results already made are not recalled if you later delete a client. On the cloud, exclusion matches by name, alias and contact address; items you filed by hand are not known to the cloud.",
            statusNote: runner ? nil : "Whimbrel Cloud is not on your account yet, so this routine runs on your computer 3 hours after its ready-by time or at 23:30, whichever is earlier.",
            readyBy: readyBy, readyByNote: "Whimbrel Cloud starts early enough to have the result ready by this time. If no result has arrived by then, Settings shows the run as late, with the last result.",
            freshness: "Updated 6:58 AM")
        let cloudBlocked = WRoutineCloud(
            isOn: false, blockedReason: "Local-only mode is on, so Whimbrel Cloud can't use a hosted model for this routine.",
            toggleCopy: "Whimbrel Cloud holds your sync key. While this runs it can read your meetings and client settings. It keeps nothing after the run.",
            disclosure: "Results already made are not recalled if you later delete a client. On the cloud, exclusion matches by name, alias and contact address; items you filed by hand are not known to the cloud.")
        return WAutomationsModel(
            runsOn: WRunsOnModel(
                options: [WChoice(id: "", title: "Any device (default)"), WChoice(id: "k1", title: "Sam's MacBook Air"), WChoice(id: "k2", title: "Studio iMac")],
                selection: devicesChosen ? "k2" : "",
                chosenLine: devicesChosen ? "Chosen on Sam's MacBook Air, Nov 20, 2026 at 9:12 AM." : nil,
                note: devicesChosen
                    ? "Routines run on Studio iMac. If it has not run Daily Brief or Reply Priorities 3 hours after their time, this device runs them (devices in different time zones may then both run). Their on/off and time sync between devices. Results sync to every device."
                    : "Each device runs its own routines until you choose one."),
            routines: [
                WRoutineRow(id: "1", name: "Daily Brief", enabled: true, autoApprove: nil, timing: .dailyAt("07:00"), modelOptions: models, modelSelection: "default", cloud: cloud ? cloudOn : nil),
                WRoutineRow(id: "2", name: "End-of-day recap", enabled: true, autoApprove: false, timing: .dailyAt("17:30"), modelOptions: models, modelSelection: "cheapest", cloud: cloud ? cloudBlocked : nil),
                WRoutineRow(id: "3", name: "Meeting Prep", enabled: true, autoApprove: nil, timing: .leadMinutes(15, range: 5...120)),
                WRoutineRow(id: "4", name: "Focus Time", enabled: false, autoApprove: true, timing: .dailyAt("09:00")),
            ])
    }

    private static func automationsPane(_ m: WAutomationsModel) -> AnyView {
        AnyView(WAutomationsSettingsView(model: m, actions: WAutomationsActions()))
    }

    /// Each picture's height, fitted to its content when the pictures were last made.
    static let fittedHeightsSync: [String: CGFloat] = [
        "settings-automations-dark": 1230,
        "settings-automations-high-contrast": 1230,
        "settings-automations-light": 1230,
        "settings-automations-light-2.0": 2264,
        "settings-automations-narrow-light": 1480,
        "settings-automations-no-cloud-light": 825,
        "settings-automations-no-runner-dark": 1250,
        "settings-automations-ready-by-light": 154,
        "settings-connectors-add-error-light": 1317,
        "settings-connectors-advanced-open-light": 1740,
        "settings-connectors-advanced-saved-dark": 1748,
        "settings-connectors-connecting-dark": 1282,
        "settings-connectors-dark": 1748,
        "settings-connectors-high-contrast": 1750,
        "settings-connectors-highlight-own-client-light": 1748,
        "settings-connectors-light": 1748,
        "settings-connectors-light-2.0": 2827,
        "settings-connectors-narrow-light": 1970,
        "settings-connectors-no-accounts-light": 1203,
        "settings-connectors-remove-account-light": 205,
        "settings-connectors-remove-client-dark": 205,
        "settings-connectors-revoke-light": 205,
        "settings-connectors-token-close-confirm-high-contrast": 358,
        "settings-connectors-token-close-confirm-light": 358,
        "settings-connectors-token-sheet-dark": 294,
        "settings-connectors-token-sheet-light": 294,
        "settings-connectors-token-sheet-light-2.0": 524,
        "settings-sync-collision-high-contrast": 1214,
        "settings-sync-dark": 1418,
        "settings-sync-delete-confirm-dark": 223,
        "settings-sync-failed-dark": 576,
        "settings-sync-high-contrast": 1418,
        "settings-sync-highlight-name-light": 1256,
        "settings-sync-highlight-phone-dark": 1256,
        "settings-sync-light": 1418,
        "settings-sync-light-2.0": 2586,
        "settings-sync-limit-joined-dark": 655,
        "settings-sync-limit-light": 1011,
        "settings-sync-loading-devices-light": 587,
        "settings-sync-narrow-light": 1560,
        "settings-sync-ready-light": 1092,
        "settings-sync-revoke-confirm-light": 241,
        "settings-sync-runner-sheet-light": 252,
        "settings-sync-runner-values-dark": 539,
        "settings-sync-signed-out-light": 523,
        "settings-sync-signing-in-dark": 420,
        "settings-sync-waiting-light": 590,
    ]

    // MARK: Cases

    static func syncConnectorsAutomationsCases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ mode: WGalleryMode, _ scale: CGFloat = 1, width: CGFloat = 900, height: CGFloat, _ view: AnyView) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: fittedHeightsSync[id] ?? height, view: view))
        }
        let variants: [(String, WGalleryMode, CGFloat)] = [("light", .light, 1), ("dark", .dark, 1), ("high-contrast", .highContrastLight, 1), ("light-2.0", .light, 2)]
        for (suffix, mode, scale) in variants {
            let big: CGFloat = scale > 1 ? 1.9 : 1
            let wide: CGFloat = scale > 1 ? 1400 : 900
            add("settings-sync-\(suffix)", mode, scale, width: wide, height: 2200 * big, window("devices", pane: syncPane(sync(collision: true))))
            add("settings-connectors-\(suffix)", mode, scale, width: wide, height: 2800 * big, window("connectors", pane: connectorsPane(connectors(ownClient: WOwnClientModel(isSaved: true, clientId: "1234-example.apps.example.com"), token: nil), advanced: true)))
            add("settings-automations-\(suffix)", mode, scale, width: wide, height: 2400 * big, window("automations", pane: automationsPane(automations(devicesChosen: true))))
        }
        // Sync & Devices states.
        add("settings-sync-signed-out-light", .light, height: 900, window("devices", pane: syncPane(sync(phase: .signedOut, runner: false, pending: false))))
        add("settings-sync-signing-in-dark", .dark, height: 900, window("devices", pane: syncPane(sync(phase: .signingIn, phone: .connected, runner: false, pending: false))))
        add("settings-sync-waiting-light", .light, height: 900, window("devices", pane: syncPane(sync(phase: .waiting, runner: false, pending: false))))
        add("settings-sync-failed-dark", .dark, height: 900, window("devices", pane: syncPane(sync(phase: .failed("Too many sign-in attempts. Try again in a few minutes."), runner: false, pending: false))))
        add("settings-sync-ready-light", .light, height: 2200, window("devices", pane: syncPane(sync(phone: .connected, collision: false, stale: true, syncNowMessage: "Synced"))))
        add("settings-sync-loading-devices-light", .light, height: 1500, window("devices", pane: syncPane(sync(phone: .connected, loading: true, runner: false, pending: false))))
        add("settings-sync-limit-light", .light, height: 1700, window("devices", pane: syncPane(sync(phase: .deviceLimit, limit: limitScreen(), runner: false, pending: false))))
        add("settings-sync-limit-joined-dark", .dark, height: 1500, window("devices", pane: syncPane(sync(phase: .deviceLimit, limit: limitScreen(joined: true, purchase: .none), runner: false, pending: false))))
        add("settings-sync-collision-high-contrast", .highContrastLight, height: 2200, window("devices", pane: syncPane(sync(phone: .connected, collision: true))))
        add("settings-sync-highlight-name-light", .light, height: 2200, window("devices", pane: syncPane(sync(), highlighted: "devices.name")))
        add("settings-sync-highlight-phone-dark", .dark, height: 2200, window("devices", pane: syncPane(sync(), highlighted: "devices.phone")))
        add("settings-sync-narrow-light", .light, width: 520, height: 3000, window("devices", pane: syncPane(sync(collision: true))))
        add("settings-sync-revoke-confirm-light", .light, height: 360, AnyView(SyncPanel {
            WConfirm(title: sync().labels.revokeTitle, message: sync().labels.revokeMessage, confirmTitle: "Revoke", onConfirm: {}, onCancel: {})
        }))
        add("settings-sync-delete-confirm-dark", .dark, height: 360, AnyView(SyncPanel {
            WConfirm(title: sync().labels.deleteAccountTitle, message: sync().deleteMessage, confirmTitle: "Delete", onConfirm: {}, onCancel: {})
        }))
        add("settings-sync-runner-sheet-light", .light, height: 460, AnyView(SyncPanel {
            WAddRunnerSheet(labels: sync().labels, register: { _ in .failure("That is not a public key.") }, copy: { _ in true }, onClose: {})
        }))
        add("settings-sync-runner-values-dark", .dark, height: 760, AnyView(SyncPanel {
            WShownOnceSheet(
                title: "Whimbrel Cloud", message: sync().labels.provisionInstructions,
                values: [
                    WShownValue(id: "p", name: "RUNNER_PROVISION", value: "eyJleGFtcGxlIjoibWFkZS11cC1ib3VuZGxlLW5vdC1yZWFsIiwid3JhcHBlZCI6ImV4YW1wbGUifQ==", isSecret: true),
                    WShownValue(id: "k", name: "RUNNER_APPROVER_PUBLIC_KEY", value: "MCowBQYDK2VuAyEAexampleexampleexampleexampleexample00=", isSecret: false),
                    WShownValue(id: "u", name: "RUNNER_WORKER_URL", value: "https://sync.example.com", isSecret: false),
                ], copyTitle: "Copy", copiedTitle: "Copied", doneTitle: "Done", closeQuestion: sync().labels.runnerCloseQuestion,
                closeAnywayTitle: "Close without copying", keepOpenTitle: "Keep it open", copy: { _ in true }, onClose: {})
        }))

        // Connectors states.
        add("settings-connectors-no-accounts-light", .light, height: 2200, window("connectors", pane: connectorsPane(connectors(accounts: false, clients: false, serverOn: false))))
        add("settings-connectors-connecting-dark", .dark, height: 2400, window("connectors", pane: connectorsPane(connectors(accounts: false, addState: .connecting))))
        add("settings-connectors-add-error-light", .light, height: 2400, window("connectors", pane: connectorsPane(connectors(accounts: false, addState: .error("Google sign-in was cancelled.")))))
        add("settings-connectors-advanced-open-light", .light, height: 2600, window("connectors", pane: connectorsPane(connectors(), advanced: true)))
        add("settings-connectors-advanced-saved-dark", .dark, height: 2600, window("connectors", pane: connectorsPane(connectors(ownClient: WOwnClientModel(isSaved: true, clientId: "1234-example.apps.example.com")))))
        add("settings-connectors-highlight-own-client-light", .light, height: 2600, window("connectors", pane: connectorsPane(connectors(ownClient: WOwnClientModel(isSaved: true, clientId: "1234-example.apps.example.com")), highlighted: "connectors.ownClient")))
        add("settings-connectors-narrow-light", .light, width: 520, height: 4200, window("connectors", pane: connectorsPane(connectors(), advanced: true)))
        add("settings-connectors-remove-account-light", .light, height: 360, AnyView(SyncPanel {
            WConfirm(
                title: "Remove this Google account?",
                message: "This removes this account's Gmail data from this Mac and revokes Whimbrel's access to the account with Google.",
                confirmTitle: "Remove", onConfirm: {}, onCancel: {})
        }))
        add("settings-connectors-remove-client-dark", .dark, height: 360, AnyView(SyncPanel {
            WConfirm(title: connectorLabels().removeClientTitle, message: connectorLabels().removeClientMessage, confirmTitle: "Remove", onConfirm: {}, onCancel: {})
        }))
        add("settings-connectors-revoke-light", .light, height: 360, AnyView(SyncPanel {
            WConfirm(title: connectorLabels().revokeTitle, message: connectorLabels().revokeMessage, confirmTitle: "Revoke", onConfirm: {}, onCancel: {})
        }))
        let tokenLabels = connectorLabels()
        func tokenSheet(asking: Bool) -> AnyView {
            AnyView(SyncPanel {
                WShownOnceSheet(
                    title: tokenLabels.tokenTitle, message: tokenLabels.tokenMessage, values: [sampleToken], copyTitle: tokenLabels.copy,
                    copiedTitle: tokenLabels.copied, doneTitle: tokenLabels.done, closeQuestion: tokenLabels.tokenCloseQuestion,
                    closeAnywayTitle: tokenLabels.closeWithoutCopying, keepOpenTitle: tokenLabels.keepOpen, copy: { _ in true }, onClose: {}, startsAsking: asking)
            })
        }
        add("settings-connectors-token-sheet-light", .light, height: 440, tokenSheet(asking: false))
        add("settings-connectors-token-sheet-dark", .dark, height: 440, tokenSheet(asking: false))
        add("settings-connectors-token-close-confirm-light", .light, height: 520, tokenSheet(asking: true))
        add("settings-connectors-token-close-confirm-high-contrast", .highContrastLight, height: 520, tokenSheet(asking: true))
        add("settings-connectors-token-sheet-light-2.0", .light, 2, width: 1100, height: 1100, tokenSheet(asking: true))

        // Automations states.
        add("settings-automations-no-cloud-light", .light, height: 1500, window("automations", pane: automationsPane(automations(cloud: false))))
        add("settings-automations-no-runner-dark", .dark, height: 2400, window("automations", pane: automationsPane(automations(runner: false))))
        add("settings-automations-narrow-light", .light, width: 520, height: 3600, window("automations", pane: automationsPane(automations(devicesChosen: true))))
        add("settings-automations-ready-by-light", .light, height: 300, AnyView(SyncPanel {
            WSettingsCard {
                WReadyByRow(
                    label: "Ready by", note: "Whimbrel Cloud starts early enough to have the result ready by this time.",
                    time: Calendar(identifier: .gregorian).date(from: DateComponents(year: 2026, month: 11, day: 23, hour: 8, minute: 0)) ?? Date(timeIntervalSince1970: 0),
                    onChange: { _ in })
                WTimeFieldRow(label: "Runs daily at", placeholder: "HH:MM", invalid: "Use a time like 07:30.", stored: "07:00", initialText: "7:5", onCommit: { _ in })
            }
        }))
        return out
    }
}
