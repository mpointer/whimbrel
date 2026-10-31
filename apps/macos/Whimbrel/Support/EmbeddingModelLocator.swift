import Foundation

/// The semantic-search embedding model's folder. The store's `models_dir` is
/// `<data folder>/models`; core downloads the model at runtime into its own subfolder there
/// (`whimbrel_embedding_model_dir`), so a newer model never reuses an older one's files.
enum EmbeddingModelLocator {
    static var modelsDirectory: URL { SpikePaths.root.appendingPathComponent("models", isDirectory: true) }

    /// The store's models directory, created if missing.
    static func modelsDirectoryPath() -> String? {
        try? FileManager.default.createDirectory(at: modelsDirectory, withIntermediateDirectories: true)
        return modelsDirectory.path
    }

    /// Whether the current model's weights are on disk.
    static var modelPresent: Bool {
        guard let dir = CoreMemoryStore.embeddingModelDir(modelsDir: modelsDirectory) else { return false }
        return FileManager.default.fileExists(atPath: dir.appendingPathComponent("model.safetensors").path)
    }
}
