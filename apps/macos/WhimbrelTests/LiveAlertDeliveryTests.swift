import XCTest
@testable import Whimbrel

@MainActor
final class LiveAlertDeliveryTests: XCTestCase {
    private var posted: [LiveAlertDelivery.Notice] = []
    private var sent: [(subject: String, body: String)] = []
    private var statuses: [(Int64, String)] = []
    private var slept: [UInt64] = []
    private var sendResults: [Result<Void, SelfSendError>] = []

    override func setUp() {
        super.setUp()
        posted = []
        sent = []
        statuses = []
        slept = []
        sendResults = []
    }

    private func makeDelivery(
        emailEnabled: Bool = true, alertsEnabled: Bool = true, sleep: ((UInt64) async -> Void)? = nil
    ) -> LiveAlertDelivery {
        LiveAlertDelivery(
            deviceId: "mac-1",
            alertsEnabled: { alertsEnabled },
            post: { [unowned self] in posted.append($0) },
            sendEmail: { [unowned self] subject, body in
                sent.append((subject, body))
                return sendResults.isEmpty ? .success(()) : sendResults.removeFirst()
            },
            setEmailStatus: { [unowned self] id, status in statuses.append((id, status)) },
            emailEnabled: { emailEnabled },
            evidence: { refs in refs.map { "Item \($0.id) · Nov 1, 2026" } },
            context: { _ in ["3:20  Them: Thanks, Sam.", "3:22  Them: So the numbers..."] },
            sleep: sleep ?? { [unowned self] seconds in slept.append(seconds) }
        )
    }

    private func alert(
        id: Int64 = 7, origin: String? = nil, sourceId: Int64? = nil, sourceMeetingId: Int64 = 3,
        asker: String? = nil, draftJson: String? = nil, createdAt: Int64 = 1_000
    ) -> CoreLiveAlert {
        CoreLiveAlert(
            id: id, meetingId: 3, originDeviceId: origin, sourceId: sourceId ?? id, sourceMeetingId: sourceMeetingId,
            meetingTitle: "Standup", quote: "Mike, can you send the numbers?", restatement: "Send the Q3 numbers",
            asker: asker, atCs: 12_345, createdAtUnixMs: createdAt, draftJson: draftJson, emailStatus: nil, notifiedAtUnixMs: nil
        )
    }

    func testRecipientIsAlwaysTheConnectedAccount() async {
        XCTAssertEqual(GmailSelfSend.recipient(connectedAccount: "mike@x.com"), "mike@x.com")
        XCTAssertNil(GmailSelfSend.recipient(connectedAccount: nil))

        var plans: [CoreWriteActionPlan] = []
        let none = GmailSelfSend(connectedAccount: { nil }, token: { _ in "tok" }, transport: { plan, _ in
            plans.append(plan)
            return .success(resultJson: "{}")
        })
        let result = await none.send(subject: "S", body: "B")
        guard case .failure(.notConnected) = result else { return XCTFail("expected notConnected") }
        XCTAssertTrue(plans.isEmpty)

        let connected = GmailSelfSend(connectedAccount: { "mike@x.com" }, token: { _ in "tok" }, transport: { plan, _ in
            plans.append(plan)
            return .success(resultJson: "{}")
        })
        let ok = await connected.send(subject: "S", body: "B")
        guard case .success = ok else { return XCTFail("expected success") }
        let args = try? JSONSerialization.jsonObject(with: Data(plans.first?.argsJson.utf8 ?? "".utf8)) as? [String: String]
        XCTAssertEqual(args, ["to": "mike@x.com", "subject": "S", "body": "B"])
        XCTAssertEqual(plans.first?.tool, "send_email")
    }

    func testEmailFailureRetriesOnceAndRecordsReason() async {
        sendResults = [.failure(.failed("Google rejected it")), .failure(.failed("Google rejected it"))]
        let delivery = makeDelivery()
        delivery.alertDetected(alert())
        await delivery.settle()
        XCTAssertEqual(sent.count, 2)
        XCTAssertEqual(statuses.map(\.1), ["failed:Google rejected it"])
        XCTAssertEqual(statuses.map(\.0), [7])
    }

