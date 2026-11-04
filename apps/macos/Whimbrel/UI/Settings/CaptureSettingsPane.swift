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
