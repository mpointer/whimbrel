import Foundation

/// Drives one streaming Ask run end to end (docs/adr/0002 §6, Wireframe C). Owns the
/// run-id guard that discards late events after cancellation or a fresh retry: every
/// applied event's `runId` must match `activeRunId`, so a stray event from a prior,
/// already-cancelled run can never render into the current response — this is the
/// mechanism INF-108/UI-104 require ("switch model / retry after cancel must not show
/// contaminated output").
///
/// Polling runs on a plain background `Task`, never `CoreMemoryStore.accessQueue` — this
/// view model never touches `StoreHandle`.
@MainActor
final class StreamingAskViewModel: ObservableObject {
    enum Phase: Equatable {
        case idle
        case connecting
        case streaming
        case cancelling
        case completed
        case cancelled
        case failed(message: String)
    }

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var streamedText: String = ""
    /// Token counts from the run's terminal event (nil fields until it arrives, and for
    /// providers that report none). Read by `TimelineView.finalizeStreamingAnswer`.
    @Published private(set) var usage = InferenceEvent.TokenUsage()

    private var activeRunId: String?
    private var pollTask: Task<Void, Never>?

    var isCancellable: Bool {
        switch phase {
        case .connecting, .streaming, .cancelling: return true
        default: return false
        }
    }

    /// Starts a new run, first discarding (releasing) any still-registered prior run —
    /// matches UI-104: switching/retrying starts a run with a distinct id, and old
    /// events can never be applied under the new run's display state.
    func start(prompt: String, model: String, provider: InferenceStreamClient.Provider = .ollama, apiKey: String? = nil, baseURLOverride: String? = nil) {
        releaseCurrentRun()
        streamedText = ""
        usage = InferenceEvent.TokenUsage()
        phase = .connecting
        guard let runId = InferenceStreamClient.start(prompt: prompt, model: model, provider: provider, apiKey: apiKey, baseURLOverride: baseURLOverride) else {
            phase = .failed(message: "Could not start — invalid prompt, model, or missing API key.")
            return
        }
        activeRunId = runId
        pollTask = Task { [weak self] in
            await self?.pollLoop(runId: runId)
        }
    }

    func cancel() {
        guard let activeRunId, isCancellable else { return }
        phase = .cancelling
        InferenceStreamClient.cancel(runId: activeRunId)
    }

    private func pollLoop(runId: String) async {
        while !Task.isCancelled {
            let events = InferenceStreamClient.poll(runId: runId)
            var sawTerminal = false
            for event in events where event.runId == runId && runId == activeRunId {
                apply(event)
                if case .terminal = event { sawTerminal = true }
            }
            if sawTerminal {
                InferenceStreamClient.release(runId: runId)
                if activeRunId == runId { activeRunId = nil }
                return
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    /// Internal so tests can feed events without a provider.
    func apply(_ event: InferenceEvent) {
        switch event {
        case .state(_, let state, _, _, _, _):
            switch state {
            case .queued, .connecting: phase = .connecting
            case .streaming: phase = .streaming
            case .cancelling: phase = .cancelling
            }
        case .textDelta(_, _, let text):
            // Cancellation is authoritative the instant it's requested (I-06):
            // never render a delta once the user has asked to stop, even if it
            // was already in flight when cancel() was called.
            guard phase != .cancelling else { return }
            streamedText += text
        case .progress:
            break
        case .terminal(_, let outcome, _, let finalText, let reportedUsage):
            usage = reportedUsage
            switch outcome {
            case .completed:
                streamedText = finalText
                phase = .completed
            case .cancelled:
                phase = .cancelled
            case .failed(_, let message):
                phase = .failed(message: message)
            }
        }
    }

    private func releaseCurrentRun() {
        pollTask?.cancel()
        pollTask = nil
        if let activeRunId {
            InferenceStreamClient.cancel(runId: activeRunId)
            InferenceStreamClient.release(runId: activeRunId)
        }
        activeRunId = nil
    }

    deinit {
        pollTask?.cancel()
    }
}
