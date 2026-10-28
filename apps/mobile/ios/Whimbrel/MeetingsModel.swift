import Foundation

struct MeetingDaySection: Identifiable {
    let day: Date
    let meetings: [PhoneMeeting]
    var id: Date { day }
}

/// The Meetings list: the newest grouped meetings, filtered by device and title.
@MainActor
final class MeetingsModel: ObservableObject {
    /// As the Mac's Meetings list: the 200 newest grouped meetings.
    static let limit = 200

    @Published private(set) var meetings: [PhoneMeeting] = []
    @Published private(set) var loaded = false
    @Published var deviceFilter: String?
    @Published var query = ""

    var isCapped: Bool { meetings.count >= Self.limit }

    /// Device ids in order of first appearance (newest meeting first).
    var devices: [String] {
        var seen = Set<String>()
        return meetings.map(\.deviceId).filter { seen.insert($0).inserted }
    }

    func reload(sync: SyncModel) async {
        meetings = await sync.read { $0.recentMeetings(limit: Self.limit) } ?? []
        loaded = true
    }

    /// Meetings grouped by local day, newest day first; `deviceFilter` keeps one device,
    /// `query` keeps titles containing it (case-insensitive; blank means everything).
    static func sections(_ meetings: [PhoneMeeting], deviceFilter: String?, query: String, calendar: Calendar) -> [MeetingDaySection] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let kept = meetings.filter { meeting in
            (deviceFilter == nil || meeting.deviceId == deviceFilter)
                && (needle.isEmpty || meeting.title.localizedCaseInsensitiveContains(needle))
        }
        let grouped = Dictionary(grouping: kept) { calendar.startOfDay(for: Date(timeIntervalSince1970: Double($0.startUnixMs) / 1000)) }
        return grouped.keys.sorted(by: >).map { day in
            MeetingDaySection(day: day, meetings: grouped[day]!.sorted { $0.startUnixMs > $1.startUnixMs })
        }
    }
}

/// One meeting's detail: the meeting, its action items and its transcript in pages.
@MainActor
final class MeetingDetailModel: ObservableObject {
    static let pageSize = 100

    let meetingId: Int64
    @Published private(set) var meeting: PhoneMeeting?
    @Published private(set) var actionItems: [PhoneActionItem] = []
    @Published private(set) var transcript: [PhoneTranscriptSegment] = []
    @Published private(set) var transcriptTotal = 0
    @Published private(set) var loaded = false

    var hasMoreTranscript: Bool { transcript.count < transcriptTotal }

    init(meetingId: Int64) { self.meetingId = meetingId }

    func reload(sync: SyncModel) async {
        let id = meetingId
        // A refresh re-reads as many segments as are already on screen, so a pull never
        // jumps the reader back to page 1.
        let count = max(transcript.count, Self.pageSize)
        let snapshot = await sync.read { store in
            (store.meeting(id: id), store.actionItems(meetingId: id), store.transcriptPage(meetingId: id, offset: 0, limit: count))
        }
        // No snapshot means the store is not open yet: keep showing "Loading…" until it is.
        guard let snapshot else { return }
        meeting = snapshot.0
        actionItems = snapshot.1
        transcript = snapshot.2?.segments ?? []
        transcriptTotal = snapshot.2?.total ?? 0
        loaded = true
    }

    func loadMoreTranscript(sync: SyncModel) async {
        guard hasMoreTranscript else { return }
        let id = meetingId
        let offset = transcript.count
        guard let page = await sync.read({ $0.transcriptPage(meetingId: id, offset: offset, limit: Self.pageSize) }) ?? nil else { return }
        // A refresh may have re-read these segments meanwhile: append only if still next in line.
        guard transcript.count == offset else { return }
        transcript += page.segments
        transcriptTotal = page.total
    }
}
