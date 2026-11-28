import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redesign Home adapter (`UI/Redesign/HomeRedesign.swift`): `TodayContent` to the
/// `WHomeModel`, memory blocks, the capture state lines and the search filter. Uses the Today sample content
/// (made-up, a pinned clock). Nothing here touches the app's defaults, the store or a window.
final class RedesignHomeAdapterTests: XCTestCase {
    private let content = TodaySampleContent.sample
    private let now = TodaySampleContent.now

    private func model(memory: WMemoryState = .on, data: HomeRedesignData = HomeRedesignData()) -> WHomeModel {
        HomeRedesignAdapter.model(
            content: content, data: data, memory: memory, elapsedSeconds: nil,
            now: now, voice: SkinDefaults.whimbrel.voice, elements: WHomeElements())
    }

    func testHeroIsTheNextEventAndIsNotRepeatedInTheTimeLine() {
        let m = model()
        guard case let .next(hero) = m.hero else { return XCTFail("expected the next meeting") }
        XCTAssertEqual(hero.id, HomeRedesignAdapter.eventId("e1"))
        XCTAssertFalse(m.timelineMeetings.contains { $0.id == hero.id })
        XCTAssertTrue(m.timelineMeetings.contains { $0.id == HomeRedesignAdapter.eventId("e2") })
    }

