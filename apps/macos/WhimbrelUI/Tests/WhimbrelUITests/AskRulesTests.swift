import Foundation
import Testing
@testable import WhimbrelUI

@Suite struct AskRulesTests {
    // MARK: The title of a conversation kept on this computer

    @Test func theTitleIsAskedAsRoutedUnlessTheAnswerWasKeptLocal() {
        #expect(WAskTitleRoute.decide(keepLocal: false, localModelAvailable: false) == .normal)
        #expect(WAskTitleRoute.decide(keepLocal: false, localModelAvailable: true) == .normal)
    }

    /// A title kept on this computer, or skipped, is made with no hosted key and no Whimbrel sign-in: none is even read.
    @Test func onlyTheAccountsOwnTitleRouteReadsCredentials() {
        #expect(WAskTitleRoute.normal.readsCredentials)
        #expect(!WAskTitleRoute.localOnly.readsCredentials)
        #expect(!WAskTitleRoute.skip.readsCredentials)
        #expect(!WAskTitleRoute.decide(keepLocal: true, localModelAvailable: true).readsCredentials)
        #expect(!WAskTitleRoute.decide(keepLocal: true, localModelAvailable: false).readsCredentials)
    }

    @Test func aLocalAnswersTitleStaysLocalOrIsSkipped() {
        #expect(WAskTitleRoute.decide(keepLocal: true, localModelAvailable: true) == .localOnly)
        #expect(WAskTitleRoute.decide(keepLocal: true, localModelAvailable: false) == .skip)
    }

    // MARK: What a failed ask held to this computer says

    @Test func aLocalOnlyAskWithNoLocalModelSaysNoModelHereCouldAnswer() {
        #expect(WAskLocalFailure.saysNoLocalModel(requireLocal: true, isProviderFailure: true, localModelAvailable: false))
    }

    @Test func aLocalModelThatFailedKeepsTheFailureWords() {
        // A model here was there and failed (Ollama stopped, the model crashed partway): not "no model could answer".
        #expect(!WAskLocalFailure.saysNoLocalModel(requireLocal: true, isProviderFailure: true, localModelAvailable: true))
    }

    @Test func otherFailuresKeepTheirOwnWords() {
        // Not held to this computer: core's provider failure is what it says.
        #expect(!WAskLocalFailure.saysNoLocalModel(requireLocal: false, isProviderFailure: true, localModelAvailable: false))
        // A spent budget, an unreadable exclusion setting and the like say their own thing.
        #expect(!WAskLocalFailure.saysNoLocalModel(requireLocal: true, isProviderFailure: false, localModelAvailable: false))
    }

    // MARK: Stop

    @Test func stopWorksWhileTheAnswerIsPreparedOrStreams() {
        #expect(WAskStop.mayStop(.preparing))
        #expect(WAskStop.mayStop(.streaming))
    }

    @Test func aFinishedAnswerBeingSavedCannotBeStopped() {
        // The stream is over and core has the answer: Stop then would drop a finished answer.
        #expect(!WAskStop.mayStop(.finalizing))
        // The blocking call has no Stop, as before.
        #expect(!WAskStop.mayStop(.blocking))
    }

    // MARK: Starting a question

    @Test func aQuestionStartsOnlyWithWordsAndWhenFree() {
        #expect(WAskStart.question("  What did Sam say?\n", busy: false) == "What did Sam say?")
        #expect(WAskStart.question("   \n ", busy: false) == nil)
        #expect(WAskStart.question("", busy: false) == nil)
        #expect(WAskStart.question("Hello", busy: true) == nil)
    }

    // MARK: Opening a saved conversation from Find's list

    @Test func onlyAnEverythingConversationOpensOnFind() {
        #expect(WFindConversationOpen.destination(.everything) == .find)
        // A topic or meeting conversation keeps its scope on the Conversations page; Find would ask it over everything.
        #expect(WFindConversationOpen.destination(.topic) == .conversationsPage)
        #expect(WFindConversationOpen.destination(.meeting) == .conversationsPage)
    }

    // MARK: The filters a Find conversation keeps

