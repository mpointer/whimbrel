import Foundation

/// The small model behind Ask with nothing configured. Core runs the helper; this finds the
/// files and tells core when they're there. See
/// docs/superpowers/specs/2026-11-01-ask-with-a-local-model-design.md.
enum LocalModel {
    private static let helperName = "whimbrel-llama-server"
    static let minimumMemory: UInt64 = 8 << 30
    static let minimumFreeDisk: Int64 = 4_000_000_000

    static var modelPath: URL { SpikePaths.root.appendingPathComponent("models/SmolLM3-Q4_K_M.gguf") }
    static var helperPath: URL? { Bundle.main.url(forResource: helperName, withExtension: nil) }
    static var isInstalled: Bool { FileManager.default.fileExists(atPath: modelPath.path) && helperPath != nil }

    /// With no Ollama to reach, an Ollama route streams from the bundled model instead.
    @MainActor
    static func replacesOllama(_ routing: RoutingSettings) -> Bool {
        isInstalled && routing.ollamaModels.isEmpty
    }

    static func canOffer(helperBundled: Bool, memory: UInt64, freeDisk: Int64) -> Bool {
        helperBundled && memory >= minimumMemory && freeDisk >= minimumFreeDisk
    }

    static var canOffer: Bool {
        let values = try? SpikePaths.root.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return canOffer(helperBundled: helperPath != nil, memory: ProcessInfo.processInfo.physicalMemory,
                        freeDisk: values?.volumeAvailableCapacityForImportantUsage ?? 0)
    }

    /// Run at launch and after a download. Clears a helper left behind by a crash first.
    static func configure() {
        let sweep = Process()
        sweep.executableURL = URL(fileURLWithPath: "/usr/bin/pkill")
        // By arguments: pkill -x compares a name cut to 15 characters, shorter than this one.
        sweep.arguments = ["-f", "/\(helperName) --host"]
        try? sweep.run()
        CoreMemoryStore.configureLocalModel(helper: isInstalled ? helperPath : nil, model: modelPath)
    }

    static func remove() {
        CoreMemoryStore.configureLocalModel(helper: nil, model: modelPath)
        try? FileManager.default.removeItem(at: modelPath)
    }
}
