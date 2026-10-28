import Foundation

/// One of Whimbrel's LLM-call "surfaces" that can carry its own free-text tuning
/// instructions, independent of every other surface (#46). See
/// docs/superpowers/specs/2026-09-03-per-surface-prompt-library-design.md.
enum PromptSurface: String, CaseIterable, Identifiable {
    case ask
    case meetingSummary
    case dailyDigest
    case weeklyDigest
    case episodeSummary
    case threadSummary

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .ask: return "Ask"
        case .meetingSummary: return "Meeting Summary"
        case .dailyDigest: return "Daily Digest"
        case .weeklyDigest: return "Weekly Digest"
        case .episodeSummary: return "Episode Summary"
        case .threadSummary: return "Thread Summary"
        }
    }

    var helpText: String {
        switch self {
        case .ask:
            return "Optional. Folded into every Ask answer -- e.g. \"answer in bullet points\" or \"be terse.\" Leave blank for the default behavior."
        case .meetingSummary:
            return "Optional. Folded into every meeting summary -- e.g. \"group decisions by topic\" or \"be terse.\" Leave blank for the default behavior."
        case .dailyDigest:
            return "Optional. Folded into every daily digest, whether generated on demand or by the Daily Brief/EOD Recap routines. Leave blank for the default behavior."
        case .weeklyDigest:
            return "Optional. Folded into the Weekly Digest routine's summary. Leave blank for the default behavior."
        case .episodeSummary:
            return "Optional. Folded into every automatic episode summary. Leave blank for the default behavior."
        case .threadSummary:
            return "Optional. Folded into every email thread's rolling summary. Leave blank for the default behavior."
        }
    }
}

/// Per-surface prompt instructions (#46) -- supersedes `RoutingSettings.
/// customPromptInstructions` (FR-NOTES-1's global standing-rules field) in
/// practice: every real prompt call site now reads its own surface's value here
/// instead, and the legacy field's Settings UI is gone. The legacy field's own
/// storage/sync mechanism is deliberately left in place rather than deleted --
/// removing it outright is a separate cleanup, not required for correctness --
/// but its only remaining role is as this class's one-time migration source (see
/// `migrateLegacyInstructionsIfNeeded()`). One independently-synced UserDefaults
/// entry per `PromptSurface`, matching `RoutingSettings`'s own persistence
/// pattern exactly -- see
/// docs/superpowers/specs/2026-09-03-per-surface-prompt-library-design.md's
/// Migration & Deprecation section.
@MainActor
final class PromptLibrarySettings: ObservableObject {
    nonisolated private static func defaultsKey(for surface: PromptSurface) -> String {
        "com.mpointer.kestrel.promptInstructions.\(surface.rawValue)"
    }

    /// The `field_key` string used by `SettingsFieldTimestamps` and, once wired into
    /// `SettingsSyncCoordinator`, the Turso `settings_field.field_key` value. `nonisolated`
    /// since it's a pure function touching no actor-isolated state -- callable synchronously
    /// from any context, including plain (non-`@MainActor`) test helpers.
    nonisolated static func fieldKey(for surface: PromptSurface) -> String {
        "prompts.\(surface.rawValue)"
    }

    /// `didSet` compares against `oldValue` per-surface so editing one surface only
    /// persists and touches the sync timestamp for that surface -- not all six. This
    /// is the property this whole feature exists to guarantee (see this class's own
    /// design spec's Testing section).
    @Published var instructions: [PromptSurface: String] {
        didSet {
            for surface in PromptSurface.allCases where instructions[surface] != oldValue[surface] {
                UserDefaults.standard.set(instructions[surface] ?? "", forKey: Self.defaultsKey(for: surface))
                SettingsFieldTimestamps.touch(Self.fieldKey(for: surface))
            }
        }
    }

    /// The legacy global field's own UserDefaults key
    /// (`RoutingSettings.customPromptInstructionsKey`) -- read directly rather than
    /// taking a `RoutingSettings` dependency, since this migration only ever needs
    /// the raw stored string, once.
    private static let legacyCustomPromptInstructionsKey = "com.mpointer.kestrel.summaryCustomPromptInstructions"

    init() {
        var loaded: [PromptSurface: String] = [:]
        for surface in PromptSurface.allCases {
            loaded[surface] = UserDefaults.standard.string(forKey: Self.defaultsKey(for: surface)) ?? ""
        }
        instructions = loaded
        migrateLegacyInstructionsIfNeeded()
    }

    /// The four surfaces the legacy global field's value formally migrates into
    /// once, on first launch after this feature ships -- exactly the surfaces that
    /// field actually reached before this class existed
    /// (`docs/superpowers/specs/2026-08-17-custom-instructions-everywhere-design.md`
    /// widened it to "every chat/digest/summary prompt": Ask, Meeting Summary,
    /// Daily Digest, Weekly Digest). Episode Summary and Thread Summary never
    /// received the legacy value -- their instructions plumbing didn't exist
    /// before this feature (#46) -- so they're deliberately excluded here.
    private static let legacyMigrationTargets: [PromptSurface] = [.ask, .meetingSummary, .dailyDigest, .weeklyDigest]

    /// One-time seed, per surface: the legacy global field's value becomes each
    /// applicable surface's starting value. This formally supersedes FR-NOTES-1's
    /// global field in practice, per the design spec's Migration & Deprecation
    /// section -- no user's existing standing rule silently vanishes on upgrade.
    /// Each surface is guarded independently on its own `prompts.<surface>` never
    /// having synced locally, so a second launch -- or a surface the user has
    /// since edited or cleared -- is never re-clobbered, even if another surface
    /// hasn't migrated yet for whatever reason.
    private func migrateLegacyInstructionsIfNeeded() {
        let legacy = UserDefaults.standard.string(forKey: Self.legacyCustomPromptInstructionsKey) ?? ""
        guard !legacy.isEmpty else { return }
        for surface in Self.legacyMigrationTargets where SettingsFieldTimestamps.timestamp(for: Self.fieldKey(for: surface)) == 0 {
            instructions[surface] = legacy
        }
    }
}
