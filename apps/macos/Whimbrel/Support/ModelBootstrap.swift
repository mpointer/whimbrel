import Combine
import Foundation

/// The one first-run screen (doc 17, item 1.1). Never for a Mac that already granted either
/// permission: that is an existing install.
enum FirstRun {
    static let doneKey = "onboarding.firstRunDone"

    static func shouldShow(done: Bool, microphoneGranted: Bool, screenGranted: Bool) -> Bool {
        !done && !microphoneGranted && !screenGranted
    }
}

/// Downloads missing models in the background at launch, with no prompt: Whimbrel is usable
/// meanwhile, and keyword search needs no model. A failure waits for "Try again" or the next launch.
@MainActor
final class ModelBootstrap: ObservableObject {
    enum Phase: Equatable {
        case idle, downloading(fraction: Double, status: String), failed, ready
    }

    typealias Download = (@escaping (Double, String) -> Void) async throws -> Void

    @Published private(set) var phase: Phase = .idle
    private let present: () -> Bool
    private let download: Download

    init(present: @escaping () -> Bool = { ModelDownloadService.allModelsPresent },
         download: @escaping Download = { try await ModelDownloadService.downloadAll(progress: $0) }) {
        self.present = present
        self.download = download
    }

    func start() async {
        if case .downloading = phase { return }
        guard !present() else {
            phase = .ready
            return
        }
        phase = .downloading(fraction: 0, status: "")
        do {
            try await download { [weak self] fraction, status in
                self?.phase = .downloading(fraction: fraction, status: status)
            }
            phase = .ready
        } catch {
            phase = .failed
        }
    }
}
