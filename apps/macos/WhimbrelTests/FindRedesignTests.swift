import WhimbrelUI
import XCTest
@testable import Whimbrel

/// Library's Find page adapter: the core's JSON shapes, cards, the Open table, filters, routes, saved answers.
/// Pure values: no store, no `AppState`, no `NSWorkspace` (app names are injected).
final class FindRedesignTests: XCTestCase {
    private var utc: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        c.locale = Locale(identifier: "en_US")
        return c
    }

    private var env: FindCardEnv {
        FindCardEnv(
            appName: { $0 == "com.microsoft.Excel" ? "Excel" : $0 }, glyph: { _ in "tablecells" }, dot: { _ in .green }, device: { _ in "This Mac" },
            strings: .english, locale: Locale(identifier: "en_US"), calendar: utc)
    }

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    // MARK: The core's shapes

    func testAFindPageDecodes() throws {
        let page = try decode(CoreFindPage.self, """
        {"results":[{"kind":"capture","id":12,"found":true,"ts_unix_ms":1000,"title":"Pricing v3","text":"Rate cell is empty","app_bundle_id":"com.microsoft.Excel","device_id":"d1"}],
         "mentions":[],"total":1,"mentions_total":0,"counts":{"capture":1},"semantic_used":false,"semantic_available":true,"more":true,"capped":false}
        """)
        XCTAssertEqual(page.results.count, 1)
        XCTAssertEqual(page.results[0].appBundleId, "com.microsoft.Excel")
        XCTAssertEqual(page.results[0].tsUnixMs, 1000)
        XCTAssertEqual(page.counts["capture"], 1)
        XCTAssertTrue(page.more)
        XCTAssertTrue(page.semanticAvailable)
    }

    func testAFindPageSaysWhenMeaningIsCatchingUp() throws {
        let catching = try decode(CoreFindPage.self, """
        {"results":[],"mentions":[],"total":0,"mentions_total":0,"counts":{},"semantic_used":true,"semantic_available":true,"more":false,"capped":false,"meaning_catching_up":true}
        """)
        XCTAssertTrue(catching.meaningCatchingUp)
        // An older core without the key reads as caught up.
        let older = try decode(CoreFindPage.self, """
        {"results":[],"mentions":[],"total":0,"mentions_total":0,"counts":{},"semantic_used":true,"semantic_available":true,"more":false,"capped":false}
        """)
        XCTAssertFalse(older.meaningCatchingUp)
    }

    func testASearchIsReadAgainWhileMeaningCatchesUpAndNoLonger() {
        var page = CoreFindPage()
        page.meaningCatchingUp = true
        XCTAssertEqual(FindController.catchUpDelay(page, tries: 0), 2)
        XCTAssertEqual(FindController.catchUpDelay(page, tries: 5), 10)
        XCTAssertEqual(FindController.catchUpDelay(page, tries: FindController.catchUpTries - 1), 10)
        XCTAssertNil(FindController.catchUpDelay(page, tries: FindController.catchUpTries), "bounded")
        // The sweep waits for an idle, plugged-in Mac: the reads cover about ten minutes.
        let total = (0..<FindController.catchUpTries).compactMap { FindController.catchUpDelay(page, tries: $0) }.reduce(0, +)
        XCTAssertGreaterThanOrEqual(total, 600)
        page.meaningCatchingUp = false
        XCTAssertNil(FindController.catchUpDelay(page, tries: 0))
    }

    func testACatchUpReadAfterLoadMoreKeepsTheRowsLoaded() {
        XCTAssertEqual(FindController.catchUpLimit(loaded: 0), FindController.pageSize)
        XCTAssertEqual(FindController.catchUpLimit(loaded: 100), 100, "every row loaded is read again")
        XCTAssertEqual(FindController.catchUpLimit(loaded: 200), 200)
        XCTAssertNil(FindController.catchUpLimit(loaded: 250), "past one read the catch-up reads stop")
        func cards(_ ids: [Int64]) -> [CoreSourceCard] {
            ids.map { CoreSourceCard(kind: "capture", id: $0) }
        }
        let ids = { (c: [CoreSourceCard]) in c.map(\.id) }
        // The read asked for 2 rows; Load more had added two more meanwhile.
        XCTAssertEqual(ids(FindController.mergeCatchUp(shown: cards([1, 2, 3, 4]), answer: cards([9, 1]), asked: 2)), [9, 1, 3, 4])
        XCTAssertEqual(ids(FindController.mergeCatchUp(shown: cards([1, 2]), answer: cards([9, 1]), asked: 2)), [9, 1])
        XCTAssertEqual(ids(FindController.mergeCatchUp(shown: cards([1, 2, 3, 4]), answer: cards([3, 1]), asked: 2)), [3, 1, 4])
    }

    func testACardWhoseRowIsGoneHasNoTextAndNothingToOpen() throws {
        let c = try decode(CoreSourceCard.self, #"{"kind":"transcript","id":5,"found":false,"title":""}"#)
        XCTAssertNil(c.text)
        let card = FindAdapter.card(c, env: env)
        XCTAssertTrue(card.isGone)
        XCTAssertNil(card.open)
        XCTAssertNil(card.text)
    }

    func testASourcedAnswerDecodes() throws {
        let a = try decode(CoreSourcedAnswer.self, """
        {"text":"Seats [1].","citations":[],"exclusion":{"kind":"allowed"},"answered_by":{"provider":"ollama","model":"gemma","local":true},
         "sources":[{"n":1,"card":{"kind":"capture","id":12,"found":true,"title":"x"}}],"cited":[1],"cited_sources":[{"n":1,"kind":"capture","id":12}],"answered_locally":true}
        """)
        XCTAssertEqual(a.cited, [1])
        XCTAssertEqual(a.citedSources, [CoreCitedSource(n: 1, kind: "capture", id: 12)])
        XCTAssertTrue(a.answeredLocally)
        XCTAssertEqual(a.sources[0].card.id, 12)
    }

    func testAFinalizeAndARoutePreviewDecode() throws {
        let done = try decode(CoreFinalizeSourced.self, #"{"citations":[],"exclusion":{"kind":"filtered_hosted","left_out":2},"cited":[2,1],"cited_sources":[],"left_out":2}"#)
        XCTAssertEqual(done.cited, [2, 1])
        XCTAssertEqual(done.leftOut, 2)
        let p = try decode(CoreRoutePreview.self, #"{"destination":"hosted","local":false,"provider":"anthropic","reason":"x","require_local":false,"may_fall_back":true}"#)
        XCTAssertEqual(p.destination, "hosted")
        XCTAssertTrue(p.mayFallBack)
    }

    func testASourcedConversationHasLegacyAndNumberedExchanges() throws {
        let c = try decode(CoreSourcedConversation.self, """
        {"id":3,"title":"t","starred":false,"created_at_unix_ms":1,"updated_at_unix_ms":2,"exchanges":[
          {"id":1,"query":"q","answer":"old","created_at_unix_ms":1,"model":null,"legacy":true,"citations_json":"[]","provider":null,"answered_locally":null,"left_out":null,"sources":[]},
          {"id":2,"query":"q2","answer":"new [1]","created_at_unix_ms":2,"model":"m","legacy":false,"provider":"anthropic","answered_locally":false,"left_out":0,
           "sources":[{"n":1,"card":{"kind":"capture","id":9,"found":false,"title":""}}]}]}
        """)
        XCTAssertTrue(c.exchanges[0].legacy)
        XCTAssertNil(c.exchanges[0].model)
        XCTAssertFalse(c.exchanges[1].legacy)
        XCTAssertFalse(c.exchanges[1].sources[0].card.found)
    }

    func testTheFilterEncodesOnlyWhatIsSet() throws {
        XCTAssertTrue(CoreFindFilter().isEmpty)
        XCTAssertNil(CoreFindFilter().json)
        var f = CoreFindFilter()
        f.startUnixMs = 5
        f.person = CoreFindFilter.Person(name: "Sam Okafor")
        let obj = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(try XCTUnwrap(f.json).utf8)) as? [String: Any])
        XCTAssertEqual(obj["start_unix_ms"] as? Int, 5)
        XCTAssertNil(obj["end_unix_ms"])
        XCTAssertNil(obj["app_bundle_id"])
        XCTAssertEqual((obj["person"] as? [String: Any])?["name"] as? String, "Sam Okafor")
    }

    // MARK: Cards and the Open table

    func testOpenFollowsTheSpecsTable() {
        func open(_ kind: String, meeting: Int64? = nil, cs: Int64? = nil, found: Bool = true) -> WFindOpen? {
            FindAdapter.open(CoreSourceCard(kind: kind, id: 4, found: found, meetingId: meeting, startCs: cs))
        }
        XCTAssertEqual(open("capture"), .capture(4))
        XCTAssertEqual(open("transcript", meeting: 7, cs: 87_200), .meetingAt(id: 7, startCs: 87_200))
        XCTAssertNil(open("transcript"))
        XCTAssertEqual(open("snapshot", meeting: 7), .meeting(7))
        XCTAssertEqual(open("meeting"), .meeting(4))
        XCTAssertEqual(open("episode"), .episode(4))
        XCTAssertEqual(open("thread"), .thread(4))
        XCTAssertNil(open("routine_run"))
        XCTAssertNil(open("capture", found: false))
    }

    func testATranscriptMomentSaysWhoSpokeAndWhen() {
        let card = FindAdapter.card(CoreSourceCard(
            kind: "transcript", id: 31, tsUnixMs: 1_000, title: "x", text: "I would add a usage tier.", deviceId: "d", meetingId: 7,
            meetingTitle: "Pricing review", speakerLabel: "Sam Okafor", startCs: 87_200, via: "speaker"), env: env)
        XCTAssertEqual(card.kind, .transcriptMoment)
        XCTAssertEqual(card.title, "Pricing review")
        XCTAssertEqual(card.appName, "Meeting audio")
        XCTAssertEqual(card.lineTime, "14:32")
        XCTAssertEqual(card.speaker, "Sam Okafor")
        XCTAssertEqual(card.via, "Spoken by Sam Okafor")
        XCTAssertEqual(card.openHint, "Opens the note at 14:32")
        XCTAssertEqual(card.open, .meetingAt(id: 7, startCs: 87_200))
    }

    func testACaptureShowsTheAppNameNotTheBundleId() {
        let card = FindAdapter.card(CoreSourceCard(kind: "capture", id: 12, tsUnixMs: 1_700_000_000_000, title: "Pricing v3.xlsx", text: "t", appBundleId: "com.microsoft.Excel", deviceId: "d"), env: env)
        XCTAssertEqual(card.appName, "Excel")
        XCTAssertEqual(card.kind, .seenOnScreen)
        XCTAssertEqual(card.device, "This Mac")
        XCTAssertEqual(card.id, "capture-12")
        XCTAssertEqual(card.openHint, "Opens the saved capture")
        XCTAssertFalse(card.time.isEmpty)
    }

    func testSourceCardsKeepOnlyTheCitedNumbersInNumberOrder() {
        let sources = (1...3).map { CoreNumberedSource(n: $0, card: CoreSourceCard(kind: "capture", id: Int64($0), title: "t\($0)")) }
        let cards = FindAdapter.sourceCards(sources, cited: [3, 1], env: env)
        XCTAssertEqual(cards.map(\.number), [1, 3])
        XCTAssertEqual(cards.map(\.id), ["source.1", "source.3"])
        XCTAssertEqual(cards.map(\.entityKey), ["capture-1", "capture-3"])
    }

    // MARK: Filters

    func testTheShellFilterBecomesTheCoresFilter() {
        let now = utc.date(from: DateComponents(year: 2026, month: 11, day: 17, hour: 12))!
        let f = FindAdapter.coreFilter(WHomeFilter(appId: "com.apple.mail", dates: .today, person: "Sam Okafor"), now: now, calendar: utc)
        XCTAssertEqual(f.appBundleId, "com.apple.mail")
        XCTAssertEqual(f.person?.name, "Sam Okafor")
        XCTAssertEqual(f.startUnixMs, Int64(utc.date(from: DateComponents(year: 2026, month: 11, day: 17))!.timeIntervalSince1970 * 1000))
        XCTAssertEqual(f.endUnixMs, Int64(utc.date(from: DateComponents(year: 2026, month: 11, day: 18))!.timeIntervalSince1970 * 1000))
        XCTAssertTrue(FindAdapter.coreFilter(WHomeFilter()).isEmpty)
    }

    func testAHomeBlocksTimeRangeKeepsItsExactBounds() {
        var lib = LibraryCaptureFilter()
        lib.startUnixMs = 1_000_000
        lib.endUnixMs = 2_500_000
        lib.dateLabel = "10:05 to 10:40"
        let home = FindAdapter.homeFilter(lib)
        let core = FindAdapter.coreFilter(home)
        XCTAssertEqual(core.startUnixMs, 1_000_000)
        XCTAssertEqual(core.endUnixMs, 2_500_000)
        XCTAssertNil(core.person)
        XCTAssertTrue(FindAdapter.homeFilter(LibraryCaptureFilter()).isEmpty)
    }

    func testAFilterChangeAfterAnAnswerStartsANewConversation() {
        let a = WHomeFilter(person: "Sam")
        XCTAssertTrue(FindAdapter.startsNewConversation(old: a, new: WHomeFilter(person: "Dana"), hasExchanges: true))
        XCTAssertFalse(FindAdapter.startsNewConversation(old: a, new: a, hasExchanges: true))
        XCTAssertFalse(FindAdapter.startsNewConversation(old: a, new: WHomeFilter(), hasExchanges: false))
    }

    // MARK: Route

    private func preview(_ destination: String, local: Bool, provider: String? = nil, mayFallBack: Bool = false) -> CoreRoutePreview {
        CoreRoutePreview(destination: destination, local: local, provider: provider, model: nil, reason: "", requireLocal: false, mayFallBack: mayFallBack)
    }

    func testOnlyALocalPreviewReadsOnThisMac() {
        XCTAssertEqual(FindAdapter.route(preview("local", local: true, provider: "ollama")), WFindRoute(destination: .local, provider: "ollama", providerName: "Ollama", mayFallBack: false))
        XCTAssertEqual(FindAdapter.route(preview("local", local: false))?.destination, .unknown)
        XCTAssertEqual(FindAdapter.route(preview("refused", local: false))?.destination, .refused)
        XCTAssertEqual(FindAdapter.route(preview("unknown", local: false))?.destination, .unknown)
        XCTAssertNil(FindAdapter.route(nil))
    }

    func testTheRouteCarriesTheProviderAndWhetherTheAnswerMayMove() throws {
        let single = try XCTUnwrap(FindAdapter.route(preview("hosted", local: false, provider: "anthropic", mayFallBack: false)))
        XCTAssertEqual(single, WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic", mayFallBack: false))
        let streamed = try XCTUnwrap(FindAdapter.route(preview("hosted", local: false, provider: "anthropic", mayFallBack: true)))
        XCTAssertTrue(streamed.mayFallBack)
        // The button then says "Ask Anthropic" only for the first, and "Ask" for the second.
        XCTAssertEqual(WFindAskButton.make(route: single, canAsk: true, strings: .english).title, "Ask Anthropic")
        XCTAssertEqual(WFindAskButton.make(route: streamed, canAsk: true, strings: .english).title, "Ask")
    }

    func testAnOllamaCloudModelIsHostedAndNeverReadsLocalOrNamed() throws {
        // Core answers destination hosted with provider ollama for a cloud model: it leaves this Mac.
        let cloud = try XCTUnwrap(FindAdapter.route(preview("hosted", local: false, provider: "ollama", mayFallBack: false)))
        XCTAssertEqual(cloud.destination, .hosted)
        let button = WFindAskButton.make(route: cloud, canAsk: true, strings: .english)
        XCTAssertEqual(button.title, "Ask")
        XCTAssertFalse(button.promisesLocal)
        XCTAssertNil(button.namesProvider)
    }

    // MARK: A follow-up and the route

    func testAFollowUpCarriesALocallyAnsweredExchangeAndStaysLocalWhenThePreviewSaysLocal() throws {
        // History is the answered exchanges as they are; core scans them when it plans the route. A previous answer made from an excluded
        // client's material reroutes the follow-up to this Mac, and the preview says so: the ask is then sent with require_local.
        let turns = AskRunner.history([AskExchange(query: "Q3 renewal terms?", answer: "Answered on this Mac from the client's notes.", citations: [], model: nil, answeredLocally: false)])
        XCTAssertEqual(turns.map(\.text), ["Q3 renewal terms?", "Answered on this Mac from the client's notes."])
        let rerouted = try decode(CoreRoutePreview.self, #"{"destination":"local","local":true,"provider":"ollama","reason":"names_excluded_client","require_local":true,"may_fall_back":false}"#)
        let fresh = FindAdapter.route(rerouted)
        XCTAssertEqual(WFindSendDecision.decide(label: nil, fresh: fresh), .send(requireLocal: true))
        // The button that said "on this Mac" for the first question and now meets a hosted preview sends nothing.
        let label = WFindAskButton.make(route: FindAdapter.route(rerouted), canAsk: true, strings: .english)
        let hosted = FindAdapter.route(preview("hosted", local: false, provider: "anthropic", mayFallBack: true))
        XCTAssertEqual(WFindSendDecision.decide(label: label, fresh: hosted), .changed)
    }

    // MARK: Saved answers, history, words

    func testAnOlderAnswerDrawsItsPlainChips() {
        let json = #"[{"item":{"kind":"capture","id":5},"label":"Excel","text":"Rate cell","open":{"kind":"capture","id":5}},{"item":{"kind":"routine_run","id":2},"label":"Friday recap","text":"x"}]"#
        let chips = FindAdapter.legacyChips(json)
        XCTAssertEqual(chips.count, 2)
        XCTAssertEqual(chips[0].open, .capture(5))
        XCTAssertEqual(chips[0].tooltip, "Excel: Rate cell")
        XCTAssertNil(chips[1].open)
        XCTAssertTrue(FindAdapter.legacyChips("[]").isEmpty)
        XCTAssertTrue(FindAdapter.legacyChips(nil).isEmpty)
        XCTAssertTrue(FindAdapter.legacyChips("not json").isEmpty)
    }

    func testAFailedExchangeIsNotReplayedToTheModel() {
        let turns = AskRunner.history(["a1", "", "a3"].enumerated().map { i, a in
            AskExchange(query: "q\(i + 1)", answer: a, citations: [], model: nil, answeredLocally: false)
        })
        XCTAssertEqual(turns.map(\.text), ["q1", "a1", "q3", "a3"])
        XCTAssertEqual(turns.map(\.role), [.user, .assistant, .user, .assistant])
    }

    func testATitleWithDashesIsShownExactlyAsItIs() {
        let title = "Q3 \u{2014} Review, Mon\u{2013}Fri"
        XCTAssertEqual(
            FindAdapter.statusLine(recordingPaused: true, meetingTitle: title, systemAudio: false, mic: false, screenText: false, captureEnabled: true),
            "Recording paused \u{00B7} " + title)
        XCTAssertEqual(
            FindAdapter.statusLine(recordingPaused: false, meetingTitle: title, systemAudio: true, mic: true, screenText: false, captureEnabled: true),
            "Capturing: system audio, mic \u{00B7} " + title)
        XCTAssertEqual(
            FindAdapter.statusLine(recordingPaused: false, meetingTitle: nil, systemAudio: false, mic: false, screenText: true, captureEnabled: true), "Capturing: screen text")
    }

    func testTheWordsTheFindPageAddsHoldNoDash() {
        let lines = [
            FindAdapter.statusLine(recordingPaused: false, meetingTitle: nil, systemAudio: false, mic: false, screenText: false, captureEnabled: true),
            FindAdapter.statusLine(recordingPaused: false, meetingTitle: nil, systemAudio: false, mic: false, screenText: false, captureEnabled: false),
            FindAdapter.failureText(.spendCap(scope: .global, spentCents: 50, capCents: 50)),
            FindAdapter.failureText(.providerFailure),
            AskLocalRouteCopy.answered, AskLocalRouteCopy.inFlight, AskLocalRouteCopy.askingPlain, AskLocalRouteCopy.waiting,
            ClientCopy.leftOut(2) ?? "", AskShared.storeUnavailableMessage,
            String(localized: "Recording on speakers. The mic may pick up meeting audio. Headphones reduce this."),
        ]
        for line in lines {
            XCTAssertFalse(line.contains("\u{2014}") || line.contains("\u{2013}"), "a dash in: \(line)")
        }
        XCTAssertTrue(FindAdapter.failureText(.spendCap(scope: .global, spentCents: 50, capCents: 60)).contains("50"))
    }

    func testOtherFailuresKeepTheOldScreensWords() {
        XCTAssertEqual(FindAdapter.failureText(.providerFailure), CoreLLMError.providerFailure.userFacingMessage)
        XCTAssertEqual(
            FindAdapter.failureText(.spendCap(scope: .user, spentCents: 5, capCents: 9)), CoreLLMError.spendCap(scope: .user, spentCents: 5, capCents: 9).userFacingMessage)
    }

    /// The reference parse: the whole answer through one Markdown parse, keeping what it can of a malformed one.
    private func wholeTextParse(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.failurePolicy = .returnPartiallyParsedIfPossible
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    func testTheAnswerParsesMarkdownAsOneWholeTextParseDoes() {
        for text in ["This is **bold** text", "- item one\n- item two", "just plain text, no formatting", "this has an *unclosed emphasis marker", "a [1] b `c[2]`"] {
            XCTAssertEqual(WAnswerAttributed.markdown(text), wholeTextParse(text), text)
        }
    }

    /// The page's blocks, read one at a time, say the same words as one parse of the whole answer (twins of the package's
    /// `OldPageEqualityTests`, which run under `swift test`).
    func testBlocksReadAsOneWholeTextParseReadsThem() {
        func newPage(_ text: String) -> String {
            WAnswerBlocks.parse(text).map { block -> String in
                switch block {
                case .paragraph(let w), .heading(let w), .item(_, let w, _): return String(WAnswerAttributed.markdown(w).characters)
                case .code(let w), .table(let w): return w
                }
            }.joined()
        }
        for text in [
            "line one\nline two", "line one\nline two\nline three\n\nnext paragraph", "Intro\n\n- x\n- y\n  - nested\n- z", "1. first\n   - inner\n2. second",
            "```code``` and then words\nmore words", "## Heading\ntext under it", "**bold** and\n*soft* breaks",
        ] {
            XCTAssertEqual(newPage(text), String(wholeTextParse(text).characters), text)
        }
    }

    func testMarksFollowTheCoresCitedListAndNothingElse() {
        let built = WAnswerAttributed.build("[1] and [2] and [3]", cited: [2])
        XCTAssertEqual(WAnswerAttributed.markNumbers(in: built), [2])
        XCTAssertEqual(String(built.characters), "[1] and [2] and [3]")
    }

    // MARK: Unreadable sources, saved cited numbers, the answer's date range

    func testASourceTheCoreCouldNotReadIsNotAGoneRow() throws {
        let s = try decode(CoreNumberedSource.self, #"{"n":2,"card":{"kind":"capture","id":9,"found":false,"title":""},"unreadable":true}"#)
        XCTAssertTrue(s.unreadable)
        let card = FindAdapter.numberedCard(s, env: env)
        XCTAssertTrue(card.isUnreadable)
        XCTAssertFalse(card.isGone)
        XCTAssertNil(card.open)
        XCTAssertEqual(card.number, 2)
        let gone = FindAdapter.numberedCard(try decode(CoreNumberedSource.self, #"{"n":3,"card":{"kind":"capture","id":9,"found":false,"title":""},"unreadable":false}"#), env: env)
        XCTAssertTrue(gone.isGone)
        XCTAssertFalse(gone.isUnreadable)
    }

    func testTheUnreadableFlagIsOptionalAndTheMemberwiseInitStays() throws {
        let s = try decode(CoreNumberedSource.self, #"{"n":1,"card":{"kind":"capture","id":9,"found":true,"title":"t"}}"#)
        XCTAssertFalse(s.unreadable)
        XCTAssertFalse(CoreNumberedSource(n: 1, card: CoreSourceCard(kind: "capture", id: 1)).unreadable)
    }

    func testASavedExchangeCarriesItsCitedNumbersAndTheirUnreadableFlags() throws {
        let c = try decode(CoreSourcedConversation.self, """
        {"id":3,"title":"t","starred":false,"exchanges":[
          {"id":2,"query":"q2","answer":"new [1] [2]","created_at_unix_ms":2,"model":"m","legacy":false,"provider":"anthropic","answered_locally":false,"left_out":0,"cited":[2,1],
           "sources":[{"n":1,"card":{"kind":"capture","id":9,"found":false,"title":""},"unreadable":true},{"n":2,"card":{"kind":"capture","id":8,"found":true,"title":"x"},"unreadable":false}]},
          {"id":3,"query":"q3","answer":"old","created_at_unix_ms":3,"model":null,"legacy":true,"sources":[]}]}
        """)
        XCTAssertEqual(c.exchanges[0].cited, [2, 1])
        XCTAssertEqual(c.exchanges[0].sources.map(\.unreadable), [true, false])
        XCTAssertNil(c.exchanges[1].cited)
    }

    func testTheAnswersDateRangeNamesTheDaysItDrewOn() throws {
        let a = try decode(CoreSourcedAnswer.self, """
        {"text":"x","sources":[],"cited":[],"cited_sources":[],"answered_locally":false,"date_range":{"start_unix_ms":1604880000000,"end_unix_ms":1605484800000}}
        """)
        XCTAssertEqual(a.dateRange, CoreDateRange(startUnixMs: 1_604_880_000_000, endUnixMs: 1_605_484_800_000))
        XCTAssertEqual(FindAdapter.rangeText(a.dateRange, calendar: utc, locale: Locale(identifier: "en_US")), "Nov 9 to Nov 15")
        XCTAssertNil(FindAdapter.rangeText(CoreDateRange(startUnixMs: 5, endUnixMs: nil), calendar: utc))
        XCTAssertNil(FindAdapter.rangeText(nil, calendar: utc))
        let none = try decode(CoreSourcedAnswer.self, #"{"text":"x","sources":[],"cited":[],"cited_sources":[],"answered_locally":false,"date_range":null}"#)
        XCTAssertNil(none.dateRange)
    }

    // MARK: Errors of the sourced calls

    func testASourcedCallsInvalidFilterIsNotAProviderFailure() {
        let bad = Data(#"{"ok":false,"error":{"kind":"invalid_filter"}}"#.utf8)
        XCTAssertEqual(CoreLLMEnvelope<CorePrepareSourcedData>.sourcedResult(data: bad).failureValue, .searchNotRun)
        XCTAssertEqual(CoreLLMEnvelope<CorePrepareSourcedData>.sourcedResult(data: Data(#"{"ok":false,"error":{"kind":"something_new"}}"#.utf8)).failureValue, .searchNotRun)
        XCTAssertEqual(CoreLLMError.searchNotRun.userFacingMessage, "That search could not be run.")
        // Every other call decodes the same envelope as before.
        XCTAssertEqual(CoreLLMEnvelope<CorePrepareSourcedData>.result(data: bad).failureValue, .providerFailure)
    }

    func testTheSourcedCallsKnownErrorsDecodeAsBefore() {
        let cap = Data(#"{"ok":false,"error":{"kind":"spend_cap","scope":"user","spent_cents":5,"cap_cents":9}}"#.utf8)
        XCTAssertEqual(CoreLLMEnvelope<CorePrepareSourcedData>.sourcedResult(data: cap).failureValue, .spendCap(scope: .user, spentCents: 5, capCents: 9))
        let failure = Data(#"{"ok":false,"error":{"kind":"provider_failure","reason":{"kind":"ollama_not_running"}}}"#.utf8)
        XCTAssertEqual(CoreLLMEnvelope<CorePrepareSourcedData>.sourcedResult(data: failure).failureValue, .providerFailureBecause(.ollamaNotRunning))
    }

    func testTheCountSaysSourcesOnceAnAnswerIsShown() {
        XCTAssertEqual(FindAdapter.countText(total: 5, mentions: 2, answerSources: nil, strings: .english), "Results: 7")
        XCTAssertEqual(FindAdapter.countText(total: 5, mentions: 2, answerSources: 3, strings: .english), "3 sources found")
        XCTAssertEqual(FindAdapter.countText(total: 5, mentions: 2, answerSources: 1, strings: .english), "1 source found")
        XCTAssertNil(FindAdapter.countText(total: 0, mentions: 0, answerSources: nil, strings: .english))
    }

    func testHiddenEpisodesAreListedOnlyOnRequest() {
        func episode(_ id: Int64) -> CoreEpisode {
            CoreEpisode(id: id, deviceId: "d", appBundleId: "com.microsoft.Excel", windowTitle: "Pricing v3.xlsx", startTsUnixMs: 1_000, endTsUnixMs: 61_000, itemCount: 3, summary: "Worked on pricing.", status: "summarized")
        }
        let shown = FindAdapter.episodeRows([episode(1), episode(2)], hidden: [2], showHidden: false, env: env)
        XCTAssertEqual(shown.map(\.id), ["episode-1"])
        let all = FindAdapter.episodeRows([episode(1), episode(2)], hidden: [2], showHidden: true, env: env)
        XCTAssertEqual(all.map(\.isHidden), [false, true])
        XCTAssertEqual(all[0].card.appName, "Excel")
        XCTAssertEqual(all[0].card.text, "Worked on pricing.")
        XCTAssertEqual(all[0].card.open, .episode(1))
    }

    // MARK: Find on the shared runner

    func testASourcedAnswerSavesCoresOwnPointersMeetingsIncluded() {
        let cited = [CoreCitedSource(n: 2, kind: "meeting", id: 7), CoreCitedSource(n: 4, kind: "capture", id: 9)]
        var exchange = AskExchange(query: "q", answer: "See [4] and [2].", citations: AskRunner.refs(cited), model: nil, answeredLocally: false)
        exchange.sourced = AskSourced(sources: [], cited: [2, 4], citedSources: cited, dateRange: nil)
        // Core's numbers and kinds, not the ones guessed from the text; a meeting has no context ref but is saved.
        XCTAssertEqual(AskConversationStore.citedSources(exchange), cited)
        XCTAssertEqual(AskRunner.refs(cited), [CoreContextItemRef(kind: .capture, id: 9)])
        // Without core's list (Today, a conversation, a meeting) the numbers still come from the answer's marks.
        let plain = AskExchange(query: "q", answer: "See [1].", citations: [CoreContextItemRef(kind: .capture, id: 9)], model: nil, answeredLocally: false)
        XCTAssertEqual(AskConversationStore.citedSources(plain), [CoreCitedSource(n: 1, kind: "capture", id: 9)])
    }

    func testALocalOnlyAskWithNoLocalModelSaysSoAndAFailedLocalModelKeepsItsWords() {
        // Never "Couldn't reach the AI provider. Try again." when no model here is set up: that is false, and invites a retry.
        XCTAssertEqual(AskRunner.sourcedFailureText(.providerFailure, requireLocal: true, localModelAvailable: false), AskRunner.localOnlyFailure)
        XCTAssertNotEqual(AskRunner.localOnlyFailure, CoreLLMError.providerFailure.userFacingMessage)
        // A local model that was there and failed keeps the failure's own words.
        XCTAssertEqual(AskRunner.sourcedFailureText(.providerFailure, requireLocal: true, localModelAvailable: true), CoreLLMError.providerFailure.userFacingMessage)
        XCTAssertEqual(AskRunner.sourcedFailureText(.providerFailure, requireLocal: false, localModelAvailable: false), CoreLLMError.providerFailure.userFacingMessage)
        XCTAssertEqual(
            AskRunner.sourcedFailureText(.providerFailureBecause(.creditExhausted), requireLocal: true, localModelAvailable: false),
            CoreLLMError.providerFailureBecause(.creditExhausted).userFacingMessage)
    }

    func testAFindConversationKeepsDatesAndAppsAndOnlyThatAPersonWasLeftOut() {
        let sent = CoreFindFilter(startUnixMs: 1_000, endUnixMs: 86_401_000, appBundleId: "com.tinyspeck.slackmacgap", person: .init(name: "Sam Okafor"))
        let kept = FindAdapter.savedFindFilter(sent)
        XCTAssertEqual(kept, CoreSavedFindFilter(startUnixMs: 1_000, endUnixMs: 86_401_000, appBundleId: "com.tinyspeck.slackmacgap", peopleLeftOut: true))
        // Encoded for core: no person field at all.
        let json = String(decoding: try! JSONEncoder().encode(kept), as: UTF8.self)
        XCTAssertFalse(json.contains("Sam"))
        XCTAssertFalse(json.contains("person\""))
        // Asked again under the same app and days, never the person.
        let follow = FindAdapter.coreFilter(kept)
        XCTAssertEqual(follow, CoreFindFilter(startUnixMs: 1_000, endUnixMs: 86_401_000, appBundleId: "com.tinyspeck.slackmacgap", person: nil))
        let chips = FindAdapter.reopenedFilter(kept)
        XCTAssertEqual(chips.appId, "com.tinyspeck.slackmacgap")
        XCTAssertNil(chips.person)
        XCTAssertNotNil(chips.dates)
        // Only a left-out person: nothing to ask under.
        XCTAssertNil(FindAdapter.coreFilter(CoreSavedFindFilter(startUnixMs: nil, endUnixMs: nil, appBundleId: nil, peopleLeftOut: true)))
        XCTAssertEqual(FindAdapter.savedFindFilter(nil), CoreSavedFindFilter(startUnixMs: nil, endUnixMs: nil, appBundleId: nil, peopleLeftOut: false))
    }

    func testAScopedConversationOpensOnTheConversationsPage() {
        XCTAssertEqual(WFindConversationOpen.destination(FindAdapter.scopeKind(.everything)), .find)
        XCTAssertEqual(WFindConversationOpen.destination(FindAdapter.scopeKind(.topic(4))), .conversationsPage)
        XCTAssertEqual(WFindConversationOpen.destination(FindAdapter.scopeKind(.meeting(9))), .conversationsPage)
    }

    func testOnlyAStreamThatRunsHereCountsAsOnThisMac() {
        let ollama = AskStreamRoute(provider: .ollama, apiKey: nil, baseURLOverride: nil)
        let local = AskStreamRoute(provider: .local, apiKey: nil, baseURLOverride: nil)
        let anthropic = AskStreamRoute(provider: .anthropic, apiKey: "k", baseURLOverride: nil)
        XCTAssertTrue(AskRunner.streamIsOnDevice(local, model: "local"))
        XCTAssertTrue(AskRunner.streamIsOnDevice(ollama, model: "gemma4:12b"))
        XCTAssertFalse(AskRunner.streamIsOnDevice(ollama, model: "gpt-oss:120b-cloud"))
        XCTAssertFalse(AskRunner.streamIsOnDevice(anthropic, model: "claude"))
        XCTAssertFalse(AskRunner.streamIsOnDevice(nil, model: "gemma4:12b"))
    }

    func testLocalOnlyRoutingKeepsKeysButMarksTheCallSensitive() {
        // Built by hand: `RoutingSettings()` reads UserDefaults, and a machine with Local-only mode on would make the base sensitive.
        let base = CoreRoutingConfig(
            anthropicKey: "k", openaiKey: nil, googleKey: nil, typesafeKey: nil, userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "anthropic", preferredModel: "m", customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: "")
        XCTAssertFalse(base.sensitive)
        let local = base.localOnly()
        XCTAssertTrue(local.sensitive)
        XCTAssertEqual(local.preferredProvider, "ollama")
        XCTAssertEqual(local.anthropicKey, "k")
    }

    func testThePageSaysAFailureOfItsOwnInPlainWords() {
        XCTAssertFalse(CoreLLMError.searchNotRun.userFacingMessage.contains("\u{2014}"))
    }
}

private extension Result {
    var failureValue: Failure? { if case .failure(let e) = self { return e } else { return nil } }
}
