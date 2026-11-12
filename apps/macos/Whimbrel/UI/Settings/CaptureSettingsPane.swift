import SwiftUI

struct CaptureSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var exclusions = CaptureExclusions.shared
    @State private var searchText = ""
    @State private var apps: [(bundleId: String, name: String)] = []
    @State private var newDomain = ""
    @State private var domainRefused = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Capture").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)

            Toggle("Show screen captures in Library", isOn: $appState.showScreenCaptures)
                .tint(Color.whimbrelAccentPrimary)
            Text("Off by default — the Recent tab shows meetings only. Screen-text captures are still recorded and searchable (Keyword/Semantic/Ask); this only controls whether they appear in the Recent list.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            HotkeyPanelRow(hotkey: appState.hotkeyService)

            Divider().overlay(Color.whimbrelBorder)

            Text("Exclude apps from screen-text capture. Excluded apps' content is never read: the check happens before any Accessibility or OCR extraction, not as a filter afterward.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            TextField("Search running apps…", text: $searchText)
                .textFieldStyle(.roundedBorder)

            if filteredApps.isEmpty {
                Text(apps.isEmpty ? "No running apps found." : "No matches.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                VStack(alignment: .leading, spacing: Spacing.sm) {
                    ForEach(filteredApps, id: \.bundleId) { app in
                        Toggle(isOn: Binding(
                            get: { exclusions.isExcluded(app.bundleId) },
                            set: { exclusions.setExcluded($0, for: app.bundleId) }
                        )) {
                            VStack(alignment: .leading) {
                                Text(app.name)
                                Text(app.bundleId).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                            }
                        }
                        .tint(Color.whimbrelAccentPrimary)
                    }
                }
            }

            Divider().overlay(Color.whimbrelBorder)

            Text("Sites never captured")
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Pages on these sites, and their subdomains, are skipped before any text is read. Works in browsers that expose the page address.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                TextField("example.com", text: $newDomain)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(addDomain)
                Button("Add") { addDomain() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(newDomain.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if domainRefused {
                Text("That doesn't look like a site. Enter a domain such as example.com.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
            if exclusions.excludedDomains.isEmpty {
                Text("No sites excluded.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(exclusions.excludedDomains.sorted(), id: \.self) { domain in
                    HStack {
                        Text(domain)
                        Spacer()
                        Button("Remove") { exclusions.removeExcludedDomain(domain) }
                            .buttonStyle(.whimbrelSecondary)
                    }
                }
            }
        }
        .onAppear { apps = exclusions.candidateApps() }
    }

    private func addDomain() {
        domainRefused = !exclusions.addExcludedDomain(newDomain)
        if !domainRefused { newDomain = "" }
    }

    private var filteredApps: [(bundleId: String, name: String)] {
        guard !searchText.isEmpty else { return apps }
        return apps.filter { $0.name.localizedCaseInsensitiveContains(searchText) }
    }
}

/// Settings row for the global hotkey: a recorder, a reset, and the failure message.
private struct HotkeyPanelRow: View {
    @ObservedObject var hotkey: HotkeyService
    @State private var recording = false
    @State private var monitor: Any?
    /// Why the last shortcut pressed or chosen wasn't used, until the next one is.
    @State private var refusal: String?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Hotkey panel")
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Press this shortcut anywhere to ask about the window you are in.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(recording ? "Press a shortcut…" : hotkey.shortcut.displayString) { toggleRecording() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Reset to default") { stopRecording(); show(hotkey.resetToDefault(), for: HotkeyService.defaultShortcut) }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(hotkey.shortcut == HotkeyService.defaultShortcut)
            }
            if let refusal {
                Text(refusal)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if case .failed = hotkey.status {
                Text("Another app already uses this shortcut")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
        }
        .onDisappear { stopRecording() }
    }

    private func toggleRecording() {
        if recording { stopRecording(); return }
        refusal = nil
        recording = true
        // Local monitor: sees only this app's key events, so it needs no permission.
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            if event.keyCode == 53 { stopRecording(); return nil } // Esc cancels
            let mods = Shortcut.carbonModifiers(event.modifierFlags)
            let candidate = Shortcut(keyCode: UInt32(event.keyCode), modifiers: mods)
            let change = hotkey.setShortcut(candidate)
            show(change, for: candidate)
            // A refused combination keeps recording, so the next press can try another.
            if case .rejected = change {} else { stopRecording() }
            return nil
        }
    }

    private func show(_ change: HotkeyService.Change, for candidate: Shortcut) {
        switch change {
        case .applied:
            refusal = nil
        case .rejected(.noModifier):
            refusal = String(localized: "\(candidate.displayString) has no modifier key, so it would block normal typing. Include Control or Option.")
        case .rejected(.needsControlOrOption):
            refusal = String(localized: "\(candidate.displayString) would take a shortcut apps use themselves. Include Control or Option, or use two modifier keys.")
        case .failed(let error):
            let why = error == .alreadyInUse
                ? String(localized: "\(candidate.displayString) is already used by another app.")
                : String(localized: "macOS wouldn't accept \(candidate.displayString).")
            // Only claim the old shortcut is in use when it is registered again.
            refusal = hotkey.status == .registered
                ? why + " " + String(localized: "Still using \(hotkey.shortcut.displayString).")
                : why
        }
    }

    private func stopRecording() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        recording = false
    }
}
