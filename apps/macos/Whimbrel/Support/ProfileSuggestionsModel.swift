import Foundation

/// The pending profile suggestions and the two things a user can do with one. Store access is
/// injected so the logic is testable; production wiring hops onto `CoreMemoryStore.accessQueue`.
@MainActor
final class ProfileSuggestionsModel: ObservableObject {
    @Published private(set) var pending: [CoreProfileSuggestion] = []

    private let profile: ProfileSettings
    // Store access runs off the main thread: the store queue can be busy for minutes, and
    // Settings reloads this on appear (including when a skin change rebuilds every pane).
    private let list: @Sendable () async -> [CoreProfileSuggestion]
    private let dismissRow: @Sendable (Int64) async -> Bool
    private let deleteRow: @Sendable (Int64) async -> Bool

    init(
        profile: ProfileSettings,
        list: @escaping @Sendable () async -> [CoreProfileSuggestion],
        dismiss: @escaping @Sendable (Int64) async -> Bool,
        delete: @escaping @Sendable (Int64) async -> Bool
    ) {
        self.profile = profile
        self.list = list
        self.dismissRow = dismiss
        self.deleteRow = delete
    }

    func reload() async { pending = await list() }

    /// Adds the suggestion to the profile as a fact. Either way the row is cleared: one that cannot
    /// be added (a duplicate, or the list is full) would otherwise stay pending forever.
    @discardableResult
    func add(_ suggestion: CoreProfileSuggestion) async -> Bool {
        let added = profile.addFact(suggestion.text, sourceMeetingId: suggestion.sourceMeetingId)
        _ = await deleteRow(suggestion.id)
        await reload()
        return added
    }

    func dismiss(_ suggestion: CoreProfileSuggestion) async {
        _ = await dismissRow(suggestion.id)
        await reload()
    }
}