    private var utc: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }

    @Test func datesAndAppsAreKeptAsAskedAndAPersonNever() {
        let now = Date(timeIntervalSince1970: 1_763_726_400) // 2025-11-21 12:00 UTC
        let saved = WSavedFindFilter(WHomeFilter(appId: "com.tinyspeck.slackmacgap", dates: .today, person: "Sam Okafor"), now: now, calendar: utc)
        let day = WHomeDateRange.today.bounds(now: now, calendar: utc)
        #expect(saved.startUnixMs == Int64(day.start.timeIntervalSince1970 * 1000))
        #expect(saved.endUnixMs == Int64(day.end.timeIntervalSince1970 * 1000))
        #expect(saved.appBundleId == "com.tinyspeck.slackmacgap")
        #expect(saved.peopleLeftOut)
        // Reopened: the same app and the same days (as asked, not "today" again), and no person.
        let back = saved.homeFilter(title: "Nov 21")
        #expect(back == WHomeFilter(appId: "com.tinyspeck.slackmacgap", dates: .exact(start: day.start, end: day.end, title: "Nov 21")))
    }

    @Test func noFiltersKeepNothing() {
        let saved = WSavedFindFilter(WHomeFilter(), now: Date(), calendar: utc)
        #expect(saved.isEmpty)
        #expect(saved.homeFilter(title: "x") == WHomeFilter())
        // A People filter alone keeps only that it was left out.
        let person = WSavedFindFilter(WHomeFilter(person: "Sam"), now: Date(), calendar: utc)
        #expect(!person.isEmpty)
        #expect(person.peopleLeftOut)
        #expect(person.homeFilter(title: "x") == WHomeFilter())
    }

    @Test func aRangeWithOneSideMissingIsNotADateFilter() {
        let saved = WSavedFindFilter(startUnixMs: 1000, endUnixMs: nil, appBundleId: nil, peopleLeftOut: false)
        #expect(saved.homeFilter(title: "x").dates == nil)
    }

    // MARK: A reopened conversation's chips are not a filter change

    /// Find restores a kept filter as an exact range (`WSavedFindFilter.homeFilter`), pushes it to the shell (`WShellPush`), and the
    /// shell echoes the chips it holds back (`onFilterChange`); the page then asks `WFindConversationFilter.startsNewConversation`.
    @Test func reopeningWithKeptFiltersIsNotAFilterChange() {
        let now = Date(timeIntervalSince1970: 1_763_726_400)
        let asked: [WHomeFilter] = [
            WHomeFilter(appId: "com.tinyspeck.slackmacgap", dates: .exact(start: Date(timeIntervalSince1970: 1_763_000_000), end: Date(timeIntervalSince1970: 1_763_300_000), title: "Nov 13 to Nov 16")),
            WHomeFilter(appId: "com.tinyspeck.slackmacgap", dates: .today),
            WHomeFilter(appId: "com.apple.mail", dates: .yesterday),
            WHomeFilter(dates: .lastSevenDays),
            WHomeFilter(appId: "com.apple.mail", dates: .custom(from: Date(timeIntervalSince1970: 1_762_000_000), to: Date(timeIntervalSince1970: 1_762_500_000))),
            WHomeFilter(appId: "com.apple.mail"),
            WHomeFilter(appId: "com.apple.mail", person: "Sam"),
        ]
        for filter in asked {
            let restored = WSavedFindFilter(filter, now: now, calendar: utc).homeFilter(title: "Nov 21")
            let echoed = WShellPush(token: 2, query: "", filter: restored).filter
            #expect(!WFindConversationFilter.startsNewConversation(old: restored, new: echoed, hasExchanges: true, now: now, calendar: utc))
            // The kept days and app are the ones asked under: the same filter, read at the same moment, less the person.
            var unpersoned = filter
            unpersoned.person = nil
            #expect(WFindConversationFilter.same(restored, unpersoned, now: now, calendar: utc))
        }
    }

    /// A relative preset is the same chip after midnight: "Today" still says Today, so the conversation goes on.
    @Test func aRelativePresetIsTheSameFilterAcrossMidnight() {
        let beforeMidnight = Date(timeIntervalSince1970: 1_763_769_540) // 2025-11-21 23:59 UTC
        let afterMidnight = Date(timeIntervalSince1970: 1_763_769_660) // 2025-11-22 00:01 UTC
        for preset in [WHomeDateRange.today, .yesterday, .lastSevenDays] {
            let f = WHomeFilter(appId: "a", dates: preset)
            #expect(WFindConversationFilter.same(f, f, now: beforeMidnight, calendar: utc))
            #expect(!WFindConversationFilter.startsNewConversation(old: f, new: f, hasExchanges: true, now: afterMidnight, calendar: utc))
        }
        // Two different presets differ even where their days overlap.
        #expect(!WFindConversationFilter.same(WHomeFilter(dates: .today), WHomeFilter(dates: .lastSevenDays), now: afterMidnight, calendar: utc))
        // An exact range is compared by its days: the kept range of yesterday's "Today" is not today's "Today".
        let kept = WSavedFindFilter(WHomeFilter(dates: .today), now: beforeMidnight, calendar: utc).homeFilter(title: "Nov 21")
        #expect(WFindConversationFilter.same(kept, WHomeFilter(dates: .today), now: beforeMidnight, calendar: utc))
        #expect(!WFindConversationFilter.same(kept, WHomeFilter(dates: .today), now: afterMidnight, calendar: utc))
    }

    @Test func aRealChangeStillStartsANewConversation() {
        let now = Date(timeIntervalSince1970: 1_763_726_400)
        let today = WHomeFilter(appId: "a", dates: .today)
        #expect(WFindConversationFilter.startsNewConversation(old: today, new: WHomeFilter(appId: "b", dates: .today), hasExchanges: true, now: now, calendar: utc))
        #expect(WFindConversationFilter.startsNewConversation(old: today, new: WHomeFilter(appId: "a", dates: .yesterday), hasExchanges: true, now: now, calendar: utc))
        #expect(WFindConversationFilter.startsNewConversation(old: today, new: WHomeFilter(appId: "a", dates: .today, person: "Sam"), hasExchanges: true, now: now, calendar: utc))
        #expect(WFindConversationFilter.startsNewConversation(old: today, new: WHomeFilter(), hasExchanges: true, now: now, calendar: utc))
        // Nothing asked yet: never a new conversation.
        #expect(!WFindConversationFilter.startsNewConversation(old: today, new: WHomeFilter(), hasExchanges: false, now: now, calendar: utc))
    }
}
