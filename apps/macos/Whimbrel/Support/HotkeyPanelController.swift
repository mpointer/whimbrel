import AppKit
import SwiftUI

/// A borderless, non-activating panel that can still become key, so its field takes
/// typing while the app the user was in stays active (its focused element and selection
/// stay valid for the reader, and Whimbrel's other windows are not raised).
final class HotkeyPanel: NSPanel {
    var onCancel: (() -> Void)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// Escape.
    override func cancelOperation(_ sender: Any?) { onCancel?() }

    /// Keeps the top edge fixed as the content grows (an answer arrives) or shrinks.
    override func setContentSize(_ size: NSSize) {
        let top = frame.maxY
        super.setContentSize(size)
        setFrameTopLeftPoint(NSPoint(x: frame.minX, y: top))
    }
}

/// Owns the hotkey panel, modelled on `MeetingAlertPanelController.ensurePanel()` but able
/// to become key. Opened by the global hotkey or the menu-bar item; the hotkey again or
/// Escape closes it. Closing discards the answer.
@MainActor
final class HotkeyPanelController {
    static let panelWidth: CGFloat = 520

    let model: HotkeyPanelModel
    /// Opens Settings → AI Providers. Set once by `AppState`.
    var openAIProviders: () -> Void = {}

    private var panel: HotkeyPanel?
    private var openTask: Task<Void, Never>?

    init(model: HotkeyPanelModel) {
        self.model = model
    }

    var isVisible: Bool { panel?.isVisible == true }
    var panelForTesting: NSPanel? { panel }

    /// The hotkey: opens for `frontmost` (captured before anything else ran), or closes.
    func toggle(frontmost: HotkeyPanelModel.Frontmost?) {
        if isVisible { close() } else { open(frontmost: frontmost) }
    }

    func open(frontmost: HotkeyPanelModel.Frontmost?) {
        let panel = ensurePanel()
        position(panel, pid: frontmost?.pid)
        panel.makeKeyAndOrderFront(nil)
        openTask?.cancel()
        // No frontmost app with a bundle id: pid 0 makes the model report it unreadable.
        let target = frontmost ?? ("", "", 0)
        openTask = Task { [model] in await model.open(frontmost: target) }
    }

    func close() {
        openTask?.cancel()
        openTask = nil
        model.close()
        panel?.orderOut(nil)
    }

    private func ensurePanel() -> HotkeyPanel {
        if let panel { return panel }
        let host = NSHostingController(rootView: ThemedRoot {
            HotkeyPanelView(model: self.model, openAIProviders: { [weak self] in
                self?.close()
                self?.openAIProviders()
            }, close: { [weak self] in self?.close() })
        })
        host.sizingOptions = [.preferredContentSize]

        let panel = HotkeyPanel(
            contentRect: NSRect(origin: .zero, size: NSSize(width: Self.panelWidth, height: 120)),
            styleMask: [.nonactivatingPanel, .borderless],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        // `whimbrelCard()` draws its own shadow (as in `MeetingAlertPanelController`).
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.contentViewController = host
        panel.onCancel = { [weak self] in self?.close() }

        self.panel = panel
        return panel
    }

    /// Centred near the top of the screen that shows the frontmost app's front window.
    private func position(_ panel: NSPanel, pid: pid_t?) {
        let screen = pid.flatMap(Self.screenOfFrontWindow(pid:)) ?? NSScreen.main ?? NSScreen.screens.first
        guard let frame = screen?.visibleFrame else { return }
        let size = panel.frame.size
        let top = frame.maxY - frame.height * 0.2
        panel.setFrameTopLeftPoint(NSPoint(x: frame.midX - size.width / 2, y: top))
    }

    /// The screen holding the centre of `pid`'s frontmost on-screen window. Window Server
    /// bounds only (no AX call), so it is cheap on the main thread.
    private static func screenOfFrontWindow(pid: pid_t) -> NSScreen? {
        guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
                as? [[String: Any]] else { return nil }
        // Front to back; layer 0 is the normal window layer.
        guard let info = list.first(where: {
            ($0[kCGWindowOwnerPID as String] as? pid_t) == pid && ($0[kCGWindowLayer as String] as? Int) == 0
        }), let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: boundsDict) else { return nil }
        // Window Server coordinates have their origin at the top left of the primary display.
        guard let primaryHeight = NSScreen.screens.first?.frame.height else { return nil }
        let centre = NSPoint(x: bounds.midX, y: primaryHeight - bounds.midY)
        return NSScreen.screens.first { $0.frame.contains(centre) }
    }
}

/// Which app was frontmost before Whimbrel came forward, for the menu-bar entry: opening
/// the menu-bar window may itself make Whimbrel frontmost.
@MainActor
final class FrontmostAppTracker {
    struct Activation {
        let app: HotkeyPanelModel.Frontmost
        let at: Date
    }

    private(set) var current: Activation?
    private(set) var previous: Activation?
    private var observer: NSObjectProtocol?

    func start() {
        if let app = NSWorkspace.shared.frontmostApplication.flatMap(Self.frontmost) {
            current = Activation(app: app, at: .distantPast)
        }
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] note in
            let app = (note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication).flatMap(Self.frontmost)
            MainActor.assumeIsolated {
                guard let self, let app else { return }
                self.record(app, at: Date())
            }
        }
    }

    func record(_ app: HotkeyPanelModel.Frontmost, at date: Date) {
        if current?.app.pid == app.pid { return }
        previous = current
        current = Activation(app: app, at: date)
    }

    /// The app the user was in when they opened the menu at `menuOpenedAt`. If Whimbrel
    /// became frontmost only as the menu opened (within `window`), that is the app before
    /// it; otherwise the frontmost app, which may be Whimbrel itself (the panel then says
    /// it is excluded).
    func appBeforeMenu(openedAt menuOpenedAt: Date, ownBundleId: String?, window: TimeInterval = 1.0) -> HotkeyPanelModel.Frontmost? {
        guard let current else { return nil }
        if current.app.bundleId == ownBundleId, abs(menuOpenedAt.timeIntervalSince(current.at)) <= window, let previous {
            return previous.app
        }
        return current.app
    }

    nonisolated static func frontmost(_ app: NSRunningApplication) -> HotkeyPanelModel.Frontmost? {
        guard let id = app.bundleIdentifier else { return nil }
        return (id, app.localizedName ?? id, app.processIdentifier)
    }
}
