import Foundation
import Testing
@testable import WhimbrelUI

@Suite struct SettingsWindowTests {
    @Test func theListStaysAtTheMinimumWindowWidthAtNormalTextSize() {
        #expect(!WSettingsWindowLayout.usesPicker(width: 640, textScale: 1))
        #expect(!WSettingsWindowLayout.usesPicker(width: 900, textScale: 1))
    }

    @Test func aNarrowWindowUsesThePicker() {
        #expect(WSettingsWindowLayout.usesPicker(width: 520, textScale: 1))
        #expect(WSettingsWindowLayout.usesPicker(width: 639, textScale: 1))
    }

    @Test func largeTextUsesThePickerEvenInAWideWindow() {
        #expect(WSettingsWindowLayout.usesPicker(width: 900, textScale: 2))
        #expect(WSettingsWindowLayout.usesPicker(width: 1400, textScale: 2))
        #expect(WSettingsWindowLayout.usesPicker(width: 1400, textScale: 1.5))
        #expect(!WSettingsWindowLayout.usesPicker(width: 900, textScale: 1.25))
        #expect(WSettingsWindowLayout.usesPicker(width: 680, textScale: 1.25))
    }

    @Test func aTextScaleBelowOneDoesNotShrinkTheColumn() {
        #expect(WSettingsWindowLayout.usesPicker(width: 639, textScale: 0.5))
    }

    @Test func theGalleryCasesHaveUniqueIdsAndTheirViewsBuild() async {
        await MainActor.run {
            let cases = WSettingsWindowGallery.cases()
            #expect(Set(cases.map(\.id)).count == cases.count)
            #expect(cases.count > 20)
        }
    }

    @Test func theSampleAnchorsAreAllDistinct() async {
        await MainActor.run {
            let all = WSettingsWindowGallery.anchorsGeneral.all + WSettingsWindowGallery.anchorsProfile.all + WSettingsWindowGallery.anchorsPlan.all
            #expect(Set(all).count == all.count)
            #expect(all.count == 13)
        }
    }

    // MARK: Search hit hand-off

    @Test func aTargetWaitsForTheDetailColumnAndIsClearedOnlyByItsOwnScroll() {
        #expect(WSettingsHandoff.ready(target: "a", detailMounted: true) == "a")
        #expect(WSettingsHandoff.ready(target: "a", detailMounted: false) == nil)
        #expect(WSettingsHandoff.ready(target: nil, detailMounted: true) == nil)
        // The scroll to "a" finished: the target is cleared, so choosing "a" again is a change from nil to "a" and runs again.
        #expect(WSettingsHandoff.afterScroll(target: "a", done: "a") == nil)
        // A newer choice replaced it while the first scroll was running: it is kept.
        #expect(WSettingsHandoff.afterScroll(target: "b", done: "a") == "b")
        #expect(WSettingsHandoff.afterScroll(target: nil, done: "a") == nil)
    }

    @Test func theSameHitTwiceIsTwoHandOffs() {
        // The sequence the window sees as the target value, with a task started for each change to a non-nil value.
        var target: String? = nil
        var runs: [String] = []
        for _ in 0..<2 {
            target = "capture.retention"
            if let id = WSettingsHandoff.ready(target: target, detailMounted: true) { runs.append(id) }
            target = WSettingsHandoff.afterScroll(target: target, done: "capture.retention")
            #expect(target == nil)
        }
        #expect(runs == ["capture.retention", "capture.retention"])
    }

    @Test func returnInTheSearchFieldOpensTheFirstResultOrNothing() {
        let results = [
            WSettingsSearchResult(id: "plan.manage", title: "Manage subscription", section: "Plan & billing"),
            WSettingsSearchResult(id: "plan.summary", title: "Plan & billing", section: "Plan & billing"),
        ]
        #expect(WSettingsHandoff.firstResult(results)?.id == "plan.manage")
        #expect(WSettingsHandoff.firstResult([]) == nil)
    }

    @Test func aFoundRowIsAnnouncedAndAMissingOneSaysItIsNotShown() {
        let found = WJumpNotice.after(found: true, title: "Manage subscription", shownFormat: "Showing %@.", hiddenText: "That setting isn't shown right now.")
        #expect(found == .shown("Showing Manage subscription."))
        let missing = WJumpNotice.after(found: false, title: "Manage subscription", shownFormat: "Showing %@.", hiddenText: "That setting isn't shown right now.")
        #expect(missing == .hidden("That setting isn't shown right now."))
        #expect(![found.text, missing.text].contains { $0.contains("\u{2014}") || $0.contains("\u{2013}") })
    }

    @MainActor @Test func anAnchorIsPresentWhileAnyRowReportsItself() async {
        let presence = WAnchorPresence()
        #expect(!presence.contains("plan.manage"))
        presence.appeared("plan.manage")
        #expect(presence.contains("plan.manage"))
        // A replacement row appears before the old one goes: the anchor stays present.
        presence.appeared("plan.manage")
        presence.disappeared("plan.manage")
        #expect(presence.contains("plan.manage"))
        presence.disappeared("plan.manage")
        #expect(!presence.contains("plan.manage"))
        // Never drawn: waiting gives up and says so.
        #expect(await presence.waitFor("plan.manage", seconds: 0.2) == false)
        // Drawn while waiting: found.
        Task { try? await Task.sleep(nanoseconds: 150_000_000); presence.appeared("plan.manage") }
        #expect(await presence.waitFor("plan.manage", seconds: 1) == true)
    }

    @Test func aHitInsideACollapsedGroupOpensTheGroupFirst() {
        #expect(WSettingsHandoff.opensGroup(["connectors.ownClient"], for: "connectors.ownClient"))
        #expect(!WSettingsHandoff.opensGroup(["connectors.ownClient"], for: "connectors.mcp"))
        #expect(!WSettingsHandoff.opensGroup(["connectors.ownClient"], for: nil))
    }
}
