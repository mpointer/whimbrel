import Foundation

/// "Sort items into topics with TypeSafe" (Settings > AI). Local to this Mac, like
/// `ReplyPrioritySettings`: the switch is in UserDefaults and the key stays in the Keychain.
/// Judging runs only after the user pressed Start on the estimate; core enforces the budget.
/// Also backs "Find steps with TypeSafe" (Settings > Topics) under its own keys.
final class TopicSortSettings: ObservableObject {
    private let enabledKey: String
    private let startedKey: String
    static let usdPerMillion = 0.042

    struct Estimate: Equatable, Decodable {
        let sources: Int
        let tokens: Int
        let usd: Double
        /// Core is still sorting items: no estimate or Start yet.
        let indexing: Bool
    }

    private let defaults: UserDefaults
    private let loadKey: () -> String?

    @Published var enabled: Bool {
        didSet {
            defaults.set(enabled, forKey: enabledKey)
            if !enabled { startFailed = false }
        }
    }
    @Published private(set) var keyRejected = false
    @Published private(set) var overBudget = false
    @Published private(set) var startFailed = false
    /// The last estimate request failed: offer "Try again".
    @Published private(set) var estimateFailed = false
    /// Shown inline ("About N items, about $X. Start?") until Start is pressed.
    @Published private(set) var estimate: Estimate?
    private(set) var started: Bool {
        get { defaults.bool(forKey: startedKey) }
        set { defaults.set(newValue, forKey: startedKey) }
    }

    init(
        enabledKey: String = "com.mpointer.kestrel.topicSortEnabled",
        startedKey: String = "com.mpointer.kestrel.topicSortStarted",
        defaults: UserDefaults = .standard,
        loadKey: @escaping () -> String? = { KeychainService.load(for: .typesafe) }
    ) {
        self.enabledKey = enabledKey
        self.startedKey = startedKey
        self.defaults = defaults
        self.loadKey = loadKey
        enabled = defaults.bool(forKey: enabledKey)
    }

    func markKeyRejected(_ rejected: Bool) {
        if keyRejected != rejected { keyRejected = rejected }
    }

    func setOverBudget(_ over: Bool) {
        if overBudget != over { overBudget = over }
    }

    /// A fetched estimate, or nil when the fetch failed.
    func setEstimate(_ e: Estimate?) {
        estimate = e
        estimateFailed = e == nil
        if e != nil { startFailed = false }
    }

    /// Whether the estimate row should be fetched and shown: switch on, Start not pressed.
    var awaitingStart: Bool { enabled && !started }

    func setStartFailed(_ failed: Bool) { startFailed = failed }

    func markStarted() {
        startFailed = false
        started = true
        estimate = nil
        overBudget = false
    }

    /// After the budget ran out, or core has no budget (database replaced): ask for a fresh
    /// Start.
    func requireStart() {
        started = false
        overBudget = false
    }

    /// Whether judging may run: switch on and Start pressed.
    var judgingAllowed: Bool { enabled && started }

    /// The key to judge with, or nil when the switch is off, Start was not pressed, there
    /// is no key, or Local-only mode is on.
    func activeKey(localOnly: Bool) -> String? {
        guard judgingAllowed, !localOnly else { return nil }
        return loadKey()
    }
}
