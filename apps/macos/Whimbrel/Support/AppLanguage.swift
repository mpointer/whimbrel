import Foundation

/// The app's UI language and the tag notes and summaries are written in (shared with iOS).
enum AppLanguage {
    /// Device-local, never synced: "" is "Same as app language".
    static let writeInKey = "app.outputLanguageOverride"
    static let writeInOptions = ["en-US", "en-GB", "en-CA", "en-AU", "es-US", "fr-CA", "fr-FR", "de-DE", "pt-BR"]
    /// Set by Settings → Transcription once multilingual transcription exists (Task 20).
    static let multilingualKey = "transcription.multilingual"

    private static let englishVariants: Set<String> = ["US", "GB", "CA", "AU"]

    /// The UI language with the region ("es" + "MX" → "es-MX"). English is en-US unless the
    /// region is one of the English variants, so an Italian OS (no Italian UI) gets en-US.
    static func uiTag(
        preferred: String? = Bundle.main.preferredLocalizations.first,
        region: String? = Locale.current.region?.identifier
    ) -> String {
        let lang = preferred ?? "en"
        if lang.contains("-") { return lang }
        if lang == "en" { return "en-" + (region.flatMap { englishVariants.contains($0) ? $0 : nil } ?? "US") }
        return region.map { "\(lang)-\($0)" } ?? lang
    }

    /// `writeIn` when set, else `uiTag`; "" for en-US with an English-only model, which keeps
    /// prompts byte-identical to before localization.
    static func outputTag(uiTag: String, writeIn: String, multilingual: Bool) -> String {
        let tag = writeIn.isEmpty ? uiTag : writeIn
        return tag == "en-US" && !multilingual ? "" : tag
    }

    static func currentOutputTag(defaults: UserDefaults = .standard) -> String {
        outputTag(uiTag: uiTag(), writeIn: defaults.string(forKey: writeInKey) ?? "", multilingual: multilingual(defaults: defaults))
    }

    /// Multilingual transcription ("I have meetings in other languages"), device-local. Unset,
    /// it is on when the UI language is not English.
    static func multilingual(defaults: UserDefaults = .standard, uiTag: String = AppLanguage.uiTag()) -> Bool {
        defaults.object(forKey: multilingualKey) as? Bool ?? !uiTag.hasPrefix("en")
    }

    /// The app's own language override (nil is Automatic); takes effect on relaunch.
    static var override: String? {
        guard let id = Bundle.main.bundleIdentifier else { return nil }
        return (UserDefaults.standard.persistentDomain(forName: id)?["AppleLanguages"] as? [String])?.first
    }

    static func setOverride(_ tag: String?) {
        if let tag { UserDefaults.standard.set([tag], forKey: "AppleLanguages") } else { UserDefaults.standard.removeObject(forKey: "AppleLanguages") }
    }

    /// The UI languages this build ships, for the Language picker.
    static var shipped: [String] { Bundle.main.localizations.filter { $0 != "Base" && $0 != "en-XA" }.sorted() }
}
