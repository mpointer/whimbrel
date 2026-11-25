import AppKit
import Combine
import Foundation

enum AppearanceChoice: String, CaseIterable, Identifiable {
    case system, light, dark

    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    /// nil follows the system.
    var nsAppearance: NSAppearance? {
        switch self {
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        }
    }
}

/// System / Light / Dark, per device (UserDefaults, not synced). It sets `NSApp.appearance`,
/// which repaints every window. A one-variant palette overrides the choice (see `appearanceName`).
final class AppearanceSettings: ObservableObject {
    static let key = "com.mpointer.kestrel.appearance"
    private let defaults: UserDefaults
    private let themes: ThemeStore
    private var paletteChange: AnyCancellable?
    private var contrastChange: AnyCancellable?

    @Published var choice: AppearanceChoice {
        didSet {
            defaults.set(choice.rawValue, forKey: Self.key)
            apply()
        }
    }

    init(defaults: UserDefaults = .standard, themes: ThemeStore = .shared) {
        self.defaults = defaults
        self.themes = themes
        choice = AppearanceChoice(rawValue: defaults.string(forKey: Self.key) ?? "") ?? .system
        // `revision` publishes before it changes, but the selection it announces is already in place.
        paletteChange = themes.$revision.dropFirst().sink { [weak self] _ in self?.apply() }
        // A pinned Light or Dark has to follow the system's Increase Contrast setting while the app runs.
        contrastChange = NSWorkspace.shared.notificationCenter
            .publisher(for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification)
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.apply() }
    }

    /// Re-applied whenever the palette changes, since a one-variant palette overrides `choice`.
    func apply() {
        let pinned = Self.appearanceName(choice: choice, variants: themes.selected.variants)
        let name = Self.contrastName(pinned, increased: NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast)
        NSApp?.appearance = name.flatMap { NSAppearance(named: $0) }
    }

    /// A pinned Light or Dark as its high contrast twin while the system asks for more contrast. A plain pinned appearance
    /// would hide the system setting from every window, and the high contrast colors would never be drawn. nil (follow the
    /// system) stays nil: the system's own appearance already carries the setting.
    static func contrastName(_ name: NSAppearance.Name?, increased: Bool) -> NSAppearance.Name? {
        guard increased, let name else { return name }
        switch name {
        case .aqua: return .accessibilityHighContrastAqua
        case .darkAqua: return .accessibilityHighContrastDarkAqua
        default: return name
        }
    }

    /// A dark-only palette forces Dark and a light-only one Light, so system-drawn text (labels,
    /// placeholders, pickers, alerts) matches it; two-variant palettes follow `choice`. nil follows the system.
    static func appearanceName(choice: AppearanceChoice, variants: ThemeVariants) -> NSAppearance.Name? {
        switch variants {
        case .darkOnly: return .darkAqua
        case .lightOnly: return .aqua
        case .both: return choice.nsAppearance?.name
        }
    }

    /// Shown under the picker when the palette overrides it, so the picker doesn't look broken. nil = picker is live.
    static func lockedCaption(variants: ThemeVariants) -> String? {
        switch variants {
        case .darkOnly: return String(localized: "This palette is dark only, so it always uses Dark.")
        case .lightOnly: return String(localized: "This palette is light only, so it always uses Light.")
        case .both: return nil
        }
    }

    var lockedCaption: String? { Self.lockedCaption(variants: themes.selected.variants) }
}
