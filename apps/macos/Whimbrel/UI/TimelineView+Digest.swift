import AppKit
import SwiftUI
import os

/// Library's Digest mode.
extension TimelineView {
    @ViewBuilder
    var digestModeBody: some View {
        if isGeneratingDigest {
            ProgressView("Generating digest with \(currentModelLabel)…")
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 40)
        } else if let digestError {
            Text(digestError)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelError)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 40)
        } else if let digest {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.md) {
                    Text(digest.summary)
                        .font(.whimbrelBody(.callout))
                        .textSelection(.enabled)
                    if !digest.openLoops.isEmpty {
                        Divider()
                        Text("Open loops").font(.whimbrelBody(.caption).bold())
                        ForEach(digest.openLoops, id: \.self) { loop in
                            Text("• \(loop)").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            Text("Generates an end-of-day digest of what you worked on today and any open loops, from today's captured timeline, written \(answeringProviderPhrase).")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 40)
        }
    }

    func generateDigestIfNeeded() {
        guard let store = appState.slowStore, !isBusy else { return }
        digest = nil
        digestError = nil
        let calendar = Calendar.current
        let startOfToday = calendar.startOfDay(for: Date())
        let startOfTomorrow = calendar.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday
        let startMs = Int64(startOfToday.timeIntervalSince1970 * 1000)
        let endMs = Int64(startOfTomorrow.timeIntervalSince1970 * 1000)

        isGeneratingDigest = true
        let capturedOllamaModel = ollamaModel
        let routing = appState.routing.routingConfig(
            preferredProvider: selectedProvider, preferredModel: currentModelLabel,
            dailyDigestInstructions: appState.promptLibrary.instructions[.dailyDigest] ?? ""
        )
        CoreMemoryStore.slowLaneQueue.async {
            let outcome = store.dailyDigest(model: capturedOllamaModel, startUnixMs: startMs, endUnixMs: endMs, routing: routing)
            DispatchQueue.main.async {
                isGeneratingDigest = false
                switch outcome {
                case .some(.success(let result)):
                    digest = result
                case .some(.failure(let error)):
                    digestError = error.userFacingMessage
                case .none:
                    digestError = "Couldn't generate the digest. Try again."
                }
            }
        }
    }
}
