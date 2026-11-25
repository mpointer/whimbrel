import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

private func card(_ id: String, ms: Int64?, key: String? = nil) -> WFindCard {
    var c = WFindCard(id: id, kind: .seenOnScreen, title: id, timeMs: ms)
    if let key { c.entityKey = key }
    return c
}

private var cal: Calendar {
    var c = Calendar(identifier: .gregorian)
    c.timeZone = TimeZone(identifier: "UTC")!
    c.locale = Locale(identifier: "en_US")
    return c
}

private func ms(_ day: Int, _ hour: Int = 12) -> Int64 {
    Int64(cal.date(from: DateComponents(year: 2026, month: 11, day: day, hour: hour))!.timeIntervalSince1970 * 1000)
}

@Suite struct AnswerMarkTests {
    @Test func marksSitWhereTheNumbersWere() {
        let parts = WAnswerMarks.parts("Friday [1], then Monday [2].", cited: [1, 2])
        #expect(parts == [.text("Friday "), .mark(1), .text(", then Monday "), .mark(2), .text(".")])
    }

    @Test func aNumberTheCoreDidNotCiteStaysPlainText() {
        let parts = WAnswerMarks.parts("See [9] and [1].", cited: [1])
        #expect(parts == [.text("See [9] and "), .mark(1), .text(".")])
    }

    @Test func theGrammarIsTheCoresDigitsOnlyClosedByABracket() {
        // The core's golden: "[0], [x], [3 and [4]" and "[1, 2]" are not markers; "[4]" after an open "[3 and " is.
        let text = "Out of range [9], [0], [x], [3 and [4]. Pair [1, 2]."
        #expect(WAnswerMarks.numbers(in: text, cited: [1, 2, 3, 4]) == [4])
        #expect(WAnswerMarks.parts("[1, 2]", cited: [1, 2]) == [.text("[1, 2]")])
    }

    @Test func citedIsFirstCitedOrderWithoutRepeats() {
        #expect(WAnswerMarks.numbers(in: "[3][1][3] and [2]", cited: [1, 2, 3]) == [3, 1, 2])
        #expect(WAnswerMarks.numbers(in: "[10] [2]", cited: [1, 2, 3]) == [2])
    }

    @Test func plainTextDropsMarksAndTheSpaceBeforeThem() {
        #expect(WAnswerMarks.plain("It is Friday [1]. Also [7] stays.", cited: [1]) == "It is Friday. Also [7] stays.")
    }

    @Test func noMarksMeansOneRun() {
        #expect(WAnswerMarks.parts("Nothing cited here.", cited: [1]) == [.text("Nothing cited here.")])
        #expect(WAnswerMarks.parts("", cited: [1]).isEmpty)
    }
}

@Suite struct FindDayTests {
    @Test func daysKeepTheOrderTheyComeAndSplitAtMidnight() {
        let days = WFindDays.group(
            [card("a", ms: ms(13, 17)), card("b", ms: ms(13, 9)), card("c", ms: ms(10, 14))],
            today: "Today", yesterday: "Yesterday", undated: "Gone", now: Date(timeIntervalSince1970: Double(ms(20)) / 1000), calendar: cal)
        #expect(days.map(\.heading) == ["Fri, Nov 13", "Tue, Nov 10"])
        #expect(days[0].cards.map(\.id) == ["a", "b"])
    }

    @Test func todayAndYesterdayUseTheGivenWords() {
        let now = Date(timeIntervalSince1970: Double(ms(20)) / 1000)
        let days = WFindDays.group([card("a", ms: ms(20, 8)), card("b", ms: ms(19, 8))], today: "Today", yesterday: "Yesterday", undated: "Gone", now: now, calendar: cal)
        #expect(days.map(\.heading) == ["Today", "Yesterday"])
    }

