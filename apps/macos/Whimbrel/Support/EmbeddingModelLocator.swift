import Foundation

/// Locates the embedding model's bundled resource directory inside the app (see
/// `apps/macos/project.yml`'s folder-reference resources and Task 1's README for
/// what's inside it: config.json, tokenizer.json, model.safetensors). Mirrors
/// `McpNodeLocator.calendarConnectorScriptPath()`'s exact pattern for the same reason:
/// confirmed empirically that a folder-reference resource lands directly under
/// `Contents/Resources/<source-folder-name>`, not the full relative source path.
enum EmbeddingModelLocator {
    /// Returns the bundled models directory's path, or nil if it genuinely isn't
    /// there (e.g. a fresh checkout that never ran Task 1's `curl` steps before a
    /// local Debug build) — so callers can pass nil through to `CoreMemoryStore`
    /// and get graceful degradation rather than a path that silently resolves to
    /// nothing useful.
    static func modelsDirectoryPath() -> String? {
        guard let url = Bundle.main.resourceURL?.appendingPathComponent("models"),
              FileManager.default.fileExists(atPath: url.path) else {
            return nil
        }
        return url.path
    }
}
