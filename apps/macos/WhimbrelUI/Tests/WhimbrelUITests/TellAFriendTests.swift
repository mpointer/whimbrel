import Foundation
import Testing
@testable import WhimbrelUI

/// Tell a friend (rollout row 1 of docs/superpowers/plans/2026-11-24-referral-and-share.md): the message a person copies, the
/// "Copied" flash, and the words. A plain link only: no code, no reward, nothing sent.
@Suite struct TellAFriendTests {
    private let english = "I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: %@"

    @Test func theMessagePutsTheLinkWhereTheTemplateSaysAndNowhereElse() {
        let text = WTellAFriend.message(template: english)
        #expect(text == "I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: https://whimbrelapp.com/?utm_source=share")
        #expect(text.components(separatedBy: WTellAFriend.link).count == 2)
        #expect(!text.contains("%@"))
    }

    @Test func theLinkIsThePlainHomePageWithNoCodeAndNoPerPersonValue() {
        let url = URL(string: WTellAFriend.link)
        #expect(url?.scheme == "https" && url?.host == "whimbrelapp.com")
        #expect(url?.path == "/" || url?.path == "")
        #expect(URLComponents(url: url!, resolvingAgainstBaseURL: false)?.queryItems == [URLQueryItem(name: "utm_source", value: "share")])
    }

    @Test func aTemplateWithTheLinkInTheMiddleKeepsItsWordsAround() {
        #expect(WTellAFriend.message(template: "Mira %@ ahora.", link: "L") == "Mira L ahora.")
    }

    @Test func aDamagedTemplateStillGivesOneLinkAtTheEnd() {
        // No placeholder: the words, then the link.
        #expect(WTellAFriend.message(template: "I use Whimbrel.", link: "L") == "I use Whimbrel. L")
        // Two placeholders: neither is filled, and the link is added once.
        #expect(WTellAFriend.message(template: "A %@ B %@", link: "L") == "A  B L")
        // Nothing at all: just the link.
        #expect(WTellAFriend.message(template: "", link: "L") == "L")
        #expect(WTellAFriend.message(template: "%@", link: "L") == "L")
    }

    @Test func theMessageHasNoDashesAndNoPromiseOfAReward() {
        let text = WTellAFriend.message(template: english)
        #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"))
        for word in ["code", "reward", "discount", "free month"] { #expect(!text.lowercased().contains(word), "\(word)") }
    }

    @Test func copiedShowsOnlyAfterACopyThatWorkedAndEachCopyRestartsTheTimer() {
        var flash = WCopyFlash()
        #expect(!flash.copied && !flash.failed)
        flash.finish(succeeded: true)
        #expect(flash.copied && !flash.failed)
        let first = flash.generation
        flash.finish(succeeded: true)
        #expect(flash.generation == first + 1)
        flash.reset()
        #expect(!flash.copied)
        flash.finish(succeeded: false)
        #expect(!flash.copied && flash.failed)
    }

    @Test func theCardWordsUseNoDashes() {
        for text in WTellAFriendLabels().all + [WStrings.english.menu.tellAFriend, WStrings.english.menu.tellAFriendCopied, WStrings.english.menu.tellAFriendHint] {
            #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"), "dash in \(text)")
        }
    }

    @Test func theRowIsFocusableAndSitsBetweenAskAndOpen() {
        let order = WMenuModel().focusOrder
        let ask = order.firstIndex(of: WMenuFocusID.ask)!
        #expect(order[ask + 1] == WMenuFocusID.tellAFriend)
        #expect(order[ask + 2] == WMenuFocusID.open)
        // It needs no account and no store, so nothing switches it off.
        #expect(WMenuModel(disabled: [WMenuFocusID.memory, WMenuFocusID.start]).focusOrder.contains(WMenuFocusID.tellAFriend))
    }

    @Test func theActionReportsWhetherTheCopyWorked() {
        var copies = 0
        let actions = WMenuActions(tellAFriend: { copies += 1; return true })
        #expect(actions.tellAFriend())
        #expect(copies == 1)
        // The default does nothing harmful and says it worked, so a menu built without the app still draws.
        #expect(WMenuActions().tellAFriend())
    }
}
