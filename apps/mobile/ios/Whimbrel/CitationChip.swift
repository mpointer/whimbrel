import SwiftUI

/// Where a citation chip leads.
enum CitationTarget: Hashable {
    case meeting(id: Int64)
    case capture(id: Int64)
    case routineRun(id: Int64)
    case none

    /// Transcript and snapshot cards open their meeting. Cards the replica doesn't have, and
    /// kinds with no screen on the phone (episodes, threads), go nowhere.
    static func `for`(_ card: CitationCard) -> CitationTarget {
        guard card.found else { return .none }
        switch card.kind {
        case "transcript", "snapshot": return card.meetingId.map { .meeting(id: $0) } ?? .none
        case "capture": return .capture(id: card.id)
        case "routine_run": return .routineRun(id: card.id)
        default: return .none
        }
    }
}

enum CitationChipLabel {
    /// "<subtitle> · <device name> · <title> · <short date>", leaving out what is empty
    /// ("Screen · MacBook · Pricing.xlsx"). A card that wasn't found keeps its kind ("Screen · Not on this iPhone").
    static func text(for card: CitationCard, deviceName: String?, now: Date = Date(), calendar: Calendar = .current, locale: Locale = .current) -> String {
        guard card.found else { return [card.subtitle, "Not on this iPhone"].filter { !$0.isEmpty }.joined(separator: " · ") }
        var parts = [card.subtitle, deviceName ?? "", card.title]
        if let ms = card.tsUnixMs { parts.append(shortDate(Date(timeIntervalSince1970: Double(ms) / 1000), now: now, calendar: calendar, locale: locale)) }
        return parts.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The weekday within the last week ("Tue"), else month and day ("Oct 7").
    static func shortDate(_ date: Date, now: Date, calendar: Calendar, locale: Locale) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = locale
        let recent = abs(now.timeIntervalSince(date)) < 7 * 86_400
        formatter.setLocalizedDateFormatFromTemplate(recent ? "EEE" : "MMMd")
        return formatter.string(from: date)
    }
}

/// One citation under an answer. Meeting and routine-run chips push (the host stack handles
/// `CitationTarget`); capture chips open a sheet; the rest are plain text.
struct CitationChip: View {
    let card: CitationCard
    let labels: DeviceLabels
    let openCapture: (Int64) -> Void

    var body: some View {
        let target = CitationTarget.for(card)
        let text = CitationChipLabel.text(for: card, deviceName: card.deviceId.map(labels.label(for:)))
        switch target {
        case .none:
            chip(text).accessibilityLabel(text)
        case .capture(let id):
            Button { openCapture(id) } label: { chip(text) }
                .buttonStyle(.plain)
                .accessibilityLabel(text)
                .accessibilityHint("Shows the captured text")
        case .meeting, .routineRun:
            NavigationLink(value: target) { chip(text) }
                .buttonStyle(.plain)
                .accessibilityLabel(text)
                .accessibilityHint("Opens the source")
        }
    }

    private func chip(_ text: String) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(Color.theme(card.found ? .textPrimary : .textSecondary))
            .multilineTextAlignment(.leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(minHeight: 44, alignment: .leading)
            .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.theme(.border), lineWidth: 1))
    }
}

/// A capture's text, in a sheet (the replica has no other screen for one).
struct CaptureSheet: View {
    let id: Int64
    @ObservedObject var sync: SyncModel
    @Environment(\.dismiss) private var dismiss
    @State private var item: PhoneCaptureItem?
    @State private var loaded = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let item {
                        Text(item.title).font(.system(.title3, design: .serif)).foregroundStyle(Color.theme(.textPrimary))
                        if let url = item.url, !url.isEmpty {
                            Text(url).font(.footnote).foregroundStyle(Color.theme(.textSecondary))
                        }
                        Text(item.text).foregroundStyle(Color.theme(.textPrimary)).textSelection(.enabled)
                    } else {
                        Text(loaded ? "This capture isn't on this iPhone." : "Loading…").foregroundStyle(Color.theme(.textSecondary))
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Color.theme(.backgroundPage).ignoresSafeArea())
            .navigationTitle("Screen capture")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .themed()
        .task {
            item = await sync.read { ($0 as? PhoneCaptureStore)?.captureItem(id: id) } ?? nil
            loaded = true
        }
    }
}

/// A routine run found by id (the replica has no getter; the newest runs include any cited).
struct RoutineRunLoader: View {
    let id: Int64
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @State private var run: PhoneRoutineRun?
    @State private var loaded = false

    var body: some View {
        Group {
            if let run {
                RoutineRunDetailView(run: run, labels: labels)
            } else {
                Text(loaded ? "This run isn't on this iPhone." : "Loading…")
                    .foregroundStyle(Color.theme(.textSecondary))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color.theme(.backgroundPage).ignoresSafeArea())
                    .navigationBarTitleDisplayMode(.inline)
            }
        }
        .themed()
        .task {
            run = await sync.read { $0.routineRuns(limit: 1000).first { $0.id == id } } ?? nil
            loaded = true
        }
    }
}
