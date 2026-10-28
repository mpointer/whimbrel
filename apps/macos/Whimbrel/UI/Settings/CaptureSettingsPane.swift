import SwiftUI

struct CaptureSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var exclusions = CaptureExclusions.shared
    @State private var searchText = ""
    @State private var apps: [(bundleId: String, name: String)] = []

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

            Text("Exclude apps from screen-text capture. Excluded apps' content is never read — checked before any Accessibility/OCR extraction happens, not filtered out afterward.")
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
        }
        .onAppear { apps = exclusions.candidateApps() }
    }

    private var filteredApps: [(bundleId: String, name: String)] {
        guard !searchText.isEmpty else { return apps }
        return apps.filter { $0.name.localizedCaseInsensitiveContains(searchText) }
    }
}
