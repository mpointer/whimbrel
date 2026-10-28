import SwiftUI

/// Settings > Profile: who you are (four fields), the facts Whimbrel may use, suggestions waiting
/// for a yes or no, the two switches and the exact text Whimbrel sends.
struct ProfileSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    @State private var newFact = ""
    @State private var factError: String?

    private var profile: ProfileSettings { appState.profile }
    private var suggestions: ProfileSuggestionsModel { appState.profileSuggestions }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Profile").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Whimbrel uses this to frame summaries, answers, meeting briefs and recaps for you. It follows your routing settings, so Local-only keeps it on this Mac.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)

            Toggle("Use my profile in summaries and answers", isOn: Binding(get: { profile.enabled }, set: { profile.enabled = $0 }))
                .accessibilityIdentifier("profile-enabled")
            Toggle("Suggest additions from my meetings", isOn: Binding(get: { profile.suggestEnabled }, set: { profile.suggestEnabled = $0 }))
                .accessibilityIdentifier("profile-suggest-enabled")

            Divider().overlay(Color.whimbrelBorder)
            field("Name", text: Binding(get: { profile.name }, set: { profile.name = $0 }))
            field("Role", text: Binding(get: { profile.role }, set: { profile.role = $0 }))
            field("Organization", text: Binding(get: { profile.organization }, set: { profile.organization = $0 }))
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text("About me and my work").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                TextEditor(text: Binding(get: { profile.about }, set: { profile.about = $0 }))
                    .font(.whimbrelBody(.body))
                    .frame(minHeight: 100)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder, lineWidth: 1))
                    .accessibilityIdentifier("profile-about")
                Text("\(profile.routingProfile.count) of \(ProfileComposer.maxChars) characters sent")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }

            Divider().overlay(Color.whimbrelBorder)
            factsSection
            suggestedSection

            Divider().overlay(Color.whimbrelBorder)
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text("What Whimbrel sees").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                Text(profile.routingProfile.isEmpty ? "Nothing is sent." : profile.routingProfile)
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("profile-preview")
            }
        }
        .task { await suggestions.reload() }
    }

    private func field(_ title: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(title).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            TextField(title, text: text).textFieldStyle(.roundedBorder).labelsHidden()
        }
    }

    private var factsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Learned facts").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            ForEach(profile.facts) { fact in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                    TextField("Fact", text: Binding(
                        get: { fact.text },
                        set: { _ = profile.updateFact(id: fact.id, text: $0) }
                    ))
                    .textFieldStyle(.plain)
                    if let meetingId = fact.sourceMeetingId {
                        Button("From meeting") { openWindow(id: "meeting-detail", value: meetingId) }
                            .buttonStyle(.plain).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelLink)
                    }
                    Button { profile.removeFact(id: fact.id) } label: { Image(systemName: "xmark.circle") }
                        .buttonStyle(.plain).foregroundStyle(Color.whimbrelTextSecondary)
                        .accessibilityLabel("Delete fact")
                }
            }
            HStack {
                TextField("Add a fact", text: $newFact).textFieldStyle(.roundedBorder).onSubmit(addTyped)
                Button("Add", action: addTyped).buttonStyle(.whimbrelSecondary).disabled(newFact.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if let factError {
                Text(factError).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
        }
    }

    private func addTyped() {
        if profile.addFact(newFact, sourceMeetingId: nil) {
            newFact = ""
            factError = nil
        } else {
            factError = "That fact is blank, already listed, too long (200 characters), or the list is full (30)."
        }
    }

    private var suggestedSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Suggested").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if suggestions.pending.isEmpty {
                Text("Once your Profile has something in it, Whimbrel looks at your day's meeting summaries once a day and suggests facts here. Nothing is added until you press Add.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(suggestions.pending) { s in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                    Text(s.text).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextPrimary)
                    Spacer(minLength: Spacing.sm)
                    if let meetingId = s.sourceMeetingId {
                        Button("Source") { openWindow(id: "meeting-detail", value: meetingId) }
                            .buttonStyle(.plain).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelLink)
                    }
                    Button("Add") { Task { await suggestions.add(s) } }.buttonStyle(.whimbrelPrimary)
                    Button("Dismiss") { Task { await suggestions.dismiss(s) } }.buttonStyle(.whimbrelSecondary)
                }
            }
        }
    }
}