    @Test func aCardWithNoTimeGoesLastUnderItsOwnHeading() {
        let days = WFindDays.group([card("gone", ms: nil), card("a", ms: ms(13))], today: "T", yesterday: "Y", undated: "No longer here", now: Date(timeIntervalSince1970: Double(ms(20)) / 1000), calendar: cal)
        #expect(days.map(\.heading) == ["Fri, Nov 13", "No longer here"])
    }

    @Test func sourcesReadForwardInTimeWithGoneOnesLast() {
        let ordered = WFindDays.sourcesInTimeOrder([card("x", ms: ms(13)), card("gone", ms: nil), card("y", ms: ms(10)), card("z", ms: ms(13))])
        #expect(ordered.map(\.id) == ["y", "x", "z", "gone"])
    }

    @Test func aSourceIsNotListedAgainAmongTheResults() {
        var model = WFindModel()
        let slack = card("capture-1", ms: ms(12))
        model.results = [slack, card("capture-2", ms: ms(11))]
        model.exchanges = [WFindExchange(id: "e", question: "q", scope: "", body: .answered(text: "x [1]"), sources: [slack.numbered(1)])]
        let rows = WFindTimeline.rows(model: model, strings: .english, today: "Today", yesterday: "Yesterday", now: Date(timeIntervalSince1970: Double(ms(20)) / 1000), calendar: cal)
        let cards = rows.flatMap(\.cards).map(\.id)
        #expect(cards == ["source.1", "capture-2"])
    }
}

@Suite struct AskButtonTests {
    private let s = WFindStrings.english

    @Test func localOnlyWhenThePreviewSaidLocal() {
        let b = WFindAskButton.make(route: WFindRoute(destination: .local, provider: "ollama"), canAsk: true, strings: s)
        #expect(b.title == "Ask, on this Mac")
        #expect(b.promisesLocal)
        #expect(b.namesProvider == nil)
    }

    @Test func hostedNamesTheProviderOnlyWhenNothingCanMoveTheAnswer() {
        let b = WFindAskButton.make(route: WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic", mayFallBack: false), canAsk: true, strings: s)
        #expect(b.title == "Ask Anthropic")
        #expect(!b.promisesLocal)
        #expect(b.namesProvider == "Anthropic")
    }

    @Test func aStreamedHostedAnswerCanFallBackSoItSaysOnlyAsk() {
        // The core says may_fall_back for every streamed hosted preview: a failed stream or a spent budget answers through the whole chain.
        let streamed = WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic", mayFallBack: true)
        let b = WFindAskButton.make(route: streamed, canAsk: true, strings: s)
        #expect(b.title == "Ask")
        #expect(b.namesProvider == nil)
        #expect(!b.promisesLocal)
        // An unknown fallback counts as a possible one.
        #expect(WFindAskButton.make(route: WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic"), canAsk: true, strings: s).title == "Ask")
    }

    @Test func anOllamaCloudModelLeavesTheMacAndNeverReadsLocalOrNamed() {
        // Core returns destination hosted with provider ollama for a cloud model.
        let cloud = WFindRoute(destination: .hosted, provider: "ollama", providerName: "Ollama", mayFallBack: false)
        let b = WFindAskButton.make(route: cloud, canAsk: true, strings: s)
        #expect(b.title == "Ask")
        #expect(!b.promisesLocal)
        #expect(b.namesProvider == nil)
    }

    @Test func unknownRefusedAndNoPreviewSayOnlyAsk() {
        for route in [WFindRoute(destination: .unknown), WFindRoute(destination: .refused), nil] {
            let b = WFindAskButton.make(route: route, canAsk: true, strings: s)
            #expect(b.title == "Ask")
            #expect(!b.promisesLocal)
        }
        #expect(WFindAskButton.make(route: WFindRoute(destination: .hosted, mayFallBack: false), canAsk: true, strings: s).title == "Ask")
    }

    @Test func aLocalPromiseNeverStreamsToAHostedRoute() {
        #expect(!WAskStreamEnd.mayStream(promisesLocal: true, streamIsOnDevice: false, hasStreamRoute: true))
        #expect(WAskStreamEnd.mayStream(promisesLocal: true, streamIsOnDevice: true, hasStreamRoute: true))
        #expect(WAskStreamEnd.mayStream(promisesLocal: false, streamIsOnDevice: false, hasStreamRoute: true))
        #expect(!WAskStreamEnd.mayStream(promisesLocal: false, streamIsOnDevice: true, hasStreamRoute: false))
    }
}

@Suite struct SendDecisionTests {
    private let s = WFindStrings.english
    private let local = WFindRoute(destination: .local, provider: "ollama")
    private let single = WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic", mayFallBack: false)
    private let streamed = WFindRoute(destination: .hosted, provider: "anthropic", providerName: "Anthropic", mayFallBack: true)

