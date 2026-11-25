import Foundation
import WhimbrelUI

/// The words the settings parts draw themselves, looked up in the app's string catalog. Only the words the redrawn
/// sections can show are localized; the rest keep the package's English until a later section needs them
/// (shortcuts, progress, typed confirmations).
enum SettingsStringsRedesign {
    static func make() -> WSettingsStrings {
        var s = WSettingsStrings()
        s.sectionsLabel = String(localized: "Settings sections", comment: "Spoken name of the list of sections in the Settings window.")
        s.searchResultsLabel = String(localized: "Search results", comment: "Spoken name of the list of results under the Settings search field.")
        s.noMatchingSettings = String(localized: "No matching settings")
        s.resultInSection = { title, section in
            String(localized: "\(title), in \(section)", comment: "Spoken name of a Settings search result. First placeholder is the setting, second is the section it is in, for example Language, in General.")
        }
        s.save = String(localized: "Save")
        s.cancel = String(localized: "Cancel")
        s.remove = String(localized: "Remove")
        s.edit = String(localized: "Edit")
        s.add = String(localized: "Add")
        s.on = String(localized: "On", comment: "Spoken state of a switch that is on.")
        s.off = String(localized: "Off")
        s.checked = String(localized: "Checked", comment: "Spoken state of a checkbox that is ticked.")
        s.unchecked = String(localized: "Not checked", comment: "Spoken state of a checkbox that is not ticked.")
        s.selected = String(localized: "Selected", comment: "Spoken state of the chosen item in a list or a row of choices.")
        s.choiceOf = { label, index, count in
            String(localized: "\(label), \(index) of \(count)", comment: "Spoken name of one choice in a row of choices. Placeholders: the setting, the number of this choice, how many there are. For example Appearance, 2 of 3.")
        }
        s.noOptions = String(localized: "No choices yet", comment: "Shown in place of a menu when the setting has nothing to choose from.")
        s.choose = String(localized: "Choose", comment: "Placeholder on a menu in Settings before anything is chosen.")
        s.unavailable = { reason in
            String(localized: "Unavailable. \(reason)", comment: "Spoken after a setting's name when it cannot be changed. The placeholder is the reason, for example This palette is dark only.")
        }
        s.emptyList = String(localized: "Nothing here yet.", comment: "Shown in a Settings list that has no items.")
        s.editNamed = { name in
            String(localized: "Edit \(name)", comment: "Spoken name of the Edit button on a list row. The placeholder is the item's text.")
        }
        s.removeNamed = { name in
            String(localized: "Remove \(name)", comment: "Spoken name of the Remove button on a list row. The placeholder is the item's text.")
        }
        s.yours = String(localized: "Yours")
        s.paletteLabel = String(localized: "Palette", comment: "Spoken name of the row of colour choices on a skin card.")
        s.skinOptions = String(localized: "More options")
        s.replace = String(localized: "Replace")
        s.secretSaved = String(localized: "Saved")
        s.secretRefused = String(localized: "Saved, but refused", comment: "Chip on an API key that is saved but that its provider refused when checked.")
        s.secretNotSet = String(localized: "Not configured", comment: "Placeholder in a secret field that has no value yet.")
        s.secretHidden = String(localized: "Saved, hidden", comment: "Spoken value of a secret that is saved and not shown.")
        s.secretFieldLabel = { String(localized: "\($0), new value", comment: "Spoken name of the field for a new secret. The placeholder is the secret's name.") }
        s.copy = String(localized: "Copy")
        s.copied = String(localized: "Copied")
        s.done = String(localized: "Done")
        s.expanded = String(localized: "Expanded")
        s.collapsed = String(localized: "Collapsed")
        s.opensInBrowser = String(localized: "Opens in your browser", comment: "Spoken hint of a link that opens a web page.")
        s.listBlank = String(localized: "Enter something to add.", comment: "Tooltip on a disabled Add button in Settings: the field is empty.")
        s.secretNewValue = String(localized: "Paste the new value", comment: "Placeholder and tooltip on a secret field in Settings that is empty.")
        s.stepperOffBecause = { String(localized: "Turn off \($0) to change the number.", comment: "Shown beside a number stepper in Settings that is off because its checkbox is on. The placeholder is the checkbox's name, for example Keep forever.") }
        s.numberNotANumber = String(localized: "Enter a number.", comment: "Shown under a number field in Settings when the text is not a number or the field cannot be empty.")
        s.numberRequired = String(localized: "Enter a number.", comment: "Shown under a number field in Settings when the text is not a number or the field cannot be empty.")
        s.numberBelow = { String(localized: "Use \($0) or more.", comment: "Shown under a number field in Settings. The placeholder is the smallest allowed number.") }
        s.numberAbove = { String(localized: "Use \($0) or less.", comment: "Shown under a number field in Settings. The placeholder is the largest allowed number.") }
        s.numberDecimals = { places in
            places == 0
                ? String(localized: "Use a whole number.", comment: "Shown under a number field in Settings that takes whole numbers only.")
                : String(localized: "Use at most \(places) decimal places.", comment: "Shown under a number field in Settings. The placeholder is the number of decimal places allowed.")
        }
        s.numberSeparator = { String(localized: "Leave out thousands separators and use one decimal mark, like \($0).", comment: "Shown under a number field in Settings when the text has a thousands separator or could be read two ways. The placeholder is an example number in the person's own format, such as 1250.50.") }
        s.skinSelected = String(localized: "Selected skin", comment: "Spoken state of the skin that is in use.")
        return s
    }
}