    func testEmailWaitsUpToSixtySecondsForDraft() async {
        let delivery = makeDelivery()
        delivery.alertDetected(alert())
        await delivery.settle()
        XCTAssertEqual(slept, [60])
        XCTAssertEqual(sent.count, 1)
        XCTAssertEqual(sent.first?.subject, "Question for you in \"Standup\"")
        XCTAssertTrue(sent.first?.body.contains("Draft wasn't ready in time; it's on the meeting page.") ?? false)
        XCTAssertEqual(statuses.map(\.1), ["sent"])
        // A draft arriving afterwards updates the notification but never sends a second email.
        delivery.alertDrafted(alert(), answer: CoreChatAnswer(text: "Yes, sending today.", citations: []))
        await delivery.settle()
        XCTAssertEqual(sent.count, 1)
        XCTAssertEqual(posted.map(\.id), ["live-alert-mac-1-7", "live-alert-mac-1-7"])
        XCTAssertEqual(posted.map(\.sound), [true, false], "the draft replaces the alert silently")
    }

    func testEmailBodyCarriesQuoteDraftEvidenceAndLink() {
        let url = DeepLink.meetingURL(id: 3, atCs: 12_345)
        let body = LiveAlertDelivery.emailBody(
            alert: alert(asker: "Sam"), answer: CoreChatAnswer(text: "Yes, by Friday.", citations: []),
            meetingURL: url, evidence: ["Q3 plan · Nov 1, 2026"]
        )
        for part in ["Mike, can you send the numbers?", "Sam", "Send the Q3 numbers", "Yes, by Friday.", "Q3 plan · Nov 1, 2026", url.absoluteString] {
            XCTAssertTrue(body.contains(part), part)
        }
    }

    func testDeepLinkRoundTrip() {
        let url = DeepLink.meetingURL(id: 42, atCs: 12_345)
        XCTAssertEqual(url.absoluteString, "whimbrel://meeting/42?at=12345")
        let parsed = DeepLink.parse(url)
        XCTAssertEqual(parsed?.meetingId, 42)
        XCTAssertEqual(parsed?.atCs, 12_345)
        XCTAssertNil(DeepLink.parse(URL(string: "whimbrel://elsewhere/42?at=1")!))
        XCTAssertNil(DeepLink.parse(URL(string: "https://meeting/42?at=1")!))
    }

    func testNotificationTitleFallsBackWhenNoAsker() {
        let draft = CoreChatAnswer(text: "Yes, by Friday.\nMore detail here.", citations: [])
        let named = LiveAlertDelivery.content(for: alert(asker: "Sam"), answer: draft)
        XCTAssertEqual(named.title, "Sam asked you")
        XCTAssertEqual(named.body, "Send the Q3 numbers\nYes, by Friday.")
        XCTAssertEqual(LiveAlertDelivery.content(for: alert(asker: nil), answer: nil).title, "Question for you")
        XCTAssertEqual(LiveAlertDelivery.content(for: alert(asker: "  "), answer: nil).body, "Send the Q3 numbers")
    }

    func testPeerAlertsNotifyOnceAfterPull() async {
        // Mirrors takeUnnotifiedPeerAlerts: an alert comes back once, then it's marked notified.
        var unnotified = [alert(id: 20, origin: "pc-1", sourceId: 5)]
        let take: () async -> [CoreLiveAlert] = {
            defer { unnotified = [] }
            return unnotified
        }
        let delivery = makeDelivery()
        await delivery.afterPull(take: take)
        await delivery.afterPull(take: take)
        XCTAssertEqual(posted.map(\.id), ["live-alert-pc-1-5"])
        XCTAssertEqual(posted.first?.deepLink, DeepLink.meetingURL(id: 3, atCs: 12_345))
        XCTAssertEqual(posted.first?.subtitle, "Standup", "a peer's alert names its meeting")
        XCTAssertTrue(sent.isEmpty, "a peer's alert is emailed by the device that heard it")
    }