    private func label(_ route: WFindRoute?) -> WFindAskButton? { route.map { WFindAskButton.make(route: $0, canAsk: true, strings: s) } }

    @Test func aLocalLabelSendsOnlyWhileThePreviewOfWhatIsSentStillSaysLocal() {
        #expect(WFindSendDecision.decide(label: label(local), fresh: local) == .send(requireLocal: true))
        for fresh in [single, streamed, WFindRoute(destination: .refused), WFindRoute(destination: .unknown), nil] {
            #expect(WFindSendDecision.decide(label: label(local), fresh: fresh) == .changed)
        }
    }

    @Test func aNamedLabelSendsOnlyToThatProviderWithNoFallback() {
        #expect(WFindSendDecision.decide(label: label(single), fresh: single) == .send(requireLocal: false))
        #expect(WFindSendDecision.decide(label: label(single), fresh: streamed) == .changed)
        #expect(WFindSendDecision.decide(label: label(single), fresh: local) == .changed)
        #expect(WFindSendDecision.decide(label: label(single), fresh: WFindRoute(destination: .hosted, provider: "openai", providerName: "OpenAI", mayFallBack: false)) == .changed)
        #expect(WFindSendDecision.decide(label: label(single), fresh: nil) == .changed)
    }

    @Test func anAskWithNoLabelPromisesNothingButStaysOnThisMacWhenThePreviewSaysLocal() {
        // Command Return from another tab, a hand-off from Today, a follow-up, a New Conversation prompt, an ask right after opening Find.
        #expect(WFindSendDecision.decide(label: nil, fresh: streamed) == .send(requireLocal: false))
        #expect(WFindSendDecision.decide(label: nil, fresh: nil) == .send(requireLocal: false))
        #expect(WFindSendDecision.decide(label: nil, fresh: WFindRoute(destination: .refused)) == .send(requireLocal: false))
        #expect(WFindSendDecision.decide(label: nil, fresh: local) == .send(requireLocal: true))
        // A plain "Ask" label is also no promise.
        #expect(WFindSendDecision.decide(label: label(streamed), fresh: streamed) == .send(requireLocal: false))
        #expect(WFindSendDecision.decide(label: label(streamed), fresh: local) == .send(requireLocal: true))
    }

    @Test func aFollowUpFieldKeepsItsTextUnlessTheAskStarted() {
        #expect(WFollowUpField.textAfterSending("and Monday?", started: true).isEmpty)
        #expect(WFollowUpField.textAfterSending("and Monday?", started: false) == "and Monday?")
    }
}

@Suite struct AnswerBlockTests {
    @Test func paragraphsListsHeadingsAndCodeBecomeBlocks() {
        let text = "Intro line.\n\n- first [1]\n- second\n  continues here\n2. numbered\n## Next\n```\nlet a = [1]\n```\nDone."
        #expect(WAnswerBlocks.parse(text) == [
            .paragraph("Intro line."), .item(marker: "\u{2022}", text: "first [1]", level: 0), .item(marker: "\u{2022}", text: "second\ncontinues here", level: 0),
            .item(marker: "2.", text: "numbered", level: 0), .heading("Next"), .code("let a = [1]"), .paragraph("Done."),
        ])
    }

    @Test func blankLinesOnlySeparateAndNothingIsDropped() {
        #expect(WAnswerBlocks.parse("a\n\n\nb") == [.paragraph("a"), .paragraph("b")])
        #expect(WAnswerBlocks.parse("").isEmpty)
        #expect(WAnswerBlocks.parse("2026. A year") == [.paragraph("2026. A year")])
        #expect(WAnswerBlocks.parse("-5 degrees") == [.paragraph("-5 degrees")])
    }

    @Test func textWithoutSpacesStaysOneBlockForTheTextViewToWrap() {
        let long = String(repeating: "\u{65E5}\u{672C}\u{8A9E}", count: 60) + " https://example.com/" + String(repeating: "a", count: 200)
        #expect(WAnswerBlocks.parse(long) == [.paragraph(long)])
    }
}

