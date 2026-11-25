import Foundation
import Testing
@testable import WhimbrelUI

@Suite struct SettingsMemoryTests {
    private func asks() -> [WRetentionAsk] {
        let kinds: [(WRetentionEffect, WRetentionKind?)] = [
            (.deletes, .screenText), (.deletes, .meetings), (.deletes, .conversations), (.deletes, .deleteAudio), (.recompressesAudio, nil), (.shrinksImages, nil),
        ]
        return kinds.flatMap { effect, kind in
            [WRetentionAsk(effect: effect, kind: kind, name: "Name", days: 7, replacing: .forever, count: nil),
             WRetentionAsk(effect: effect, kind: kind, name: "Name", days: 7, replacing: .days(30), count: 12)]
        }
    }

    @Test func everyRetentionQuestionSaysItCannotBeUndoneAndWhatHappens() {
        let l = WMemoryPrivacyLabels()
        for ask in asks() {
            let message = l.askMessage(ask)
            #expect(message.contains("can't be undone"), "\(ask)")
            #expect(message.contains("7 days"), "\(ask)")
        }
        let audio = WRetentionAsk(effect: .recompressesAudio, kind: nil, name: "x", days: 7, replacing: .days(30))
        #expect(l.askMessage(audio).contains("lower quality"))
        let images = WRetentionAsk(effect: .shrinksImages, kind: nil, name: "x", days: 7, replacing: .forever)
        #expect(l.askMessage(images).contains("made smaller"))
        // The destructive button says what it does.
        let meetings = WRetentionAsk(effect: .deletes, kind: .meetings, name: "x", days: 7, replacing: .days(30))
        #expect(l.askConfirm(meetings) == "Keep 7 days and delete older")
        #expect(l.askMessage(meetings).contains("Meetings older than 7 days are deleted"))
        // Without a count the question still says what will be deleted.
        let conversations = WRetentionAsk(effect: .deletes, kind: .conversations, name: "x", days: 7, replacing: .forever)
        #expect(l.askMessage(conversations).contains("Conversations older than 7 days are deleted"))
        let deleteAudio = WRetentionAsk(effect: .deletes, kind: .deleteAudio, name: "x", days: 7, replacing: .forever)
        #expect(l.askMessage(deleteAudio).contains("The audio of meetings older than 7 days is deleted"))
    }