    func testPeerAlertsCapAtSixThenOneSummary() async {
        let delivery = makeDelivery()
        let first = (1...8).map { alert(id: 100 + Int64($0), origin: "pc-1", sourceId: Int64($0)) }
        await delivery.afterPull(take: { first })
        XCTAssertEqual(posted.count, 7)
        XCTAssertEqual(posted.prefix(6).map(\.id), (1...6).map { "live-alert-pc-1-\($0)" })
        XCTAssertEqual(posted.last?.id, "live-alert-more-pc-1-3")
        XCTAssertEqual(posted.last?.body, "2 more questions for you")

        // A later pull for the same meeting updates the one summary in place.
        await delivery.afterPull(take: { [self] in [alert(id: 109, origin: "pc-1", sourceId: 9)] })
        XCTAssertEqual(posted.count, 8)
        XCTAssertEqual(posted.last?.id, "live-alert-more-pc-1-3")
        XCTAssertEqual(posted.last?.body, "3 more questions for you")
        XCTAssertFalse(posted.last?.sound ?? true, "the summary updates in place silently")

        // Another device's alerts for its own meeting have their own ceiling.
        await delivery.afterPull(take: { [self] in [alert(id: 110, origin: "mac-2", sourceId: 1)] })
        XCTAssertEqual(posted.last?.id, "live-alert-mac-2-1")
    }

    func testFailedInsertStillNotifiesWithoutStatusWrites() async {
        let delivery = makeDelivery()
        delivery.alertDetected(alert(id: -1))
        delivery.alertDrafted(alert(id: -1), answer: CoreChatAnswer(text: "Draft", citations: []))
        await delivery.settle()
        XCTAssertEqual(posted.count, 2)
        XCTAssertEqual(sent.count, 1)
        XCTAssertTrue(statuses.isEmpty)
    }

    func testEmailOffRecordsOffAndSendsNothing() async {
        let delivery = makeDelivery(emailEnabled: false)
        delivery.alertDetected(alert())
        delivery.alertDrafted(alert(), answer: nil)
        await delivery.settle()
        XCTAssertTrue(sent.isEmpty)
        XCTAssertEqual(statuses.map(\.1), ["off"])
    }

    // MARK: - Fix round 1

    func testOnlyLiveAlertsPresentInTheForeground() {
        XCTAssertEqual(NotificationClickHandler.presentationOptions(forIdentifier: "live-alert-mac-1-7"), [.banner, .list, .sound])
        XCTAssertEqual(NotificationClickHandler.presentationOptions(forIdentifier: "live-alert-more-mac-1-3"), [.banner, .list, .sound])
        XCTAssertEqual(NotificationClickHandler.presentationOptions(forIdentifier: "recording-notice-ABC"), [])
        XCTAssertEqual(NotificationClickHandler.presentationOptions(forIdentifier: "routine-run-4"), [])
    }

    func testEmailBodyCarriesContextLinesInOrderAboveTheQuote() {
        let body = LiveAlertDelivery.emailBody(
            alert: alert(), answer: nil, meetingURL: DeepLink.meetingURL(id: 3, atCs: 12_345),
            context: ["3:20  Them: Thanks, Sam.", "3:22  Them: So the numbers..."]
        )
        let first = body.range(of: "3:20  Them: Thanks, Sam.")
        let second = body.range(of: "3:22  Them: So the numbers...")
        let quote = body.range(of: "Mike, can you send the numbers?")
        XCTAssertNotNil(first)
        XCTAssertNotNil(second)
        XCTAssertLessThan(first!.lowerBound, second!.lowerBound)
        XCTAssertLessThan(second!.lowerBound, quote!.lowerBound)
    }