@Suite struct AnswerAttributedTests {
    private func plain(_ a: AttributedString) -> String { String(a.characters) }

    @Test func aMarkBecomesALinkRunToItsSourceInsideOneString() {
        let built = WAnswerAttributed.build("Sam wants a tier [1] and churn data [2].", cited: [1, 2])
        #expect(plain(built) == "Sam wants a tier [1] and churn data [2].")
        #expect(WAnswerAttributed.markNumbers(in: built) == [1, 2])
        let urls = built.runs.compactMap(\.link)
        #expect(urls.map(\.absoluteString) == ["whimbrelsource:1", "whimbrelsource:2"])
        #expect(urls.compactMap(WAnswerAttributed.source(in:)) == [1, 2])
    }

    @Test func markdownIsRenderedAroundTheMarks() {
        let built = WAnswerAttributed.build("It is **due Friday** [1], see *notes*.", cited: [1])
        #expect(!plain(built).contains("**"))
        #expect(plain(built) == "It is due Friday [1], see notes.")
        let bold = built.runs.first { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true && !String(built[$0.range].characters).contains("[") }
        #expect(plain(AttributedString(built[bold!.range])) == "due Friday")
        #expect(WAnswerAttributed.markNumbers(in: built) == [1])
    }

    @Test func aMarkInsideBoldKeepsTheBoldAndStillLinks() {
        let built = WAnswerAttributed.build("**Total [3]**", cited: [3])
        #expect(plain(built) == "Total [3]")
        #expect(WAnswerAttributed.markNumbers(in: built) == [3])
    }

    @Test func aMarkerInsideACodeSpanStaysLiteral() {
        let built = WAnswerAttributed.build("Use `a[1]` here [1].", cited: [1])
        #expect(plain(built) == "Use a[1] here [1].")
        #expect(WAnswerAttributed.markNumbers(in: built) == [1])
        let code = built.runs.first { $0.inlinePresentationIntent?.contains(.code) == true }
        #expect(code != nil && code!.link == nil)
    }

    @Test func aNumberThatIsNotASourceAndBracketedTextStayWords() {
        let built = WAnswerAttributed.build("See [9], [x], [1, 2] and [1].", cited: [1])
        #expect(plain(built) == "See [9], [x], [1, 2] and [1].")
        #expect(WAnswerAttributed.markNumbers(in: built) == [1])
    }

    @Test func linksThatAreNotSourcesAreLeftAlone() {
        let built = WAnswerAttributed.build("Docs at [the site](https://example.com) [1].", cited: [1])
        let links = built.runs.compactMap(\.link)
        #expect(links.count == 2)
        #expect(links.first.flatMap(WAnswerAttributed.source(in:)) == nil)
        #expect(WAnswerAttributed.markNumbers(in: built) == [1])
    }

    @Test func plainTextAndMalformedMarkdownKeepTheirWords() {
        #expect(plain(WAnswerAttributed.build("just words", cited: [])) == "just words")
        #expect(plain(WAnswerAttributed.build("an *unclosed marker", cited: [])).contains("unclosed marker"))
    }