    /// The row's own name ("Delete audio after") is a label, not a noun: no sentence is built from it.
    @Test func everyRetentionRowAsksInItsOwnWordsNotWithTheRowLabel() {
        let l = WMemoryPrivacyLabels()
        let rows: [(String, WRetentionEffect, WRetentionKind?, String, String)] = [
            ("Screen text", .deletes, .screenText, "Delete screen text older than 7 days?", "Screen text is kept for 30 days today."),
            ("Meetings and recordings", .deletes, .meetings, "Delete meetings and recordings older than 7 days?", "Meetings and recordings are kept for 30 days today."),
            ("Conversations", .deletes, .conversations, "Delete conversations older than 7 days?", "Conversations are kept for 30 days today."),
            ("Delete audio after", .deletes, .deleteAudio, "Delete audio from recordings older than 7 days?", "Audio from recordings is kept for 30 days today."),
            ("Compress audio after", .recompressesAudio, nil, "Compress audio older than 7 days?", "Audio is compressed after 30 days today."),
            ("Shrink images after", .shrinksImages, nil, "Shrink images older than 7 days?", "Images are shrunk after 30 days today."),
        ]
        for (label, effect, kind, title, today) in rows {
            let ask = WRetentionAsk(effect: effect, kind: kind, name: label, days: 7, replacing: .days(30), count: nil)
            #expect(l.askTitle(ask) == title)
            #expect(l.askMessage(ask).hasPrefix(today), "\(l.askMessage(ask))")
            // A label like "Delete audio after" is not a noun; it must never appear inside a sentence.
            if label.hasSuffix("after") {
                #expect(!l.askTitle(ask).contains(label) && !l.askMessage(ask).contains(label), "\(label)")
            }
            let forever = WRetentionAsk(effect: effect, kind: kind, name: label, days: 7, replacing: .forever, count: nil)
            if label.hasSuffix("after") { #expect(!l.askMessage(forever).contains(label), "\(label)") }
        }
        let deleteAudioForever = WRetentionAsk(effect: .deletes, kind: .deleteAudio, name: "Delete audio after", days: 7, replacing: .forever, count: nil)
        #expect(l.askMessage(deleteAudioForever).hasPrefix("Audio from recordings is kept forever today."))
    }

    /// Each clause of the Memory sentence is what the capture code does (ScreenTextCaptureService): the text of the focused
    /// window with its title, app and page address; a picture is read only when the window gives under 8 characters, and only
    /// its text is kept; apps are skipped always and sites only where the browser shows the page address.
    @Test func theMemorySentenceSaysOnlyWhatCaptureDoes() {
        let text = WMemoryPrivacyLabels().memoryExplainer
        #expect(!text.contains("has no text"), "OCR also runs when there is a little text")
        #expect(text.contains("title") && text.contains("page address"))
        #expect(text.contains("almost no text") && text.contains("keeps only the text"))
        #expect(text.contains("in browsers that show Whimbrel the page address"))
        #expect(!text.contains("does not save anything from the apps and sites"))
    }

    @Test func shorteningCompressOrShrinkAsksFirstAndLengtheningDoesNot() {
        // The same decision type drives every row, so compress and shrink ask exactly like delete.
        let limited = WRetentionStored(days: 30, keepForever: false)
        #expect(WRetentionDecision.commit(stored: limited, proposed: 7) == .askFirst(days: 7, replacing: .days(30)))
        #expect(WRetentionDecision.commit(stored: limited, proposed: 60) == .save(days: 60))
        let full = WRetentionStored(days: 30, keepForever: true)
        #expect(WRetentionDecision.commit(stored: full, proposed: 30) == .askFirst(days: 30, replacing: .forever))
    }

    @Test func theCompanionSavesKeepingMoreAtOnceAndNeverSavesTurningItOff() {
        let limited = WRetentionStored(days: 30, keepForever: false)
        let full = WRetentionStored(days: 45, keepForever: true)
        #expect(WRetentionDecision.companion(stored: limited, keepForever: true) == .saveKeepForever)
        #expect(WRetentionDecision.companion(stored: full, keepForever: true) == .nothingToSave)
        #expect(WRetentionDecision.companion(stored: full, keepForever: false) == .choosePeriod(showing: 45))
        #expect(WRetentionDecision.companion(stored: WRetentionStored(days: 99999, keepForever: true), keepForever: false) == .choosePeriod(showing: 3650))
        #expect(WRetentionDecision.companion(stored: limited, keepForever: false) == .nothingToSave)
    }

    @Test func applyRetentionNowHasNothingToDeleteWhenEveryRowKeepsForever() {
        let forever = WRetentionStored(days: 30, keepForever: true)
        let limited = WRetentionStored(days: 30, keepForever: false)
        #expect(!WRetentionApplyPlan.hasLimit([forever, forever, forever]))
        #expect(WRetentionApplyPlan.hasLimit([forever, limited, forever]))
        #expect(!WRetentionApplyPlan.hasLimit([]))
    }

    @Test func aShorterPeriodAsksFirstAndALongerOneDoesNot() {
        let stored = WRetentionStored(days: 90, keepForever: false)
        #expect(WRetentionDecision.commit(stored: stored, proposed: 30) == .askFirst(days: 30, replacing: .days(90)))
        #expect(WRetentionDecision.commit(stored: stored, proposed: 91) == .save(days: 91))
        #expect(WRetentionDecision.commit(stored: stored, proposed: 90) == .nothingToSave)
    }

    @Test func leavingKeepForeverAlwaysAsksBecauseItStartsDeleting() {
        let stored = WRetentionStored(days: 30, keepForever: true)
        #expect(WRetentionDecision.commit(stored: stored, proposed: 30) == .askFirst(days: 30, replacing: .forever))
        #expect(WRetentionDecision.commit(stored: stored, proposed: 3650) == .askFirst(days: 3650, replacing: .forever))
    }

    @Test func aBadNumberSavesNothingAndSaysWhy() {
        let stored = WRetentionStored(days: 30, keepForever: false)
        #expect(WRetentionDecision.commit(stored: stored, proposed: nil) == .refuse(.empty))
        #expect(WRetentionDecision.commit(stored: stored, proposed: 0) == .refuse(.belowMinimum(1)))
        #expect(WRetentionDecision.commit(stored: stored, proposed: -5) == .refuse(.belowMinimum(1)))
        #expect(WRetentionDecision.commit(stored: stored, proposed: 3651) == .refuse(.aboveMaximum(3650)))
        #expect(WRetentionDecision.validate(1) == nil)
        #expect(WRetentionDecision.validate(3650) == nil)
    }

    @Test func turningKeepForeverOffShowsTheStoredNumberClamped() {
        #expect(WRetentionDecision.numberToShow(stored: WRetentionStored(days: 45, keepForever: true)) == 45)
        #expect(WRetentionDecision.numberToShow(stored: WRetentionStored(days: 99999, keepForever: true)) == 3650)
        #expect(WRetentionDecision.storedProblem(WRetentionStored(days: 0, keepForever: false)) == .belowMinimum(1))
        #expect(WRetentionDecision.storedProblem(WRetentionStored(days: 30, keepForever: false)) == nil)
    }

    @Test func theMemoryControlsFollowTheState() {
        #expect(WMemoryControls.controls(for: .on, storeOpen: true) == WMemoryControls(showsPause: true, toggle: .turnOff, isEnabled: true))
        #expect(WMemoryControls.controls(for: .off, storeOpen: true).toggle == .turnOn)
        #expect(WMemoryControls.controls(for: .pausedUntil("3:10 PM"), storeOpen: true).toggle == nil)
        #expect(!WMemoryControls.controls(for: .onNotCapturing, storeOpen: false).isEnabled)
    }

    @Test func theAppListFiltersByNameIgnoringCase() {
        let apps = [WExcludedApp(id: "a", name: "Notes"), WExcludedApp(id: "b", name: "Mail")]
        #expect(WExcludedAppsFilter.filter(apps, search: "") == .apps(apps))
        #expect(WExcludedAppsFilter.filter(apps, search: "NOT") == .apps([apps[0]]))
        #expect(WExcludedAppsFilter.filter(apps, search: "zzz") == .noMatches)
        #expect(WExcludedAppsFilter.filter([], search: "") == .noneRunning)
    }

    @Test func theSectionHasTwelveDistinctAnchorsInTheGallery() async {
        await MainActor.run {
            let all = WSettingsWindowGallery.anchorsMemory.all
            #expect(all.count == 12)
            #expect(Set(all).count == 12)
            #expect(WSettingsWindowGallery.memoryPrivacyCases().count > 25)
        }
    }

    @Test func noSampleSentenceHasAnEmOrEnDash() {
        let l = WMemoryPrivacyLabels()
        let all = [l.memoryExplainer, l.showScreenCapturesCaption, l.neverIntro, l.keepingIntro, l.deleteRowHelp, l.applyConfirmMessage([.init(kind: .screenText, days: 30)])]
            + asks().flatMap { [l.askTitle($0), l.askMessage($0), l.askConfirm($0)] }
        for text in all { #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}")) }
    }

    // MARK: Plain counts, plurals and the Apply Retention Now sentences

    @Test func aDayCountAgreesInOneAndInThirty() {
        let l = WMemoryPrivacyLabels()
        #expect(l.days(1) == "1 day")
        #expect(l.days(30) == "30 days")
        #expect(l.savePeriod(1) == "Keep for 1 day")
        #expect(l.savePeriod(30) == "Keep for 30 days")
        #expect(l.periodProblem(.belowMinimum(1)) == "Use 1 day or more. Nothing was changed.")
        #expect(l.storedOutOfRange(1).hasPrefix("The saved period is 1 day,"))
        for days in [1, 30] {
            let ask = WRetentionAsk(effect: .deletes, kind: .screenText, name: "Screen text", days: days, replacing: .days(1), count: nil)
            let word = days == 1 ? "1 day" : "30 days"
            #expect(l.askTitle(ask) == "Delete screen text older than \(word)?")
            #expect(l.askConfirm(ask) == "Keep \(word) and delete older")
            #expect(l.askMessage(ask).hasPrefix("Screen text is kept for 1 day today. Screen text older than \(word) is deleted"))
        }
        for text in asks().flatMap({ [l.askTitle($0), l.askMessage($0), l.askConfirm($0)] }) { #expect(!text.contains("1 days")) }
    }

    @Test func theCountSentenceNamesTheThingAndFormatsTheNumber() {
        let l = WMemoryPrivacyLabels()
        func message(_ kind: WRetentionKind, _ count: Int64, days: Int) -> String {
            l.askMessage(WRetentionAsk(effect: .deletes, kind: kind, name: "", days: days, replacing: .forever, count: count))
        }
        #expect(message(.screenText, 3904, days: 30).hasPrefix("\(Int64(3904).formatted()) screen captures are older than 30 days. "))
        #expect(message(.screenText, 1, days: 1).hasPrefix("1 screen capture is older than 1 day. "))
        #expect(message(.meetings, 2, days: 30).hasPrefix("2 meetings are older than 30 days. "))
        #expect(message(.conversations, 1, days: 30).hasPrefix("1 conversation is older than 30 days. "))
        #expect(message(.deleteAudio, 5, days: 30).hasPrefix("5 recordings are older than 30 days. "))
        #expect(!message(.screenText, 3904, days: 30).contains("of them"))
    }

    @Test func applyRetentionNowSaysWhatEachKindWithAPeriodDeletes() {
        let l = WMemoryPrivacyLabels()
        let text = l.applyConfirmMessage([.init(kind: .meetings, days: 90), .init(kind: .screenText, days: 1)])
        #expect(text == "Meetings and recordings older than 90 days are deleted.\nScreen text older than 1 day is deleted.\nThis can't be undone.")
        #expect(!text.contains("This deletes") && !text.contains(": older than"))
        #expect(l.applyConfirmMessage([.init(kind: .conversations, days: 30)]).contains("Conversations older than 30 days are deleted, except starred ones."))
        #expect(l.applyConfirmMessage([]) == "Every period is set to Keep forever, so nothing will be deleted.")
    }
}
