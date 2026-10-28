import SwiftUI
@testable import Whimbrel

/// `TodayContent` values for layout tests and snapshots: the shipped sample content plus stress cases.
enum TodayFixtures {
    private typealias S = TodaySampleContent

    static let now = S.now

    static let sample = S.sample

    /// Layout stress: a 120-character title, 40 long-named attendees, 200 recent meetings,
    /// a five-digit approval count and a long recap.
    static let worstCase = S.content(
        events: [
            S.event(
                "w1", String(repeating: "Quarterly cross-functional alignment ", count: 4).prefix(120).description, S.at(hour: 11),
                attendees: (1...40).map { "Bartholomew Featherstonehaugh-Wellington \($0)" }
            ),
            S.event("w2", "Short one", S.at(hour: 14)),
        ],
        meetings: (1...200).map { i in
            S.meeting(
                Int64(i), "Meeting number \(i) with an unreasonably long title that keeps going and going",
                S.at(dayOffset: -(i % 6), hour: 9 + i % 8), attendees: ["Person \(i)", "Other Person \(i)"]
            )
        },
        approvals: 12_345,
        threads: (1...5).map { S.thread(Int64($0), String(repeating: "A very long subject line about nothing ", count: 4)) },
        digest: CoreDailyDigest(
            summary: String(repeating: "A long recap sentence that runs on and on to test wrapping across many lines. ", count: 12),
            openLoops: []
        )
    )

    /// No events and no meetings.
    static let empty = S.content(events: [], meetings: [], approvals: 0, threads: [], digest: nil)

    static var composer: AnyView { S.composer }
}

extension TodayContent {
    static var sample: TodayContent { TodayFixtures.sample }
    static var worstCase: TodayContent { TodayFixtures.worstCase }
    static var empty: TodayContent { TodayFixtures.empty }
}