    @Test func aSourceLinkIsReadBackFromItsUrlAndNothingElseIs() throws {
        #expect(WAnswerAttributed.source(in: try #require(WAnswerAttributed.url(forSource: 12))) == 12)
        #expect(WAnswerAttributed.source(in: try #require(URL(string: "https://example.com/12"))) == nil)
    }

    @Test func theSelectedMarkIsTheOneThatGetsABackground() {
        let built = WAnswerAttributed.build("a [1] b [2]", cited: [1, 2], selected: 2, markColor: .blue, selectedBackground: .yellow)
        let filled = built.runs.filter { $0.backgroundColor != nil }.compactMap(\.link).compactMap(WAnswerAttributed.source(in:))
        #expect(filled == [2])
    }
}

@Suite struct AnnouncementTests {
    @Test func anAnswerIsSpokenWithoutItsMarksAndAFailureAsItsMessage() {
        let sources = [WFindCard(id: "x", kind: .seenOnScreen, title: "t").numbered(1)]
        let answered = WFindExchange(id: "e", question: "q", scope: "", body: .answered(text: "Fridays [1]."), sources: sources)
        #expect(WFindAnnouncement.spoken(for: answered, strings: .english) == "Answer: Fridays.")
        let failed = WFindExchange(id: "f", question: "q", scope: "", body: .failed(message: "Not sent.", buttonTitle: nil))
        #expect(WFindAnnouncement.spoken(for: failed, strings: .english) == "Not sent.")
    }
}

@Suite struct StreamEndTests {
    private func end(_ phase: WAskStreamEnd.Phase, output: Bool = false, c402: Bool = false, unreadable: Bool = false, local: Bool = false) -> WAskStreamEnd {
        WAskStreamEnd.decide(phase: phase, hadOutput: output, isWhimbrelAI402: c402, exclusionUnreadable: unreadable, requireLocal: local)
    }

