import SwiftUI

/// Routine runs from every device, newest first; read-only.
@MainActor
final class RoutinesModel: ObservableObject {
    static let limit = 100

    @Published private(set) var runs: [PhoneRoutineRun] = []
    @Published private(set) var loaded = false

    func reload(sync: SyncModel) async {
        let all = await sync.read { $0.routineRuns(limit: Self.limit) } ?? []
        runs = all.sorted { $0.ranAtUnixMs > $1.ranAtUnixMs }
        loaded = true
    }
}

struct RoutinesView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model = RoutinesModel()

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: WTokens.Space.sm) {
                ForEach(model.runs) { run in
                    WSpineStop(time: ThreadTime.pastLabel(run.ranAtUnixMs), node: .past) {
                        NavigationLink { RoutineRunDetailView(run: run, labels: labels) } label: { RoutineRunCard(run: run, labels: labels) }
                            .buttonStyle(.plain)
                    }
                }
                if model.runs.isEmpty {
                    WStateView(kind: model.loaded ? .empty : .loading, title: model.loaded ? String(localized: "No routine runs yet") : String(localized: "Loading…"))
                }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.bottom, WTokens.Space.xl)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .environment(\.wStrings, .app)
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Routines")
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }
}

/// One run on the thread: whether it worked (a mark, and the word when it failed), the routine,
/// the first lines of what it produced, and the device it ran on.
private struct RoutineRunCard: View {
    let run: PhoneRoutineRun
    let labels: DeviceLabels

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
            HStack(alignment: .firstTextBaseline, spacing: WTokens.Space.sm) {
                Image(systemName: run.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .foregroundStyle(run.succeeded ? Color.theme(.success) : Color.theme(.error))
                    .accessibilityLabel(run.succeeded ? "Succeeded" : "Failed")
                Text(run.routineName).wType(WTokens.TypeScale.subtitle).foregroundStyle(Color.theme(.textPrimary))
            }
            if !run.succeeded {
                Text("Failed").wType(WTokens.TypeScale.caption).fontWeight(.semibold).foregroundStyle(Color.theme(.error))
                    .accessibilityHidden(true)
            }
            Text(run.outputSummary)
                .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary)).lineLimit(2)
                .multilineTextAlignment(.leading)
            if let origin = run.originDeviceId {
                Text(labels.label(for: origin)).wType(WTokens.TypeScale.caption).foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .padding(WTokens.Space.md)
        .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
        .wCard()
        .accessibilityElement(children: .combine)
    }
}

struct RoutineRunDetailView: View {
    let run: PhoneRoutineRun
    let labels: DeviceLabels

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WTokens.Space.md) {
                Text(run.routineName).wReading(WTokens.TypeScale.readingTitle).foregroundStyle(Color.theme(.textPrimary))
                    .accessibilityAddTraits(.isHeader)
                Text(subtitle).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                Text(run.outputSummary)
                    .wReading(WTokens.TypeScale.reading)
                    .foregroundStyle(run.succeeded ? Color.theme(.textPrimary) : Color.theme(.error))
                    .textSelection(.enabled)
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.md)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
    }

    private var subtitle: String {
        var parts = [Date(timeIntervalSince1970: Double(run.ranAtUnixMs) / 1000).formatted(.dateTime.weekday(.wide).month(.abbreviated).day().hour().minute())]
        if let origin = run.originDeviceId { parts.append(String(localized: "Ran on \(labels.label(for: origin))")) }
        if !run.succeeded { parts.append(String(localized: "Failed")) }
        return parts.joined(separator: " · ")
    }
}
