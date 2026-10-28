import SwiftUI

/// Meetings grouped by day, with a device filter and a title search.
struct MeetingsView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model = MeetingsModel()

    var body: some View {
        let sections = MeetingsModel.sections(model.meetings, deviceFilter: model.deviceFilter, query: model.query, calendar: .current)
        List {
            if model.devices.count > 1 { deviceChips }
            ForEach(sections) { section in
                Section {
                    ForEach(section.meetings) { meeting in
                        NavigationLink(value: meeting.id) { MeetingRow(meeting: meeting, labels: labels) }
                            .listRowBackground(Color.theme(.backgroundPage))
                    }
                } header: {
                    Text(dayTitle(section.day)).foregroundStyle(Color.theme(.textSecondary))
                }
            }
            if sections.isEmpty {
                Text(emptyText).foregroundStyle(Color.theme(.textSecondary)).listRowBackground(Color.theme(.backgroundPage))
            }
            if model.isCapped {
                Text("Showing the newest \(MeetingsModel.limit) meetings.")
                    .font(.footnote).foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundPage))
            }
        }
        .themed()
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Meetings")
        .searchable(text: $model.query, prompt: "Search titles")
        .navigationDestination(for: Int64.self) { MeetingDetailView(meetingId: $0, sync: sync, labels: labels) }
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }

    private var emptyText: String {
        if !model.loaded { return "Loading…" }
        return model.meetings.isEmpty ? "No meetings yet" : "No meetings match"
    }

    private var deviceChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All", selected: model.deviceFilter == nil) { model.deviceFilter = nil }
                ForEach(model.devices, id: \.self) { device in
                    chip(labels.label(for: device), selected: model.deviceFilter == device) { model.deviceFilter = device }
                }
            }
        }
        .listRowBackground(Color.theme(.backgroundPage))
        .listRowSeparator(.hidden)
    }

    private func chip(_ text: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(text)
                .font(.subheadline.weight(.medium))
                .padding(.horizontal, 14)
                .frame(minHeight: 44)
                .foregroundStyle(selected ? Color.theme(.textOnAccent) : Color.theme(.textPrimary))
                .background(selected ? Color.theme(.accentPrimary) : Color.theme(.backgroundCard), in: Capsule())
                .overlay(Capsule().stroke(Color.theme(.border), lineWidth: selected ? 0 : 1))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func dayTitle(_ day: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(day) { return "Today" }
        if calendar.isDateInYesterday(day) { return "Yesterday" }
        if calendar.isDate(day, equalTo: Date(), toGranularity: .year) {
            return day.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
        }
        return day.formatted(.dateTime.weekday(.wide).month(.abbreviated).day().year())
    }
}