    @Test func aCompletedStreamIsFinalized() { #expect(end(.completed) == .finalize) }
    @Test func stoppingRestoresTheQuestion() { #expect(end(.cancelled) == .discardAndRestoreQuestion) }
    @Test func aWhimbrelAICreditFailureSaysSoWithoutRetrying() { #expect(end(.failed, c402: true) == .failWith(.creditExhausted)) }
    @Test func anUnreadableExclusionSettingIsNotRetried() { #expect(end(.failed, unreadable: true) == .failWith(.exclusionUnreadable)) }
    @Test func failureAfterOutputIsShownNotRetried() { #expect(end(.failed, output: true) == .failWith(.providerFailure)) }
    @Test func failureWithNoOutputFallsBackAndKeepsTheLocalPromise() {
        #expect(end(.failed, local: true) == .fallBackToBlocking(requireLocal: true))
        #expect(end(.failed, local: false) == .fallBackToBlocking(requireLocal: false))
    }
}

@Suite struct CitedMarkTests {
    /// The answer's marks are the numbers the core returned as cited and nothing else, even when another number is also a source the model saw.
    @Test func onlyTheCoresCitedNumbersAreMarks() {
        let sources = (1...3).map { WFindCard(id: "x\($0)", kind: .seenOnScreen, title: "t").numbered($0) }
        let exchange = WFindExchange(id: "e", question: "q", scope: "", body: .answered(text: "a [1] b [2] c [3]"), sources: sources, cited: [2])
        #expect(exchange.citedNumbers == [2])
        #expect(WAnswerMarks.parts("a [1] b [2] c [3]", cited: exchange.citedNumbers) == [.text("a [1] b "), .mark(2), .text(" c [3]")])
        #expect(WAnswerMarks.plain("a [1] b [2]", cited: exchange.citedNumbers) == "a [1] b")
    }

    @Test func withNoExplicitListTheSourcesNumbersAreTheCitedOnes() {
        let sources = [WFindCard(id: "x", kind: .seenOnScreen, title: "t").numbered(4), WFindCard(id: "y", kind: .seenOnScreen, title: "t").numbered(2)]
        #expect(WFindExchange(id: "e", question: "q", scope: "", body: .answered(text: ""), sources: sources).citedNumbers == [2, 4])
        #expect(WFindExchange(id: "e", question: "q", scope: "", body: .legacy(text: "", chips: [])).citedNumbers.isEmpty)
    }

    @Test func aBuiltStringMarksOnlyTheCitedNumbers() {
        let built = WAnswerAttributed.build("[1] and [2] and [3]", cited: [3])
        #expect(WAnswerAttributed.markNumbers(in: built) == [3])
        #expect(String(built.characters) == "[1] and [2] and [3]")
    }
}

@Suite struct UnreadableSourceTests {
    private func gone(_ id: String) -> WFindCard { WFindCard(id: id, kind: .seenOnScreen, title: "", isGone: true) }
    private func unreadable(_ id: String) -> WFindCard { WFindCard(id: id, kind: .seenOnScreen, title: "", isUnreadable: true) }

    @Test func anUnreadableSourceIsNotListedUnderNoLongerOnThisDevice() {
        let days = WFindDays.group(
            [card("a", ms: ms(13)), gone("g"), unreadable("u")], today: "Today", yesterday: "Yesterday", undated: "No longer on this device",
            unreadable: "Could not read just now", now: Date(timeIntervalSince1970: Double(ms(20)) / 1000), calendar: cal)
        #expect(days.map(\.heading).suffix(2) == ["No longer on this device", "Could not read just now"])
        #expect(days.last?.cards.map(\.id) == ["u"])
        #expect(days[days.count - 2].cards.map(\.id) == ["g"])
    }

    @Test func withoutAnUnreadableHeadingTheOldGroupingHolds() {
        let days = WFindDays.group([gone("g"), unreadable("u")], today: "T", yesterday: "Y", undated: "Gone", now: Date(), calendar: cal)
        #expect(days.count == 1 && days[0].cards.count == 2)
    }

    @Test func theTimeLineGroupsAnUnreadableSourceApart() {
        var m = WFindModel()
        let sources = [WFindCard(id: "a", kind: .seenOnScreen, title: "a", timeMs: ms(13)).numbered(1), unreadable("u").numbered(2), gone("g").numbered(3)]
        m.exchanges = [WFindExchange(id: "e", question: "q", scope: "", body: .answered(text: "x [1] [2] [3]"), sources: sources)]
        let rows = WFindTimeline.rows(model: m, strings: .english, today: "Today", yesterday: "Yesterday")
        let headings = rows.compactMap { row -> String? in if case .day(_, let heading, _) = row { return heading } else { return nil } }
        #expect(headings.suffix(2) == [WFindStrings.english.noLongerOnDevice, WFindStrings.english.couldNotReadHeading])
    }
}

@Suite struct FlowLayoutProposalTests {
    @Test func aChildThatFitsKeepsItsIdealSize() {
        #expect(WFlowLayout.proposal(idealWidth: 100, container: 300).width == nil)
        #expect(WFlowLayout.proposal(idealWidth: 300, container: 300).width == nil)
        #expect(WFlowLayout.proposal(idealWidth: 100, container: nil).width == nil)
        #expect(WFlowLayout.proposal(idealWidth: 100, container: .infinity).width == nil)
        #expect(WFlowLayout.proposal(idealWidth: 100, container: 0).width == nil)
    }

    @Test func aChildWiderThanTheContainerIsProposedTheContainersWidth() {
        #expect(WFlowLayout.proposal(idealWidth: 900, container: 300).width == 300)
    }
}

/// What the old Ask page does with an answer: the whole text through one Markdown parse (`TimelineView.markdownAttributedString`, three lines
/// the app's tests hold equal to `WAnswerAttributed.markdown`). The blocks of the new page, parsed one by one, must read as the same words
/// wherever the old parse defines them.
@Suite struct OldPageEqualityTests {
    private func oldPage(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.failurePolicy = .returnPartiallyParsedIfPossible
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    /// The words of the blocks, joined the way the old parse joins its blocks (no separator): markers and code are drawn apart.
    private func newPage(_ text: String) -> String {
        WAnswerBlocks.parse(text).map { block -> String in
            switch block {
            case .paragraph(let words), .heading(let words), .item(_, let words, _): return String(WAnswerAttributed.markdown(words).characters)
            case .code(let words): return words
            case .table(let words): return words
            }
        }.joined()
    }

    @Test(arguments: [
        "line one\nline two",
        "line one\nline two\nline three\n\nnext paragraph",
        "a hard\nwrapped sentence that a model broke at eighty columns,\nand then went on.",
        "Intro\n\n- x\n- y\n  - nested\n- z",
        "1. first\n   - inner\n2. second",
        "```code``` and then words\nmore words",
        "Use ```a``` or `b` here.\nThen more.",
        "## Heading\ntext under it",
        "**bold** and\n*soft* breaks",
    ])
    func blocksReadAsTheOldPageReads(_ text: String) {
        #expect(newPage(text) == String(oldPage(text).characters), "\(text.debugDescription)")
    }

    @Test func aFencedBlockKeepsItsLinesLikeTheOldPage() {
        let text = "before\n\n```\nlet a = [1]\nlet b = 2\n```\n\nafter"
        let old = String(oldPage(text).characters).replacingOccurrences(of: "\n", with: "")
        #expect(newPage(text).replacingOccurrences(of: "\n", with: "") == old)
    }
}

@Suite struct AnswerBlockLayoutTests {
    @Test func aSingleLineTripleBacktickIsInlineCodeNotAFence() {
        #expect(WAnswerBlocks.parse("Run ```ls``` now.\n\nThen the rest.") == [.paragraph("Run ```ls``` now."), .paragraph("Then the rest.")])
        // Nothing after it is swallowed.
        #expect(WAnswerBlocks.parse("```code```\nstill an answer [1]\n- and a list") == [
            .paragraph("```code```\nstill an answer [1]"), .item(marker: "\u{2022}", text: "and a list", level: 0),
        ])
    }

    @Test func hardWrappedLinesAreOneParagraphWithSoftBreaks() {
        #expect(WAnswerBlocks.parse("first line\nsecond line\nthird line") == [.paragraph("first line\nsecond line\nthird line")])
        #expect(WAnswerBlocks.parse("one\ntwo\n\nthree") == [.paragraph("one\ntwo"), .paragraph("three")])
        #expect(String(WAnswerAttributed.markdown("first line\nsecond line").characters) == "first line second line")
    }

    @Test func aHeadingOrAnItemEndsAParagraph() {
        #expect(WAnswerBlocks.parse("para\n# Head\nmore") == [.paragraph("para"), .heading("Head"), .paragraph("more")])
        #expect(WAnswerBlocks.parse("para\n- item") == [.paragraph("para"), .item(marker: "\u{2022}", text: "item", level: 0)])
    }

    @Test func nestedListsKeepTheirLevels() {
        let text = "- a\n  - b\n    - c\n  - d\n- e\n1. one\n   1. inner"
        #expect(WAnswerBlocks.parse(text) == [
            .item(marker: "\u{2022}", text: "a", level: 0), .item(marker: "\u{2022}", text: "b", level: 1), .item(marker: "\u{2022}", text: "c", level: 2),
            .item(marker: "\u{2022}", text: "d", level: 1), .item(marker: "\u{2022}", text: "e", level: 0),
            .item(marker: "1.", text: "one", level: 0), .item(marker: "1.", text: "inner", level: 1),
        ])
    }

    @Test func aParagraphResetsTheListLevels() {
        #expect(WAnswerBlocks.parse("- a\n  - b\n\nplain\n  - c") == [
            .item(marker: "\u{2022}", text: "a", level: 0), .item(marker: "\u{2022}", text: "b", level: 1), .paragraph("plain"), .item(marker: "\u{2022}", text: "c", level: 0),
        ])
    }