    func testContextLinesAreTheTwoMinutesBeforeTheQuestion() {
        func seg(_ id: Int64, _ startCs: Int64, _ label: String, _ text: String) -> CoreLiveSegment {
            CoreLiveSegment(id: id, meetingId: 3, channel: "others", chunkIndex: 0, startCs: startCs, endCs: startCs + 100, speakerLabel: label, text: text)
        }
        let lines = LiveAlertDelivery.contextLines(
            [seg(4, 12_000, "Them", "So the numbers"), seg(1, 100, "Them", "too early"), seg(2, 345, "Me", "first"),
             seg(5, 12_345, "Them", "the question itself"), seg(3, 6_100, "", "no label")],
            atCs: 12_345
        )
        XCTAssertEqual(lines, ["0:03  Me: first", "1:01  others: no label", "2:00  Them: So the numbers"])
    }

    func testDelivererPassesContextIntoTheEmail() async {
        let delivery = makeDelivery()
        delivery.alertDetected(alert())
        await delivery.settle()
        XCTAssertTrue(sent.first?.body.contains("3:20  Them: Thanks, Sam.") ?? false)
    }

    func testTranscriptCitationResolvesToMeetingTitleDateAndLink() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        let meetingId = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "d", startUnixMs: 1_762_000_000_000, endUnixMs: nil, title: "Roadmap review",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording)))
        XCTAssertTrue(store.insertTranscriptSegments(meetingId: meetingId, segments: [
            NewTranscriptSegmentInput(startCs: 0, speakerLabel: "Me", text: "hello"),
            NewTranscriptSegmentInput(startCs: 4_200, speakerLabel: "Them", text: "the roadmap ships Friday"),
        ]))
        let segment = try XCTUnwrap(store.getTranscriptSegment(id: try XCTUnwrap(store.searchTranscriptKeyword("roadmap", limit: 5).first).id))
        XCTAssertEqual(segment.meetingId, meetingId)
        XCTAssertEqual(segment.startCs, 4_200)

        let lines = LiveAlertDelivery.evidenceLines([CoreContextItemRef(kind: .transcript, id: segment.id)], store: store)
        XCTAssertEqual(lines.count, 1)
        XCTAssertTrue(lines[0].hasPrefix("Roadmap review · "), lines[0])
        XCTAssertTrue(lines[0].contains(DeepLink.meetingURL(id: meetingId, atCs: 4_200).absoluteString), lines[0])
        XCTAssertNil(store.getTranscriptSegment(id: 999_999))
    }

    func testCaptureEvidenceKeepsOnlyWebLinks() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        let jsonl = dir.appendingPathComponent("c.jsonl")
        let records = [("Secrets", "file:///etc/passwd"), ("Docs", "HTTPS://example.com/doc")].map { title, url in
            String(decoding: try! JSONSerialization.data(withJSONObject: [
                "timestamp": "2026-09-10T12:00:00Z", "bundle_id": "com.example.\(title)", "window_title": title,
                "url": url, "text": "text for \(title)", "source": "ax",
            ]), as: UTF8.self)
        }
        try records.joined(separator: "\n").write(to: jsonl, atomically: true, encoding: .utf8)
        XCTAssertEqual(store.ingestJSONL(path: jsonl, deviceId: "d")?.itemsInserted, 2)
        let items = store.recent(limit: 10)
        let secrets = try XCTUnwrap(items.first { $0.windowTitle == "Secrets" })
        let docs = try XCTUnwrap(items.first { $0.windowTitle == "Docs" })

        let evidence = LiveAlertDelivery.evidence(
            [CoreContextItemRef(kind: .capture, id: secrets.id), CoreContextItemRef(kind: .capture, id: docs.id)], store: store)
        XCTAssertEqual(evidence.count, 2)
        XCTAssertNil(evidence[0].link, "a file: URL is never a link")
        XCTAssertEqual(evidence[1].link?.absoluteString, "HTTPS://example.com/doc")
    }

    func testPeerAlertsRespectTheAlertSwitchButAreStillConsumed() async {
        var takes = 0
        let delivery = makeDelivery(alertsEnabled: false)
        await delivery.afterPull(take: { [self] in
            takes += 1
            return [alert(id: 20, origin: "pc-1", sourceId: 5)]
        })
        XCTAssertEqual(takes, 1, "rows are still taken (marked notified) so they don't fire later")
        XCTAssertTrue(posted.isEmpty)
    }

    func testDraftInsideTheWindowGoesInTheEmail() async throws {
        var timer: CheckedContinuation<Void, Never>?
        let delivery = makeDelivery(sleep: { _ in await withCheckedContinuation { timer = $0 } })
        delivery.alertDetected(alert())
        delivery.alertDrafted(alert(), answer: CoreChatAnswer(text: "Yes, sending today.", citations: []))
        // Both tasks run on the main actor: wait (no cap) until the timer is parked and the
        // draft's email has gone. The timer can't fire on its own, so the order is fixed.
        while timer == nil || statuses.isEmpty { await Task.yield() }
        XCTAssertEqual(sent.count, 1)
        XCTAssertTrue(sent.first?.body.contains("Yes, sending today.") ?? false)
        XCTAssertFalse(sent.first?.body.contains("wasn't ready") ?? true)
        try XCTUnwrap(timer).resume()  // the 60 s timer fires after the draft: no second email
        await delivery.settle()
        XCTAssertEqual(sent.count, 1)
        XCTAssertEqual(statuses.map(\.1), ["sent"])
    }

    // MARK: - Final review fixes

    func testOwnAlertNoticeHasNoSubtitle() async {
        let delivery = makeDelivery(emailEnabled: false)
        delivery.alertDetected(alert())
        await delivery.settle()
        XCTAssertNil(posted.first?.subtitle)
        XCTAssertEqual(posted.first?.sound, true)
    }

    func testDeepLinkRoundTripWithOriginAndSource() {
        let url = DeepLink.meetingURL(id: 42, atCs: 12_345, origin: "mac 1&x")
        XCTAssertEqual(url.absoluteString, "whimbrel://meeting/42?at=12345&origin=mac%201%26x&src=42")
        XCTAssertEqual(DeepLink.parse(url), DeepLink.Target(meetingId: 42, atCs: 12_345, origin: "mac 1&x", sourceMeetingId: 42))
        XCTAssertEqual(DeepLink.parse(DeepLink.meetingURL(id: 42, atCs: 7)), DeepLink.Target(meetingId: 42, atCs: 7))
        XCTAssertNil(DeepLink.parse(URL(string: "whimbrel://meeting/42?at=1&origin=mac-1")!), "origin needs src")
        XCTAssertNil(DeepLink.parse(URL(string: "whimbrel://meeting/42?at=1&origin=mac-1&src=x")!))
        XCTAssertNil(DeepLink.parse(URL(string: "whimbrel://meeting/42?at=1&origin=&src=42")!))
        XCTAssertNil(DeepLink.parse(URL(string: "whimbrel://meeting/42?at=1&origin=\(String(repeating: "a", count: 129))&src=42")!))
        XCTAssertNotNil(DeepLink.parse(URL(string: "whimbrel://meeting/42?at=1&origin=\(String(repeating: "a", count: 128))&src=42")!))
    }

    func testOwnDeviceLinkUsesTheIdAndAForeignOneResolvesThroughTheMap() {
        var lookups: [(String, Int64)] = []
        let lookup: (String, Int64) -> Int64? = { origin, src in
            lookups.append((origin, src))
            return origin == "mac-2" && src == 42 ? 7 : nil
        }
        let own = DeepLink.Target(meetingId: 42, atCs: 0, origin: "mac-1", sourceMeetingId: 42)
        XCTAssertEqual(DeepLink.localMeetingId(own, deviceId: "mac-1", lookup: lookup), 42)
        XCTAssertEqual(DeepLink.localMeetingId(DeepLink.Target(meetingId: 42, atCs: 0), deviceId: "mac-1", lookup: lookup), 42)
        XCTAssertTrue(lookups.isEmpty)
        let foreign = DeepLink.Target(meetingId: 42, atCs: 0, origin: "mac-2", sourceMeetingId: 42)
        XCTAssertEqual(DeepLink.localMeetingId(foreign, deviceId: "mac-1", lookup: lookup), 7)
        let unsynced = DeepLink.Target(meetingId: 42, atCs: 0, origin: "mac-3", sourceMeetingId: 42)
        XCTAssertNil(DeepLink.localMeetingId(unsynced, deviceId: "mac-1", lookup: lookup), "not synced here: no longer available")
    }

    func testEmailLinkCarriesThisDeviceAndItsMeetingId() async {
        let delivery = makeDelivery()
        delivery.alertDetected(alert())
        await delivery.settle()
        XCTAssertTrue(sent.first?.body.contains("whimbrel://meeting/3?at=12345&origin=mac-1&src=3") ?? false, sent.first?.body ?? "")
    }

    func testCitationDateIsOmittedWhenUnknown() {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        XCTAssertEqual(LiveAlertDelivery.evidenceText("Roadmap review", unixMs: nil, formatter: formatter), "Roadmap review")
        XCTAssertFalse(LiveAlertDelivery.evidenceText("Roadmap review", unixMs: 1_762_000_000_000, formatter: formatter).contains("1970"))
        XCTAssertTrue(LiveAlertDelivery.evidenceText("Roadmap review", unixMs: 1_762_000_000_000, formatter: formatter).hasPrefix("Roadmap review · "))
    }

    func testContextLinesShiftLiveRowsIntoMeetingTime() {
        let row = CoreLiveSegment(id: 1, meetingId: 3, channel: "system", chunkIndex: 0, startCs: 5_000, endCs: 5_100, speakerLabel: "Them", text: "So the deck")
        XCTAssertEqual(LiveAlertDelivery.contextLines([row], atCs: 246_000, partOffsetCs: 240_000), ["40:50  Them: So the deck"])
        XCTAssertEqual(LiveAlertDelivery.contextLines([row], atCs: 246_000), [])
    }

    func testEvidenceDedupesOnTitleAndDateKeepingTheFirstLink() {
        let a = URL(string: "https://a.example/x")!
        let b = URL(string: "https://b.example/y")!
        let out = LiveAlertDelivery.deduped([
            .init(text: "Plan · Nov 2", link: nil), .init(text: "Plan · Nov 2", link: a), .init(text: "Plan · Nov 2", link: b),
            .init(text: "Other · Nov 3", link: b), .init(text: "Other · Nov 3", link: a),
        ])
        XCTAssertEqual(out, [.init(text: "Plan · Nov 2", link: a), .init(text: "Other · Nov 3", link: b)])
    }

    func testAPeerAlertWithoutALocalMeetingLinksThroughItsOrigin() {
        var peer = alert(id: 5, origin: "mac-2", sourceId: 9, sourceMeetingId: 77)
        peer = CoreLiveAlert(
            id: peer.id, meetingId: nil, originDeviceId: "mac-2", sourceId: 9, sourceMeetingId: 77, meetingTitle: peer.meetingTitle,
            quote: peer.quote, restatement: peer.restatement, asker: nil, atCs: 12_345, createdAtUnixMs: 1_000,
            draftJson: nil, emailStatus: nil, notifiedAtUnixMs: nil
        )
        let target = LiveAlertDelivery.link(for: peer, atCs: 12_345).flatMap(DeepLink.parse)
        XCTAssertEqual(target, DeepLink.Target(meetingId: 77, atCs: 12_345, origin: "mac-2", sourceMeetingId: 77))
        XCTAssertEqual(LiveAlertDelivery.link(for: alert(id: 1), atCs: 0), DeepLink.meetingURL(id: 3, atCs: 0))
    }
}
