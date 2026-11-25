import SwiftUI
import WhimbrelUI

/// Profile in the redesigned Settings window: the model builder (pure, tested) and the view that feeds the package's
/// `WProfileSettingsView` and calls the same `ProfileSettings` and `ProfileSuggestionsModel` functions the classic pane calls.
enum ProfileSettingsAdapter {
    static func labels() -> WProfileLabels {
        var l = WProfileLabels()
        l.profile = String(localized: "Profile")
        l.intro = String(localized: "Whimbrel uses this to frame summaries, answers, meeting briefs and recaps for you. It follows your routing settings, so Local-only keeps it on this Mac.")
        l.useProfile = String(localized: "Use my profile in summaries and answers")
        l.suggest = String(localized: "Suggest additions from my meetings")
        l.name = String(localized: "Name")
        l.role = String(localized: "Role")
        l.organization = String(localized: "Organization")
        l.about = String(localized: "About me and my work")
        l.learnedFacts = String(localized: "Learned facts")
        l.addFact = String(localized: "Add a fact")
        l.addFactPlaceholder = String(localized: "A short fact about you", comment: "Grey text in the field where you type a fact about yourself in Settings, Profile.")
        l.add = String(localized: "Add")
        l.fromMeeting = String(localized: "From meeting")
        l.suggested = String(localized: "Suggested")
        l.suggestedEmpty = String(localized: "Once your Profile has something in it, Whimbrel looks at your day's meeting summaries once a day and suggests facts here. Nothing is added until you press Add.")
        l.source = String(localized: "Source")
        l.dismiss = String(localized: "Dismiss")
        l.whatSees = String(localized: "What Whimbrel sees")
        l.factError = String(localized: "That fact is blank, already listed, too long (200 characters), or the list is full (30).")
        return l
    }

    struct Input {
        var enabled: Bool
        var suggestEnabled: Bool
        var name: String
        var role: String
        var organization: String
        var about: String
        /// `ProfileSettings.routingProfile`: what is sent, empty when the switch is off or nothing is written.
        var routingProfile: String
        var facts: [ProfileFact]
        var suggestions: [CoreProfileSuggestion]
    }

    static func model(_ i: Input) -> WProfileSettingsModel {
        WProfileSettingsModel(
            labels: labels(), enabled: i.enabled, suggestEnabled: i.suggestEnabled, name: i.name, role: i.role,
            organization: i.organization, about: i.about,
            aboutFooter: String(localized: "\(i.routingProfile.count) of \(ProfileComposer.maxChars) characters sent"),
            facts: i.facts.map { WProfileSettingsModel.Item(id: $0.id, text: $0.text, hasMeeting: $0.sourceMeetingId != nil) },
            suggestions: i.suggestions.map { WProfileSettingsModel.Item(id: String($0.id), text: $0.text, hasMeeting: $0.sourceMeetingId != nil) },
            preview: i.routingProfile.isEmpty ? String(localized: "Nothing is sent.") : i.routingProfile,
            anchors: SettingsAnchorID.profile)
    }
}

struct ProfileSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        ProfileSettingsRedesignBody(profile: appState.profile, suggestions: appState.profileSuggestions, highlighted: highlighted)
    }
}

private struct ProfileSettingsRedesignBody: View {
    @ObservedObject var profile: ProfileSettings
    @ObservedObject var suggestions: ProfileSuggestionsModel
    let highlighted: String?
    @Environment(\.openWindow) private var openWindow

    private var model: WProfileSettingsModel {
        ProfileSettingsAdapter.model(ProfileSettingsAdapter.Input(
            enabled: profile.enabled, suggestEnabled: profile.suggestEnabled, name: profile.name, role: profile.role,
            organization: profile.organization, about: profile.about, routingProfile: profile.routingProfile,
            facts: profile.facts, suggestions: suggestions.pending))
    }

    private func suggestion(_ id: String) -> CoreProfileSuggestion? { suggestions.pending.first { String($0.id) == id } }

    private var actions: WProfileSettingsActions {
        WProfileSettingsActions(
            setEnabled: { profile.enabled = $0 },
            setSuggestEnabled: { profile.suggestEnabled = $0 },
            setName: { profile.name = $0 },
            setRole: { profile.role = $0 },
            setOrganization: { profile.organization = $0 },
            setAbout: { profile.about = $0 },
            updateFact: { id, text in profile.updateFact(id: id, text: text) },
            removeFact: { profile.removeFact(id: $0) },
            openFactMeeting: { id in
                if let meetingId = profile.facts.first(where: { $0.id == id })?.sourceMeetingId { openWindow(id: "meeting-detail", value: meetingId) }
            },
            addFact: { profile.addFact($0, sourceMeetingId: nil) },
            addSuggestion: { id in if let s = suggestion(id) { Task { await suggestions.add(s) } } },
            dismissSuggestion: { id in if let s = suggestion(id) { Task { await suggestions.dismiss(s) } } },
            openSuggestionMeeting: { id in
                if let meetingId = suggestion(id)?.sourceMeetingId { openWindow(id: "meeting-detail", value: meetingId) }
            })
    }

    var body: some View {
        WProfileSettingsView(model: model, actions: actions, highlighted: highlighted)
            .task { await suggestions.reload() }
    }
}