    @Test func aFenceFollowsTheCoresGrammar() {
        // Tildes open one; a backtick fence's info string may not hold a backtick; four spaces of indent is not a fence.
        #expect(WAnswerBlocks.parse("~~~\nx [1]\n~~~\nafter") == [.code("x [1]"), .paragraph("after")])
        #expect(WAnswerBlocks.parse("```swift\nlet a\n```") == [.code("let a")])
        #expect(!WAnswerBlocks.parse("    ```\nnot code [1]\n    ```").contains { if case .code = $0 { return true } else { return false } })
        #expect(WAnswerBlocks.parse("   ```\nin\n   ```") == [.code("in")])
        // A closer must be the same character, at least as long, with only whitespace after it.
        #expect(WAnswerBlocks.parse("````\na\n```\nb\n````  \nc") == [.code("a\n```\nb"), .paragraph("c")])
        #expect(WAnswerBlocks.parse("```\na\n~~~\nb\n```") == [.code("a\n~~~\nb")])
        #expect(WAnswerBlocks.parse("```\na\n``` x\nb") == [.code("a\n``` x\nb")])
        // Unclosed, it runs to the end.
        #expect(WAnswerBlocks.parse("```\nrest\nof it") == [.code("rest\nof it")])
    }

    @Test func aTableIsOneAlignedMonospacedBlock() {
        let text = "Before.\n\n| Plan | Seats |\n|---|:-:|\n| Starter | 5 [1] |\n| Team | 25 |\n\nAfter."
        let blocks = WAnswerBlocks.parse(text)
        #expect(blocks.count == 3)
        #expect(blocks[0] == .paragraph("Before."))
        #expect(blocks[2] == .paragraph("After."))
        guard case .table(let words) = blocks[1] else { Issue.record("not a table"); return }
        #expect(words.split(separator: "\n").map(String.init) == [
            "Plan    | Seats", "------- | -----", "Starter | 5 [1]", "Team    | 25",
        ])
        // A mark inside a cell is still a mark.
        #expect(WAnswerAttributed.markNumbers(in: WAnswerAttributed.build(words, cited: [1], markdown: false)) == [1])
    }

    @Test func aRuleOrAPipeInProseIsNotATable() {
        #expect(WAnswerBlocks.parse("one | two\nsecond") == [.paragraph("one | two\nsecond")])
        #expect(WAnswerBlocks.parse("text\n---\nmore") == [.paragraph("text\n---\nmore")])
        #expect(WAnswerBlocks.isTableSeparator("|---|---|"))
        #expect(WAnswerBlocks.isTableSeparator("--- | :---:"))
        #expect(!WAnswerBlocks.isTableSeparator("---"))
        #expect(!WAnswerBlocks.isTableSeparator("| a | b |"))
    }

    @Test func escapedPipesStayInTheirCell() {
        #expect(WAnswerBlocks.cells(of: "| a \\| b | c |") == ["a | b", "c"])
    }
}

struct FindDefaultOrderTests {
    @Test func aQuestionOpensOnBestMatchesFirst() {
        #expect(WFindOrder.default(for: "Where is the kite festival?") == .bestFirst)
        #expect(WFindOrder.default(for: "who owns the budget") == .bestFirst)
        #expect(WFindOrder.default(for: "budget approved?") == .bestFirst)
        #expect(WFindOrder.default(for: "¿Dónde es el festival") == .bestFirst)
        #expect(WFindOrder.default(for: "Wann ist das Treffen") == .bestFirst)
    }

    @Test func plainWordsOpenByDay() {
        #expect(WFindOrder.default(for: "kite festival") == .byDay)
        #expect(WFindOrder.default(for: "whoever") == .byDay)
        #expect(WFindOrder.default(for: "  ") == .byDay)
    }
}
