import Foundation

/// The words the settings parts draw themselves: button names, spoken states and the short messages for
/// bad input. Text that belongs to one setting (its label, its description, its unit) is passed to the part.
/// English defaults; the app supplies its own from the string catalog through `WStrings.settings`.
public struct WSettingsStrings: Sendable {
    // Navigation and search
    public var sectionsLabel = "Settings sections"
    public var searchResultsLabel = "Search results"
    public var noMatchingSettings = "No matching settings"
    /// "Language, in General": the spoken name of a search result.
    public var resultInSection: @Sendable (String, String) -> String = { "\($0), in \($1)" }

    // Tones, spoken before the text of a chip or a notice
    public var toneGood = "Good"
    public var toneWarning = "Warning"
    public var toneProblem = "Problem"
    public var noticeInfo = "Note"
    public var noticeWarning = "Warning"
    public var noticeProblem = "Problem"

    // Common buttons
    public var save = "Save"
    public var cancel = "Cancel"
    public var remove = "Remove"
    public var replace = "Replace"
    public var edit = "Edit"
    public var add = "Add"
    public var clear = "Clear"
    public var copy = "Copy"
    public var copied = "Copied"
    public var done = "Done"
    public var on = "On"
    public var off = "Off"

    // Rows
    public var checked = "Checked"
    public var unchecked = "Not checked"
    public var selected = "Selected"
    /// Said when a segmented control is read: "Appearance, Light, 1 of 3".
    public var choiceOf: @Sendable (String, Int, Int) -> String = { "\($0), \($1) of \($2)" }
    public var noOptions = "No choices yet"
    public var choose = "Choose"
    public var opensInBrowser = "Opens in your browser"
    public var opensSection = "Opens another section"
    public var expanded = "Expanded"
    public var collapsed = "Collapsed"
    /// "Disabled. <reason>": the spoken form of a row's unavailable reason.
    public var unavailable: @Sendable (String) -> String = { "Unavailable. \($0)" }

    // Secrets
    public var secretSaved = "Saved"
    /// The chip on a saved key the provider refused.
    public var secretRefused = "Saved, but refused"
    public var secretNotSet = "Not configured"
    public var secretHidden = "Saved, hidden"
    public var secretNewValue = "Paste the new value"
    public var secretFieldLabel: @Sendable (String) -> String = { "\($0), new value" }

    // Lists
    public var emptyList = "Nothing here yet."
    public var addFieldLabel: @Sendable (String) -> String = { "Add to \($0)" }
    public var editNamed: @Sendable (String) -> String = { "Edit \($0)" }
    public var removeNamed: @Sendable (String) -> String = { "Remove \($0)" }
    public var listBlank = "Enter something to add."
    public var listDuplicate = "That one is already in the list."
    public var listTooLong: @Sendable (Int) -> String = { "Use \($0) characters or fewer." }
    public var listFull: @Sendable (Int) -> String = { "The list is full (\($0))." }

    // Numbers
    public var numberNotANumber = "Enter a number."
    public var numberRequired = "Enter a number."
    public var numberBelow: @Sendable (String) -> String = { "Use \($0) or more." }
    public var numberAbove: @Sendable (String) -> String = { "Use \($0) or less." }
    public var numberDecimals: @Sendable (Int) -> String = { $0 == 0 ? "Use a whole number." : "Use at most \($0) decimal places." }
    /// The placeholder is an example in the person's own format, such as 1250.50.
    public var numberSeparator: @Sendable (String) -> String = { "Leave out thousands separators and use one decimal mark, like \($0)." }
    /// The line beside a stepper that is off because its checkbox is on; the placeholder is the checkbox's name.
    public var stepperOffBecause: @Sendable (String) -> String = { "Turn off \($0) to change the number." }
    public var increase = "Increase"
    public var decrease = "Decrease"

    // Shortcut
    public var record = "Record"
    public var pressAShortcut = "Press a shortcut\u{2026}"
    public var stopRecording = "Cancel"
    public var resetToDefault = "Reset to default"
    public var shortcutNotSet = "Not set"
    public var shortcutLabel: @Sendable (String) -> String = { "Shortcut, \($0)" }
    public var recordingAnnouncement = "Recording a shortcut. Press the keys, or Escape to cancel."

    // Values
    public var copyNamed: @Sendable (String) -> String = { "Copy \($0)" }

    // Progress
    public var progressPercent: @Sendable (Int) -> String = { "\($0) percent" }
    public var working = "Working"

    // Sheets and confirms
    /// "Type DELETE to confirm."
    public var typeToConfirm: @Sendable (String) -> String = { "Type \($0) to confirm." }
    public var confirmQuestionSuffix = "This cannot be undone."
    public var confirmAnnouncement: @Sendable (String) -> String = { "\($0) Confirm or cancel." }

    // Skin card
    public var yours = "Yours"
    public var paletteLabel = "Palette"
    public var skinOptions = "More options"
    public var skinSelected = "Selected skin"

    public init() {}

    public static let english = WSettingsStrings()
}