    func testLineIsNewestFirstWithNowBetweenLaterAndEarlier() {
        let ids = model().timeline.map(\.id)
        let nowIndex = try! XCTUnwrap(ids.firstIndex(of: "now"))
        // Later meetings sit above the now marker, furthest ahead first (the hero, e1, is drawn between them and now).
        let e3 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.eventId("e3")))
        let e2 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.eventId("e2")))
        XCTAssertLessThan(e3, e2)
        XCTAssertLessThan(e2, nowIndex)
        // Held meetings sit below it, newest first.
        let h2 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.heldId(2)))
        let h1 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.heldId(1)))
        XCTAssertGreaterThan(h2, nowIndex)
        XCTAssertLessThan(h2, h1)
    }

    func testMemoryBlocksAreNewestFirstBelowNow() {
        let m = model(data: HomeRedesignData(blocks: [
            CoreDayBlock(startUnixMs: Int64(now.addingTimeInterval(-7200).timeIntervalSince1970 * 1000), endUnixMs: Int64(now.addingTimeInterval(-6000).timeIntervalSince1970 * 1000), apps: [], moreApps: 0),
            CoreDayBlock(startUnixMs: Int64(now.addingTimeInterval(-1800).timeIntervalSince1970 * 1000), endUnixMs: Int64(now.addingTimeInterval(-600).timeIntervalSince1970 * 1000), apps: [], moreApps: 0),
        ]))
        let ids = m.timeline.map(\.id)
        let newer = ids.firstIndex { $0.hasPrefix("b") && $0.contains(String(Int64(now.addingTimeInterval(-1800).timeIntervalSince1970 * 1000))) }
        let older = ids.firstIndex { $0.hasPrefix("b") && $0.contains(String(Int64(now.addingTimeInterval(-7200).timeIntervalSince1970 * 1000))) }
        XCTAssertNotNil(newer)
        XCTAssertNotNil(older)
        XCTAssertLessThan(newer!, older!)
        XCTAssertGreaterThan(newer!, ids.firstIndex(of: "now")!)
    }

    func testYesterdayAndLastWeekRowsUseRecentMeetings() {
        let m = model()
        let histories = m.timeline.compactMap { stop -> WHomeHistory? in if case .history(let h) = stop { return h } else { return nil } }
        // The sample has meetings 3 and 4 yesterday and meeting 5 three days ago.
        XCTAssertEqual(histories.map(\.id), ["yesterday", "lastWeek"])
        XCTAssertEqual(histories[0].meetings.count, 2)
        XCTAssertEqual(histories[1].meetings.count, 1)
        XCTAssertEqual(m.thread(todayTitle: "Today", nowTime: "x").map(\.id), ["today", "yesterday", "lastWeek"])
    }

    func testPeopleComeFromAttendeesMostOftenFirstWithoutAddresses() {
        let people = HomeRedesignAdapter.people(content: content)
        XCTAssertTrue(people.contains("Priya Nair"))
        XCTAssertFalse(people.contains { $0.contains("@") })
        XCTAssertEqual(Set(people).count, people.count)
    }

    func testPausedOrOffMemoryAddsOneNoticeJustBelowNow() {
        let ids = model(memory: .pausedUntil("3:10 PM")).timeline.map(\.id)
        XCTAssertEqual(ids.filter { $0 == "notice" }.count, 1)
        XCTAssertEqual(ids.firstIndex(of: "notice"), ids.firstIndex(of: "now")! + 1)
        XCTAssertNil(model(memory: .on).timeline.first { if case .notice = $0 { return true } else { return false } })
        XCTAssertNotNil(HomeRedesignAdapter.notice(.off))
    }

    func testBelowTheLineKeepsTodaysOrder() {
        let kinds = model().sections.map(\.kind)
        XCTAssertEqual(kinds, [.approvals, .followups, .threads, .recap])
    }

    func testBlocksCarryTitlesAndAppsButNoText() {
        let core = CoreDayBlock(startUnixMs: 1_000_000, endUnixMs: 1_000_000 + 22 * 60_000, apps: [
            CoreDayBlock.App(appBundleId: "com.microsoft.Excel", items: ["Pricing v3.xlsx"], moreItems: 0),
        ], moreApps: 2)
        let block = HomeRedesignAdapter.block(core)
        XCTAssertEqual(block.id, "1000000")
        XCTAssertEqual(block.items.count, 1)
        XCTAssertEqual(block.items[0].glyph, "tablecells")
        XCTAssertNotNil(block.more)
    }

    func testMeetingIntervalsMergeOverlaps() {
        let day = Calendar.current.startOfDay(for: now)
        let spans = HomeRedesignAdapter.meetingIntervals(content: content, startOfDay: day, endOfDay: day.addingTimeInterval(86400))
        XCTAssertEqual(spans.map(\.start), spans.map(\.start).sorted())
        for (a, b) in zip(spans, spans.dropFirst()) { XCTAssertLessThan(a.end, b.start) }
    }

    func testFilterBecomesMillisecondBounds() {
        let f = HomeRedesignAdapter.libraryFilter(WHomeFilter(appId: "com.apple.mail", dates: .today), apps: [], strings: .english, now: now)
        XCTAssertEqual(f.appId, "com.apple.mail")
        XCTAssertEqual(f.endUnixMs - f.startUnixMs, 86_400_000, accuracy: 3_600_000)
        XCTAssertEqual(f.dateLabel, "Today")
        XCTAssertEqual(HomeRedesignAdapter.libraryFilter(WHomeFilter(person: "Sam"), apps: [], strings: .english, now: now).person, "Sam")
        let none = HomeRedesignAdapter.libraryFilter(WHomeFilter(appId: "x"), apps: [], strings: .english, now: now)
        XCTAssertEqual(none.startUnixMs, 0)
        XCTAssertEqual(none.endUnixMs, 0)
    }

    func testAppDotColorIsSteady() {
        XCTAssertEqual(HomeRedesignAdapter.kind("com.apple.mail"), HomeRedesignAdapter.kind("com.apple.mail"))
    }

    func test_ask_history_pairs_user_and_assistant_turns_in_order() {
        let exchanges = [
            AskExchange(query: "What did Priya say?", answer: "She approved the plan [1].",
                        citations: [CoreContextItemRef(kind: .transcript, id: 3)], model: "m", answeredLocally: false),
            AskExchange(query: "And the budget?", answer: "It stays at 40k.", citations: [], model: nil, answeredLocally: true),
        ]
        let turns = AskRunner.history(exchanges)
        XCTAssertEqual(turns.map(\.role), [.user, .assistant, .user, .assistant])
        XCTAssertEqual(turns.map(\.text), ["What did Priya say?", "She approved the plan [1].", "And the budget?", "It stays at 40k."])
        // An exchange with no answer (it failed) adds no turns.
        let failed = AskExchange(query: "Lost", answer: "", citations: [], model: nil, answeredLocally: false)
        XCTAssertEqual(AskRunner.history([failed] + exchanges).count, 4)
    }

    func test_scope_line_names_where_the_question_is_asked() {
        XCTAssertEqual(AskRunner.scopeLine(.everything, topicName: nil, meetingLabel: nil), "Asking about everything")
        XCTAssertEqual(AskRunner.scopeLine(.topic(4), topicName: "FUEL", meetingLabel: nil), "Asking about FUEL")
        XCTAssertEqual(AskRunner.scopeLine(.meeting(9), topicName: nil, meetingLabel: "Pod 2 Team Call, Nov 19"), "Asking about Pod 2 Team Call, Nov 19")
        // No name to show: no line (the caller hides it).
        XCTAssertNil(AskRunner.scopeLine(.topic(4), topicName: nil, meetingLabel: nil))
        XCTAssertNil(AskRunner.scopeLine(.topic(4), topicName: "", meetingLabel: nil))
        XCTAssertNil(AskRunner.scopeLine(.meeting(9), topicName: nil, meetingLabel: "  "))
    }

    func test_answered_locally_sticks_once_any_earlier_exchange_was() {
        let hosted = AskExchange(query: "q1", answer: "a1", citations: [], model: "m", answeredLocally: false)
        let local = AskExchange(query: "q2", answer: "a2", citations: [], model: nil, answeredLocally: true)
        XCTAssertFalse(AskRunner.answeredLocally(fresh: false, history: []))
        XCTAssertFalse(AskRunner.answeredLocally(fresh: false, history: [hosted]))
        XCTAssertTrue(AskRunner.answeredLocally(fresh: true, history: [hosted]))
        XCTAssertTrue(AskRunner.answeredLocally(fresh: false, history: [hosted, local]))
    }

    // MARK: Today's answer and Continue (Task 6)

    private let ex = AskExchange(query: "What did Priya say?", answer: "She approved the plan [1].",
                                 citations: [CoreContextItemRef(kind: .transcript, id: 3)], model: "m", answeredLocally: false)

    func test_today_answer_is_replaced_not_stacked() {
        var s = TodayAnswerState()
        let second = AskExchange(query: "And the budget?", answer: "It stays at 40k.", citations: [], model: nil, answeredLocally: false)
        s.replace(with: ex)
        s.replace(with: second)
        XCTAssertEqual(s.exchange, second)
        s.clear()
        XCTAssertNil(s.exchange)
        XCTAssertFalse(s.continued)
    }

    func test_continue_only_once_and_only_when_done() {
        var s = TodayAnswerState(); s.replace(with: ex)
        XCTAssertFalse(s.canContinue(phase: .streaming)); XCTAssertTrue(s.canContinue(phase: .done(ex)))
        s.continued = true; XCTAssertFalse(s.canContinue(phase: .done(ex)))
        XCTAssertFalse(TodayAnswerState().canContinue(phase: .failed("x")))
        // A new answer can be continued again.
        s.replace(with: ex); XCTAssertTrue(s.canContinue(phase: .done(ex)))
    }

    func test_continued_citations_are_saved_in_the_library_format() {
        let saved = SavedCitation(item: CoreContextItemRef(kind: .thread, id: 7), label: "Renewal", text: "Dana wrote back.",
                                  open: SavedCitation.Open(kind: "thread", id: 7))
        let json = AskConversationStore.citationsJson([.saved(saved)])
        XCTAssertEqual(try JSONDecoder().decode([SavedCitation].self, from: Data(json.utf8)), [saved])
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]]
        XCTAssertEqual((object?.first?["item"] as? [String: Any])?["kind"] as? String, "thread")
        XCTAssertEqual(AskConversationStore.citationsJson([]), "[]")
    }

    func test_source_numbers_follow_the_answers_markers() {
        // Core lists citations in first-seen marker order: "[2] ... [1]" is citations [ctx2, ctx1].
        XCTAssertEqual(AskSourceCardModel.markerNumbers(answer: "Approved [2], budget [1], again [2].", citationCount: 2), [2, 1])
        // Out-of-range markers were dropped by core; they are always the largest.
        XCTAssertEqual(AskSourceCardModel.markerNumbers(answer: "See [99] and [3] and [abc] and [1].", citationCount: 2), [3, 1])
        // Text that does not account for every citation: plain order.
        XCTAssertEqual(AskSourceCardModel.markerNumbers(answer: "No markers.", citationCount: 2), [1, 2])
        XCTAssertEqual(AskSourceCardModel.markerNumbers(answer: "", citationCount: 0), [])
    }

    func test_source_kind_follows_the_citation_kind() {
        XCTAssertEqual(AskSourceCardModel.sourceKind(.transcript), .transcriptMoment)
        XCTAssertEqual(AskSourceCardModel.sourceKind(.thread), .email)
        XCTAssertEqual(AskSourceCardModel.sourceKind(.routineRun), .routineRun)
        XCTAssertEqual(AskSourceCardModel.sourceKind(.capture), .seenOnScreen)
        XCTAssertEqual(AskSourceCardModel.sourceKind(.episode), .seenOnScreen)
        XCTAssertEqual(AskSourceCardModel.sourceKind(.snapshot), .seenOnScreen)
    }

    func test_a_source_with_nowhere_to_open_has_no_open_control() {
        let slide = SavedCitation(item: CoreContextItemRef(kind: .snapshot, id: 4), label: "A meeting slide", text: "", open: nil)
        let thread = SavedCitation(item: CoreContextItemRef(kind: .thread, id: 7), label: "Renewal", text: "",
                                   open: SavedCitation.Open(kind: "thread", id: 7))
        XCTAssertFalse(AskSourceCardModel.canOpen(.saved(slide)))
        XCTAssertTrue(AskSourceCardModel.canOpen(.saved(thread)))
    }

    func test_title_job_asks_on_the_local_route_whatever_the_hosted_default() {
        // The sentinel query is not time-critical: the title never spends the hosted default.
        let hosted = ConversationTitleJob.defaultRoute((provider: .anthropic, ollamaModel: "llama3", hostedModel: "claude-x"))
        XCTAssertEqual(hosted.provider, .ollama)
        XCTAssertEqual(hosted.modelLabel, "llama3")
        let local = ConversationTitleJob.defaultRoute((provider: .ollama, ollamaModel: "llama3", hostedModel: "llama3"))
        XCTAssertEqual(local.provider, .ollama)
        XCTAssertEqual(local.modelLabel, "llama3")
    }

    func test_conversations_tab_comes_after_topics() {
        XCTAssertEqual(WhimbrelTab.visibleTabs, [.today, .meetings, .library, .topics, .conversations, .automations, .settings])
        XCTAssertEqual(WhimbrelTab.conversations.goKey, "7")
        // The Go keys keep their old numbers; Topics and Conversations took the next free ones.
        XCTAssertEqual(WhimbrelTab.visibleTabs.map(\.goKey), ["1", "2", "3", "6", "7", "4", "5"])
    }

    // MARK: The conversation page's topic chip (Task 7)

    private func topic(_ id: Int64, _ name: String) -> TopicRef { TopicRef(topicId: id, key: "k\(id)", name: name) }

    func test_topic_chip_shows_the_linked_topic() {
        let fuel = topic(1, "FUEL")
        XCTAssertEqual(TopicChipModel.make(CoreConversationTopic(topic: fuel, setByUser: false)), .topic(fuel))
        XCTAssertEqual(TopicChipModel.make(CoreConversationTopic(topic: fuel, setByUser: true)), .topic(fuel))
    }

    func test_topic_chip_offers_add_with_no_topic_or_after_the_user_removed_it() {
        XCTAssertEqual(TopicChipModel.make(CoreConversationTopic(topic: nil, setByUser: false)), .add)
        XCTAssertEqual(TopicChipModel.make(CoreConversationTopic(topic: nil, setByUser: true)), .add)
    }

    func test_topic_chip_menu_lists_cited_topics_once_without_the_current_one() {
        let fuel = topic(1, "FUEL"), pod = topic(2, "Pod 2"), hiring = topic(3, "Hiring")
        XCTAssertEqual(TopicChipModel.menuTopics(cited: [pod, fuel, pod, hiring, fuel], current: fuel), [pod, hiring])
        XCTAssertEqual(TopicChipModel.menuTopics(cited: [hiring, pod, hiring], current: nil), [hiring, pod])
        XCTAssertEqual(TopicChipModel.menuTopics(cited: [fuel], current: fuel), [])
    }

    func test_a_stored_citation_keeps_its_saved_card_when_its_item_is_gone() {
        let goneRef = CoreContextItemRef(kind: .thread, id: 9)
        let stored = SavedCitation(item: goneRef, label: "Renewal", text: "Terms", open: SavedCitation.Open(kind: "thread", id: 9))
        let placeholder = SavedCitation(item: goneRef, label: "", text: "", open: nil)
        let merged = ConversationPageLoad.sources(live: [.saved(placeholder)], stored: [stored])
        guard case .saved(let kept) = merged.first else { return XCTFail("expected the stored citation") }
        XCTAssertEqual(kept, stored)
        // Stored JSON from before citations were kept ("[]") leaves the live reads as they are.
        XCTAssertEqual(ConversationPageLoad.sources(live: [.saved(placeholder)], stored: []).map(\.id), ["thread-9"])
    }

    func test_stored_exchanges_keep_the_answered_locally_line_from_then_on() {
        // Exchange 11 was answered on this Mac: it and every later one say so (spec call 4); earlier ones do not.
        XCTAssertEqual(ConversationPageLoad.answeredLocally(exchangeIds: [10, 11, 12], localOnly: [11]), [false, true, true])
        XCTAssertEqual(ConversationPageLoad.answeredLocally(exchangeIds: [10, 11], localOnly: []), [false, false])
        XCTAssertEqual(ConversationPageLoad.answeredLocally(exchangeIds: [], localOnly: [3]), [])
    }

    // MARK: Saved answers keep pointers to their sources only

    func test_a_saved_answer_holds_only_the_shown_number_kind_and_id_of_each_source() {
        let refs = [CoreContextItemRef(kind: .capture, id: 7), CoreContextItemRef(kind: .routineRun, id: 3)]
        let exchange = AskExchange(query: "q", answer: "Approved [2], budget [1].", citations: refs, model: "m", answeredLocally: false)
        XCTAssertEqual(AskConversationStore.citedSources(exchange),
                       [CoreCitedSource(n: 2, kind: "capture", id: 7), CoreCitedSource(n: 1, kind: "routine_run", id: 3)])
        let json = String(data: try! JSONEncoder().encode(AskConversationStore.citedSources(exchange)), encoding: .utf8)!
        XCTAssertFalse(json.contains("Approved") || json.contains("label") || json.contains("text"), json)
        XCTAssertEqual(AskConversationStore.citedSources(AskExchange(query: "q", answer: "a", citations: [], model: nil, answeredLocally: false)), [])
    }

    func test_a_reopened_answer_reads_its_cards_from_live_rows_and_a_gone_row_has_no_words() throws {
        let json = """
        {"id":1,"title":"t","starred":false,"created_at_unix_ms":1,"updated_at_unix_ms":2,"exchanges":[
          {"id":5,"query":"q","answer":"See [3] and [1] and [2].","created_at_unix_ms":1,"model":"m","legacy":false,
           "provider":"ollama","answered_locally":true,"left_out":0,"cited":[3,1,2,4],
           "sources":[{"n":3,"card":{"kind":"thread","id":9,"found":false,"title":""},"unreadable":false},
                      {"n":1,"card":{"kind":"routine_run","id":4,"found":true,"title":"Daily brief","text":"Live words"},"unreadable":false},
                      {"n":2,"card":{"kind":"capture","id":8,"found":false,"title":""},"unreadable":true},
                      {"n":4,"card":{"kind":"meeting","id":2,"found":true,"title":"x"},"unreadable":false}]}]}
        """
        let convo = try JSONDecoder().decode(CoreSourcedConversation.self, from: Data(json.utf8))
        let row = try XCTUnwrap(convo.exchanges.first)
        XCTAssertFalse(row.legacy)
        XCTAssertEqual(row.cited, [3, 1, 2, 4])
        var asked: [CoreContextItemRef] = []
        let shown = ConversationPageLoad.sourcedSources(row.sources, gone: "Gone", unreadable: "Unreadable") { refs in
            asked = refs
            // A routine run older than the recent list: no read by id, a placeholder.
            return refs.map { .saved(SavedCitation(item: $0, label: "", text: "", open: nil)) }
        }
        // Lowest number first; a kind with no card here is left out; only the row that is there is read.
        XCTAssertEqual(shown.numbers, [1, 2, 3])
        XCTAssertEqual(asked, [CoreContextItemRef(kind: .routineRun, id: 4)])
        guard case .saved(let live) = shown.items[0], case .saved(let unreadable) = shown.items[1], case .saved(let gone) = shown.items[2] else {
            return XCTFail("expected three cards")
        }
        XCTAssertEqual([live.label, live.text], ["Daily brief", "Live words"])
        XCTAssertEqual([unreadable.label, unreadable.text], ["Unreadable", ""])
        XCTAssertNil(unreadable.open)
        XCTAssertEqual([gone.label, gone.text], ["Gone", ""])
        XCTAssertNil(gone.open)
        XCTAssertEqual(gone.item, CoreContextItemRef(kind: .thread, id: 9))
        // The cited meeting has no context ref, but keeps its own card under its saved number.
        XCTAssertEqual(shown.meetings, [ConversationMeetingSource(n: 4, meetingId: 2, title: "x", tsUnixMs: nil, openable: true)])
    }

    /// A follow-up asked under kept Find filters shows its cards from core's numbered list at once, a meeting among them, matching the
    /// answer's marks (no reload).
    func test_a_sourced_follow_up_shows_every_cited_card_meetings_included() {
        let sources = [
            CoreNumberedSource(n: 1, card: CoreSourceCard(kind: "capture", id: 8, title: "Rates")),
            CoreNumberedSource(n: 2, card: CoreSourceCard(kind: "meeting", id: 3, tsUnixMs: 1_000, title: "Pod 2")),
            CoreNumberedSource(n: 3, card: CoreSourceCard(kind: "episode", id: 5, title: "Not cited")),
        ]
        let sourced = AskSourced(sources: sources, cited: [2, 1], citedSources: [], dateRange: nil)
        let shown = ConversationPageLoad.citedSources(sourced, gone: "Gone", unreadable: "Unreadable") { refs in
            refs.map { .saved(SavedCitation(item: $0, label: "", text: "", open: nil)) }
        }
        XCTAssertEqual(shown.numbers, [1])
        XCTAssertEqual(shown.meetings, [ConversationMeetingSource(n: 2, meetingId: 3, title: "Pod 2", tsUnixMs: 1_000, openable: true)])
        XCTAssertEqual(shown.numbers.count + shown.meetings.count, 2, "one card per cited number")
    }

    func test_a_legacy_exchange_is_read_from_its_saved_citations() throws {
        let json = """
        {"id":1,"title":"t","starred":true,"created_at_unix_ms":1,"updated_at_unix_ms":2,"exchanges":[
          {"id":5,"query":"q","answer":"a","created_at_unix_ms":1,"model":null,"legacy":true,"citations_json":"[]",
           "provider":null,"answered_locally":null,"left_out":null,"cited":[],"sources":[]}]}
        """
        let row = try XCTUnwrap(JSONDecoder().decode(CoreSourcedConversation.self, from: Data(json.utf8)).exchanges.first)
        XCTAssertTrue(row.legacy)
        XCTAssertEqual(row.citationsJson, "[]")
        XCTAssertNil(row.answeredLocally)
    }

    func test_the_saved_answered_locally_is_made_on_this_computer_as_core_rules_it() {
        // Kept local by client exclusion, whatever the route.
        XCTAssertTrue(AskRunner.madeOnThisComputer(provider: "anthropic", model: "claude-x", keptLocal: true))
        // A plain Ollama answer, and the bundled model, were made here.
        XCTAssertTrue(AskRunner.madeOnThisComputer(provider: "ollama", model: "gemma3:4b", keptLocal: false))
        XCTAssertTrue(AskRunner.madeOnThisComputer(provider: "local", model: nil, keptLocal: false))
        // An Ollama cloud model and a hosted provider were not; nor an answer whose provider is unknown.
        XCTAssertFalse(AskRunner.madeOnThisComputer(provider: "ollama", model: "gpt-oss:120b-cloud", keptLocal: false))
        XCTAssertFalse(AskRunner.madeOnThisComputer(provider: "openai", model: "gpt-x", keptLocal: false))
        XCTAssertFalse(AskRunner.madeOnThisComputer(provider: nil, model: nil, keptLocal: false))
        // The excluded-client line is not that flag: a plain Ollama answer has no line.
        let ollama = AskExchange(query: "q", answer: "a", citations: [], model: "gemma3:4b", answeredLocally: false,
                                 provider: "ollama", madeOnThisComputer: true)
        XCTAssertFalse(ollama.answeredLocally)
        XCTAssertEqual(ConversationPageLoad.answeredLocally(fresh: [false, true, false]), [false, true, true])
        XCTAssertEqual(ConversationPageLoad.answeredLocally(fresh: []), [])
    }

    func test_a_meeting_answer_continues_with_the_provider_that_streamed_it() {
        var done = MeetingAskExchange(question: "Q", answer: "A [1].")
        done.model = "gemma3:4b"
        done.provider = "ollama"
        done.finished = true
        let exchange = MeetingAskSession.continuable(done)
        XCTAssertEqual(exchange?.provider, "ollama")
        XCTAssertEqual(exchange?.madeOnThisComputer, true)
        XCTAssertEqual(exchange?.answeredLocally, false)
        // The governed chain does not say which provider answered: nil, not made here as far as we know.
        done.provider = nil
        done.model = nil
        XCTAssertNil(MeetingAskSession.continuable(done)?.provider)
        XCTAssertEqual(MeetingAskSession.continuable(done)?.madeOnThisComputer, false)
    }

    func test_left_out_is_the_filtered_count_only() {
        XCTAssertEqual(AskRunner.leftOut(.filteredHosted(leftOut: 3)), 3)
        XCTAssertEqual(AskRunner.leftOut(.localOnly), 0)
        XCTAssertEqual(AskRunner.leftOut(nil), 0)
    }

    // MARK: The Conversations section, and Continue from a meeting (Task 8)

    private func summary(_ id: Int64, _ title: String, starred: Bool = false, updated: Int64) -> CoreMemoryStore.ConversationSummary {
        CoreMemoryStore.ConversationSummary(id: id, title: title, starred: starred, updatedAtUnixMs: updated)
    }

    func test_conversations_sort_starred_first_then_newest() {
        let rows = [summary(1, "Old", updated: 100), summary(2, "Starred old", starred: true, updated: 50),
                    summary(3, "New", updated: 300), summary(4, "Starred new", starred: true, updated: 200)]
        XCTAssertEqual(ConversationsListRedesign.sorted(rows).map(\.id), [4, 2, 3, 1])
        XCTAssertEqual(ConversationsListRedesign.sorted([]).map(\.id), [])
    }

    func test_conversation_search_matches_titles_ignoring_case() {
        let rows = [summary(1, "FUEL renewal terms", updated: 1), summary(2, "Pod 2 hiring", updated: 2), summary(3, "Fuel budget", updated: 3)]
        XCTAssertEqual(ConversationsListRedesign.filter(rows, query: "fuel").map(\.id), [1, 3])
        XCTAssertEqual(ConversationsListRedesign.filter(rows, query: "  HIRING ").map(\.id), [2])
        // An empty search shows every row; one that matches nothing shows none.
        XCTAssertEqual(ConversationsListRedesign.filter(rows, query: " ").map(\.id), [1, 2, 3])
        XCTAssertEqual(ConversationsListRedesign.filter(rows, query: "budgets").map(\.id), [])
    }

    func test_a_conversation_rows_menu_button_names_its_conversation() {
        // Tab and VoiceOver reach the row's menu button on its own: a bare "More" says nothing of which conversation it acts on.
        let row = CoreConversationRow(id: 1, title: "Bake sale prices", starred: false, updatedAtUnixMs: 1, topic: nil, latestAnswerLine: nil)
        XCTAssertEqual(ConversationsListRedesign.menuLabel(row), "Actions for Bake sale prices")
    }

    func test_a_conversation_row_reads_its_topic_and_latest_answer_line() throws {
        let json = #"[{"id":3,"title":"Renewal","starred":true,"updated_at_unix_ms":2200,"topic":{"topic_id":7,"key":"k7","name":"FUEL"},"latest_answer_line":"Due Friday."},{"id":4,"title":"New","starred":false,"updated_at_unix_ms":1000,"topic":null,"latest_answer_line":null}]"#
        let rows = try CoreConversationRow.decodeList(Data(json.utf8))
        XCTAssertEqual(rows.map(\.id), [3, 4])
        XCTAssertEqual(rows[0].topic, TopicRef(topicId: 7, key: "k7", name: "FUEL"))
        XCTAssertEqual(rows[0].latestAnswerLine, "Due Friday.")
        XCTAssertEqual(rows[0].summary.updatedAtUnixMs, 2200)
        XCTAssertTrue(rows[0].summary.starred)
        XCTAssertNil(rows[1].topic)
        XCTAssertNil(rows[1].latestAnswerLine)
    }

    func test_the_ask_bar_on_conversations_starts_an_everything_conversation() {
        XCTAssertEqual(WhimbrelTab.today.askTarget(topicId: nil), .today)
        XCTAssertEqual(WhimbrelTab.conversations.askTarget(topicId: 7), .conversation(.everything))
        XCTAssertEqual(WhimbrelTab.library.askTarget(topicId: 7), .library)
        XCTAssertEqual(WhimbrelTab.meetings.askTarget(topicId: nil), .library)
    }

    func test_a_meeting_answer_continues_only_once_it_is_finished() {
        let refs = [CoreContextItemRef(kind: .transcript, id: 4)]
        var done = MeetingAskExchange(question: "What did we decide?", answer: "Ship Friday [1].")
        done.citations = refs
        done.model = "claude-x"
        done.finished = true
        XCTAssertEqual(MeetingAskSession.continuable(done),
                       AskExchange(query: "What did we decide?", answer: "Ship Friday [1].", citations: refs, model: "claude-x", answeredLocally: false))
        // Its citations not in yet, failed, empty, or already continued: no Continue.
        var reading = done
        reading.finished = false
        XCTAssertNil(MeetingAskSession.continuable(reading))
        var failed = done
        failed.errorText = "Couldn't reach the AI provider."
        XCTAssertNil(MeetingAskSession.continuable(failed))
        var empty = MeetingAskExchange(question: "Q")
        empty.finished = true
        XCTAssertNil(MeetingAskSession.continuable(empty))
        var continued = done
        continued.continued = true
        XCTAssertNil(MeetingAskSession.continuable(continued))
    }

    func test_a_meeting_answer_citing_provisional_live_lines_does_not_continue() {
        // Live lines carry negative ids (-rowid) and are replaced by the final transcript, so a saved
        // pointer to one would never resolve: Continue waits for the final transcript.
        var live = MeetingAskExchange(question: "Q", answer: "A [1] [2].")
        live.citations = [CoreContextItemRef(kind: .transcript, id: 7), CoreContextItemRef(kind: .transcript, id: -3)]
        live.finished = true
        XCTAssertNil(MeetingAskSession.continuable(live))
        live.citations = [CoreContextItemRef(kind: .transcript, id: 7)]
        XCTAssertNotNil(MeetingAskSession.continuable(live))
    }

    // MARK: Final review fixes

    func test_a_follow_up_uses_the_resolved_scope_and_keeps_the_stored_one_when_it_cannot_be_read() {
        // Core resolved it: a merged topic's survivor, or everything once its topic or meeting is gone.
        XCTAssertEqual(ConversationPageLoad.followUpScope(stored: .topic(4), resolved: .topic(9)), .topic(9))
        XCTAssertEqual(ConversationPageLoad.followUpScope(stored: .topic(4), resolved: .everything), .everything)
        XCTAssertEqual(ConversationPageLoad.followUpScope(stored: .meeting(2), resolved: .everything), .everything)
        // A failed read never widens the scope.
        XCTAssertEqual(ConversationPageLoad.followUpScope(stored: .topic(4), resolved: nil), .topic(4))
        XCTAssertEqual(ConversationPageLoad.followUpScope(stored: .meeting(2), resolved: nil), .meeting(2))
        // The scope line for a widened conversation says so.
        XCTAssertEqual(AskRunner.scopeLine(ConversationPageLoad.followUpScope(stored: .topic(4), resolved: .everything),
                                           topicName: nil, meetingLabel: nil), "Asking about everything")
    }

    func test_a_streamed_answer_is_answered_locally_when_the_prepare_kept_it_on_this_mac() {
        XCTAssertTrue(AskRunner.answeredOnThisMac(localRetry: true, exclusion: nil))
        XCTAssertTrue(AskRunner.answeredOnThisMac(localRetry: false, exclusion: .localOnly))
        XCTAssertFalse(AskRunner.answeredOnThisMac(localRetry: false, exclusion: .allowed))
        XCTAssertFalse(AskRunner.answeredOnThisMac(localRetry: false, exclusion: .filteredHosted(leftOut: 2)))
        XCTAssertFalse(AskRunner.answeredOnThisMac(localRetry: false, exclusion: nil))
    }

    func test_a_conversations_read_that_fails_is_an_error_not_an_empty_list() {
        let row = CoreConversationRow(id: 1, title: "One", starred: false, updatedAtUnixMs: 1, topic: nil, latestAnswerLine: nil)
        XCTAssertEqual(ConversationsListModel.listState(nil), .failed)
        XCTAssertEqual(ConversationsListModel.listState([]), .rows([]))
        XCTAssertEqual(ConversationsListModel.listState([row]).rows.map(\.id), [1])
        XCTAssertEqual(ConversationsListModel.listState(nil).rows.map(\.id), [])
    }
}
