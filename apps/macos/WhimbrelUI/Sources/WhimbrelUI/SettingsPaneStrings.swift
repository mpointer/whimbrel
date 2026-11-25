import Foundation

// The English words and identifiers of the Settings panes (General, Profile, Plan & billing, the skin gallery). The app
// fills every field from its string catalog; these are the defaults the gallery shows.

public struct WGeneralLabels: Equatable, Sendable {
    public var general = "General"
    public var appVersion = "App version"
    public var coreVersion = "Core version"
    public var checkForUpdates = "Check for Updates"
    /// The name of the row whose button is "Check for Updates".
    public var updatesLabel = "Updates"
    public var restartToUpdate = "Restart to Update"
    public var language = "Language"
    public var writeIn = "Write notes and summaries in"
    public var appearance = "Appearance"
    public var skin = "Skin"
    public var makeSkin = "Make a skin"
    public init() {}
}

public struct WSkinGalleryLabels: Equatable, Sendable {
    public var footer = "Each Mac keeps its own skin."
    public var importTitle = "Import Skin\u{2026}"
    public var rename = "Rename\u{2026}"
    public var export = "Export\u{2026}"
    public var delete = "Delete\u{2026}"
    public var deleteConfirm = "Delete"
    public var nameField = "Name"
    public var noSkins = "No skins are installed."
    public init() {}
}

public struct WProfileLabels: Equatable, Sendable {
    public var profile = "Profile"
    public var intro = "Whimbrel uses this to frame summaries, answers, meeting briefs and recaps for you. It follows your routing settings, so Local-only keeps it on this Mac."
    public var useProfile = "Use my profile in summaries and answers"
    public var suggest = "Suggest additions from my meetings"
    public var name = "Name"
    public var role = "Role"
    public var organization = "Organization"
    public var about = "About me and my work"
    public var learnedFacts = "Learned facts"
    public var addFact = "Add a fact"
    /// The grey text in the empty field. The row is named by `addFact` and the button by `add`.
    public var addFactPlaceholder = "A short fact about you"
    public var add = "Add"
    public var fromMeeting = "From meeting"
    public var suggested = "Suggested"
    public var suggestedEmpty = "Once your Profile has something in it, Whimbrel looks at your day's meeting summaries once a day and suggests facts here. Nothing is added until you press Add."
    public var source = "Source"
    public var dismiss = "Dismiss"
    public var whatSees = "What Whimbrel sees"
    public var factError = "That fact is blank, already listed, too long (200 characters), or the list is full (30)."
    public init() {}
}

public struct WPlanLabels: Equatable, Sendable {
    public var heading = "Plan & billing"
    public var signedOut = "Sign in to sync to see your plan."
    public var signIn = "Sign in to sync"
    public var loading = "Checking your plan\u{2026}"
    public var readFailed = "Couldn\u{2019}t check your plan. Try again in a moment."
    public var tryAgain = "Try again"
    public var finishSigningIn = "Finish signing in to manage your plan."
    public var manage = "Manage subscription"
    public var plusMonthly = "Get Plus: $15 a month"
    public var plusYearly = "Get Plus: $144 a year"
    public var plusNote = "Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies."
    public var cloudMonthly = "Upgrade to Cloud: $5 a month"
    public var cloudYearly = "Upgrade to Cloud: $50 a year"
    public var cloudNote = "Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server."
    public var provider = "Use Whimbrel AI as your provider"
    public var aiSettings = "AI settings"
    public init() {}
}

public enum WSettingsPaneDefaults {
    @Sendable public static func optionsLabel(_ name: String) -> String { "\(name) options" }
    @Sendable public static func deleteQuestion(_ name: String) -> String { "Delete \u{201C}\(name)\u{201D}? Its chat and earlier versions go too." }
}

/// Accessibility identifiers the Settings panes keep from the classic panes.
public enum WSettingsIDs {
    public static let appearancePicker = "appearance-picker"
    public static let profileEnabled = "profile-enabled"
    public static let profileSuggestEnabled = "profile-suggest-enabled"
    public static let profileAbout = "profile-about"
    public static let profilePreview = "profile-preview"
}
