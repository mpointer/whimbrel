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
        List {
            ForEach(model.runs) { run in
                NavigationLink { RoutineRunDetailView(run: run, labels: labels) } label: { RoutineRunRow(run: run, labels: labels) }
                    .listRowBackground(Color.theme(.backgroundPage))
            }
            if model.runs.isEmpty {
                Text(model.loaded ? "No routine runs yet" : "Loading…")
                    .foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundPage))
            }
        }
        .themed()
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Routines")
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }
}

private struct RoutineRunRow: View {
    let run: PhoneRoutineRun
    let labels: DeviceLabels

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: run.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(run.succeeded ? Color.theme(.success) : Color.theme(.error))
                .accessibilityLabel(run.succeeded ? "Succeeded" : "Failed")
            VStack(alignment: .leading, spacing: 2) {
                Text(run.routineName).font(.headline).foregroundStyle(Color.theme(.textPrimary))
                Text(run.outputSummary)
                    .font(.subheadline).foregroundStyle(Color.theme(.textSecondary)).lineLimit(2)
                Text(caption).font(.caption).foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .frame(minHeight: 44, alignment: .leading)
    }

    private var caption: String {
        var parts = [Date(timeIntervalSince1970: Double(run.ranAtUnixMs) / 1000).formatted(.relative(presentation: .named))]
        if let origin = run.originDeviceId { parts.append(labels.label(for: origin)) }
        return parts.joined(separator: " · ")
    }
}

struct RoutineRunDetailView: View {
    let run: PhoneRoutineRun
    let labels: DeviceLabels

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(run.routineName).font(.system(.title2, design: .serif)).foregroundStyle(Color.theme(.textPrimary))
                Text(subtitle).font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
                Text(run.outputSummary)
                    .foregroundStyle(run.succeeded ? Color.theme(.textPrimary) : Color.theme(.error))
                    .textSelection(.enabled)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
    }

    private var subtitle: String {
        var parts = [Date(timeIntervalSince1970: Double(run.ranAtUnixMs) / 1000).formatted(.dateTime.weekday(.wide).month(.abbreviated).day().hour().minute())]
        if let origin = run.originDeviceId { parts.append("Ran on \(labels.label(for: origin))") }
        if !run.succeeded { parts.append("Failed") }
        return parts.joined(separator: " · ")
    }
}
