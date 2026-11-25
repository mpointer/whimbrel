import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

@Suite struct MenuTests {
    private let strings = WStrings.english

    // MARK: Recording section

    @Test func aRecordingOffersPauseAndStop() {
        let r = WMenuRecording(state: .recording, elapsedSeconds: 10, title: "Pricing review")
        #expect(r.showsPauseButton && r.offersStop && !r.offersResume)
    }

    @Test func aPausedRecordingOffersResumeAndStop() {
        for state in [WRecordingState.paused, .resuming] {
            let r = WMenuRecording(state: state)
            #expect(r.showsPauseButton && r.offersStop && r.offersResume)
        }
    }

    @Test func aProblemKeepsStopOnlyAndSaysTheReasonInWords() {
        let r = WMenuRecording(state: .problem("Microphone not found"))
        // Resume only runs from a pause, so a problem offers Stop alone, as the live meeting screen does.
        #expect(!r.showsPauseButton)
        #expect(r.offersStop)
        #expect(r.problemReason == "Microphone not found")
        #expect(WMenuRecording(state: .problem("")).problemReason == nil)
        #expect(WMenuRecording(state: .recording).problemReason == nil)
    }

    @Test func aLinkActionSaysItIsALinkToAScreenReader() {
        #expect(WMenuAction(title: "Backup help", isLink: true) {}.isLink)
        #expect(!WMenuAction(title: "Dismiss") {}.isLink)
    }

    @Test func processingHasNoButtons() {
        let r = WMenuRecording(state: .processing)
        #expect(!r.showsPauseButton && !r.offersStop)
    }

    @Test func everyStateSaysInWordsThatARecordingIsOpen() {
        #expect(WMenuText.stateWord(.recording, strings: strings) == "Recording")
        #expect(WMenuText.stateWord(.paused, strings: strings) == "Recording paused")
        #expect(WMenuText.stateWord(.problem("x"), strings: strings) == "Recording problem")
        #expect(WMenuText.stateWord(.problem(""), strings: strings) == "Recording problem")
        #expect(WMenuText.stateWord(.starting, strings: strings) == "Starting recording")
        #expect(WMenuText.stateWord(.resuming, strings: strings) == "Resuming recording")
        // Never the bare words a memory pause or a problem elsewhere would also use.
        for state in [WRecordingState.recording, .paused, .problem("x"), .starting, .resuming] {
            let word = WMenuText.stateWord(state, strings: strings)
            #expect(word != "Paused" && word != "Problem")
            #expect(word.lowercased().contains("recording"))
        }
    }

    @Test func aProblemKeepsTheTimeTheTitleAndTheReason() {
        let r = WMenuRecording(state: .problem("The microphone stopped."), elapsedSeconds: 848, title: "Pricing review")
        #expect(WMenuText.titleLine(r) == "Pricing review")
        #expect(WMenuText.problemLine(r, strings: strings) == "The microphone stopped.")
        let spoken = WMenuText.spokenState(r, strings: strings)
        #expect(spoken == "Recording problem, 14 minutes 8 seconds, Pricing review")
    }

    @Test func aProblemWithoutAReasonStillSaysSomethingTrue() {
        let r = WMenuRecording(state: .problem(""), elapsedSeconds: 5, title: nil)
        let line = WMenuText.problemLine(r, strings: strings)
        #expect(line == "Some audio may not be recording.")
        #expect(line != strings.problem)
        // With no streams live the streams line already says so; the generic line would only repeat it, weaker.
        let none = WMenuRecording(state: .problem(""), streams: WMenuStreams(microphone: false, systemAudio: false))
        #expect(WMenuText.problemLine(none, strings: strings) == nil)
        #expect(WMenuText.streamsLine(none, strings: strings) == "No audio is being captured.")
        #expect(WMenuText.problemLine(WMenuRecording(state: .problem(""), streams: WMenuStreams(microphone: true, systemAudio: false)), strings: strings) == "Some audio may not be recording.")
        #expect(WMenuText.problemLine(WMenuRecording(state: .recording), strings: strings) == nil)
        #expect(WMenuText.problemLine(WMenuRecording(state: .paused), strings: strings) == nil)
    }

    @Test func theTitleLineIsTheTitleAloneOrLeftOut() {
        #expect(WMenuText.titleLine(WMenuRecording(state: .recording, title: "Pricing review")) == "Pricing review")
        #expect(WMenuText.titleLine(WMenuRecording(state: .paused, title: "Pricing review")) == "Pricing review")
        #expect(WMenuText.titleLine(WMenuRecording(state: .recording, title: nil)) == nil)
        #expect(WMenuText.titleLine(WMenuRecording(state: .recording, title: "")) == nil)
        #expect(WMenuText.titleLine(WMenuRecording(state: .processing, title: "Pricing review")) == nil)
    }

    @Test func theStreamsLineNamesWhatIsLive() {
        func line(_ mic: Bool, _ system: Bool, _ state: WRecordingState = .recording) -> String? {
            WMenuText.streamsLine(WMenuRecording(state: state, streams: WMenuStreams(microphone: mic, systemAudio: system)), strings: strings)
        }
        #expect(line(true, true) == "Microphone and system audio")
        #expect(line(true, false) == "Microphone only")
        #expect(line(false, true) == "System audio only")
        #expect(line(false, false, .problem("")) == "No audio is being captured.")
        // A problem after a failed resume names the stream that still runs.
        #expect(line(false, true, .problem("x")) == "System audio only")
    }

    @Test func aPausedRecordingSaysNothingAboutStreamsAndAnUnknownOneNeither() {
        let paused = WMenuRecording(state: .paused, streams: WMenuStreams(microphone: true, systemAudio: true))
        #expect(WMenuText.streamsLine(paused, strings: strings) == nil)
        #expect(WMenuText.streamsLine(WMenuRecording(state: .recording, streams: nil), strings: strings) == nil)
        #expect(WMenuText.streamsLine(WMenuRecording(state: .starting, streams: WMenuStreams(microphone: false, systemAudio: false)), strings: strings) == nil)
    }

    // MARK: Memory

    @Test func theSpokenMemoryNameSaysMemoryWhenPaused() {
        #expect(WMenuText.memoryName(.on, strings: strings) == "Memory on")
        #expect(WMenuText.memoryName(.off, strings: strings) == "Memory off")
        #expect(WMenuText.memoryName(.pausedUntil("3:10 PM"), strings: strings) == "Memory paused until 3:10 PM")
        // The words on screen say "Memory", so they are not taken for the recording's "Recording paused".
        #expect(WMemoryState.pausedUntil("3:10 PM").title(using: strings) == "Memory paused until 3:10 PM")
    }

    @Test func memoryThatIsOnButNotCapturingIsNeverSaidAsPlainMemoryOn() {
        let title = WMemoryState.onNotCapturing.title(using: strings)
        #expect(title == "Memory on, but not capturing")
        #expect(title != WMemoryState.on.title(using: strings))
        #expect(WMenuText.memoryName(.onNotCapturing, strings: strings) == title)
    }

    @Test func theMenuNeverShowsTheBareWordPausedForMemory() {
        // The recording says "Recording paused"; memory says "Memory paused until". Neither is the word alone.
        let words = [WMenuText.stateWord(.paused, strings: strings), WMemoryState.pausedUntil("3:10 PM").title(using: strings)]
        #expect(words == ["Recording paused", "Memory paused until 3:10 PM"])
        #expect(!words.contains("Paused"))
    }

    // MARK: Order and focus

    @Test func theMenuOffersStartOnlyWhenNothingRecords() {
        #expect(WMenuModel().offersStart)
        #expect(!WMenuModel(recording: WMenuRecording(state: .recording)).offersStart)
    }

    @Test func focusGoesTopToBottomInTheSpecOrder() {
        let model = WMenuModel(
            recording: WMenuRecording(state: .recording),
            needs: [WMenuNeed(id: "sync", kind: .sync, title: "Sync", actions: []), WMenuNeed(id: "lib", kind: .library, title: "Library", actions: [WMenuAction(title: "Dismiss") {}])],
            next: WMenuNext(title: "Design sync", time: "1:30 PM", topic: "Pricing", hasBrief: true))
        #expect(model.focusOrder == [
            "pause", "stop", "meetingWindow", "memory", "need.sync.0", "need.lib.0", "topic", "brief", "ask", "tellAFriend", "open", "settings", "quit",
        ])
    }

    @Test func focusSkipsControlsThatAreSwitchedOff() {
        let model = WMenuModel(
            recording: WMenuRecording(state: .paused, canTogglePause: false),
            needs: [WMenuNeed(id: "sync", kind: .sync, title: "Sync")], disabled: [WMenuFocusID.memory, "need.sync.0"])
        #expect(model.focusOrder == ["stop", "meetingWindow", "ask", "tellAFriend", "open", "settings", "quit"])
        let idle = WMenuModel(disabled: [WMenuFocusID.memory, WMenuFocusID.start])
        #expect(idle.focusOrder == ["ask", "tellAFriend", "open", "settings", "quit"])
    }

    @Test func theMenuOpensWithFocusOnTheFirstControlThatIsOn() {
        #expect(WMenuModel().initialFocus == "memory")
        #expect(WMenuModel(recording: WMenuRecording(state: .recording)).initialFocus == "pause")
        #expect(WMenuModel(recording: WMenuRecording(state: .problem("x"))).initialFocus == "stop")
        #expect(WMenuModel(disabled: [WMenuFocusID.memory]).initialFocus == "start")
    }

    @Test func aStepNeverLandsOnADisabledControl() {
        let model = WMenuModel(disabled: [WMenuFocusID.start])
        #expect(WMenuFocus.move(from: "memory", in: model.focusOrder, .down) == "ask")
        #expect(WMenuFocus.move(from: "ask", in: model.focusOrder, .up) == "memory")
    }

    @Test func everyControlTheMenuDrawsIsInTheFocusOrder() {
        let model = WMenuModel(
            recording: WMenuRecording(state: .recording),
            needs: [WMenuNeed(id: "lib", kind: .library, title: "L", actions: [WMenuAction(title: "Help") {}, WMenuAction(title: "Dismiss") {}])],
            next: WMenuNext(title: "x", time: "1:30 PM", topic: "t", hasBrief: true))
        for id in ["pause", "stop", "meetingWindow", "memory", "need.lib.0", "need.lib.1", "topic", "brief", "ask", "tellAFriend", "open", "settings", "quit"] {
            #expect(model.focusOrder.contains(id), "\(id)")
        }
    }

    @Test func theMiddleScrollsOnlyWhenTheMenuIsTallerThanTheScreen() {
        // Fits: the middle keeps its own height.
        #expect(WMenuLayout.middleHeight(ideal: 300, pinnedTop: 150, footer: 130, cap: 760) == 300)
        // Taller: the middle takes what the pinned parts leave.
        #expect(WMenuLayout.middleHeight(ideal: 700, pinnedTop: 150, footer: 130, cap: 760) == 480)
        // No cap: as tall as it is.
        #expect(WMenuLayout.middleHeight(ideal: 700, pinnedTop: 150, footer: 130, cap: nil) == 700)
        // The pinned parts alone are taller than the cap: the middle is still a usable list.
        #expect(WMenuLayout.middleHeight(ideal: 700, pinnedTop: 500, footer: 400, cap: 760) == 96)
        #expect(WMenuLayout.middleHeight(ideal: 40, pinnedTop: 500, footer: 400, cap: 760) == 40)
    }

    @Test func withoutARecordingStartComesBeforeAsk() {
        let order = WMenuModel().focusOrder
        #expect(order == ["memory", "start", "ask", "tellAFriend", "open", "settings", "quit"])
    }

    @Test func aSyncNeedHasASlotAfterItsOwnActions() {
        let model = WMenuModel(needs: [WMenuNeed(id: "sync", kind: .sync, title: "Sync")])
        #expect(model.focusOrder.contains("need.sync.0"))
    }

    @Test func arrowKeysStepAndStopAtTheEnds() {
        let order = ["memory", "start", "ask"]
        #expect(WMenuFocus.move(from: nil, in: order, .down) == "memory")
        #expect(WMenuFocus.move(from: nil, in: order, .up) == "ask")
        #expect(WMenuFocus.move(from: "memory", in: order, .down) == "start")
        #expect(WMenuFocus.move(from: "ask", in: order, .down) == "ask")
        #expect(WMenuFocus.move(from: "memory", in: order, .up) == "memory")
        #expect(WMenuFocus.move(from: "start", in: order, .up) == "memory")
        #expect(WMenuFocus.move(from: "gone", in: order, .down) == "memory")
        #expect(WMenuFocus.move(from: "memory", in: [], .down) == nil)
    }

    // MARK: Alert windows

    @Test func theMainActionComesFirst() {
        let model = WAlertPanelModel(
            title: "Meeting detected", main: WAlertAction(id: "start", title: "Start Recording") {}, alternate: WAlertAction(id: "open", title: "Start Recording & Open Whimbrel") {},
            others: [WAlertAction(id: "brief", title: "Open brief") {}, WAlertAction(id: "dismiss", title: "Dismiss") {}])
        #expect(model.actionIDs == ["start", "open", "brief", "dismiss"])
    }

    @Test func noButtonIsTheReturnDefaultUnlessItIsMarkedSafe() {
        let main = WAlertAction(id: "start", title: "Start Recording") {}
        #expect(!main.isSafeDefault)
        #expect(WAlertAction(id: "keep", title: "Keep recording", isSafeDefault: true) {}.isSafeDefault)
    }

    // MARK: The hotkey panel

    @Test func aStreamedAnswerStaysInOnePhase() {
        #expect(WHotkeyPhase.answering(partial: "The sheet").kind == WHotkeyPhase.answering(partial: "The sheet lists three tiers").kind)
        #expect(WHotkeyPhase.answering(partial: "").kind == .answering)
        #expect(WHotkeyPhase.answered("a").kind != WHotkeyPhase.answering(partial: "a").kind)
        #expect(WHotkeyPhase.blocked(message: "a", action: nil).kind == WHotkeyPhase.blocked(message: "b", action: WHotkeyAction(id: "x", title: "y")).kind)
    }

    // MARK: The start of a recording

    private let t0 = Date(timeIntervalSince1970: 1_800_000_000)

    @Test func justStartedWithNoStreamsSaysItIsStartingNotAProblem() {
        for seconds in [0.0, 1, 4.9] {
            let state = WMenuRecordingRules.judged(
                .recording, microphoneLive: false, systemAudioLive: false, startedAt: t0, now: t0.addingTimeInterval(seconds), reason: nil)
            #expect(state == .starting, "\(seconds)")
        }
    }

    @Test func tenSecondsInWithNoStreamsIsAProblemAndKeepsTheReason() {
        #expect(WMenuRecordingRules.judged(.recording, microphoneLive: false, systemAudioLive: false, startedAt: t0, now: t0.addingTimeInterval(10), reason: nil) == .problem(""))
        #expect(WMenuRecordingRules.judged(.recording, microphoneLive: false, systemAudioLive: false, startedAt: t0, now: t0.addingTimeInterval(5), reason: "Stream stopped") == .problem("Stream stopped"))
    }

    @Test func aLiveStreamIsNeverAProblemAndPausedOrResumingIsNeverJudged() {
        #expect(WMenuRecordingRules.judged(.recording, microphoneLive: true, systemAudioLive: false, startedAt: t0, now: t0.addingTimeInterval(60), reason: nil) == .recording)
        for state in [WRecordingState.paused, .resuming, .starting, .processing, .off, .problem("x")] {
            #expect(WMenuRecordingRules.judged(state, microphoneLive: false, systemAudioLive: false, startedAt: t0, now: t0.addingTimeInterval(60), reason: nil) == state)
            #expect(WMenuRecordingRules.judged(state, microphoneLive: false, systemAudioLive: false, startedAt: t0, now: t0, reason: nil) == state)
        }
    }

    @Test func withNoStartTimeKnownTheStreamsAreJudgedAtOnce() {
        #expect(WMenuRecordingRules.judged(.recording, microphoneLive: false, systemAudioLive: false, startedAt: nil, now: t0, reason: nil) == .problem(""))
    }

    @Test func theStartIsTheLaterOfTheMeetingsAndTheAudiosOwn() {
        let old = t0.addingTimeInterval(-3600)
        // A continued meeting keeps its first start; the audio's start is this part's.
        #expect(WMenuRecordingRules.startAnchor(meetingStartedAt: old, audioStartedAt: t0) == t0)
        // A manual start: the meeting is open before the audio task runs, so the audio's time is still the last recording's.
        #expect(WMenuRecordingRules.startAnchor(meetingStartedAt: t0, audioStartedAt: old) == t0)
        #expect(WMenuRecordingRules.startAnchor(meetingStartedAt: t0, audioStartedAt: nil) == t0)
        #expect(WMenuRecordingRules.startAnchor(meetingStartedAt: nil, audioStartedAt: t0) == t0)
        #expect(WMenuRecordingRules.startAnchor(meetingStartedAt: nil, audioStartedAt: nil) == nil)
    }

    @Test func aStartWithTheMicrophoneOnButNoMeetingYetSaysStopWaitsAndNamesTheLiveStream() {
        let waiting = WMenuRecording(state: .starting, canTogglePause: false, streams: WMenuStreams(microphone: true, systemAudio: false), canStop: false)
        #expect(waiting.stopWaitsForMeeting)
        #expect(waiting.showsStreams)
        #expect(WMenuText.streamsLine(waiting, strings: strings) == "Microphone only")
        #expect(WMenuText.stopWaitsLine(waiting, strings: strings) == "Recording has started. Stop is not available until the meeting is saved.")
        // Nothing live yet: no alarming "no audio" line, and no claim about Stop.
        let quiet = WMenuRecording(state: .starting, streams: WMenuStreams(microphone: false, systemAudio: false), canStop: false)
        #expect(!quiet.stopWaitsForMeeting && !quiet.showsStreams)
        #expect(WMenuText.streamsLine(quiet, strings: strings) == nil)
        // With a meeting open Stop works, so there is nothing to explain.
        let open = WMenuRecording(state: .starting, streams: both, canStop: true)
        #expect(!open.stopWaitsForMeeting)
        #expect(WMenuText.stopWaitsLine(open, strings: strings) == nil)
        #expect(WMenuText.stopWaitsLine(WMenuRecording(state: .recording, streams: both, canStop: false), strings: strings) == nil)
    }

    private var both: WMenuStreams { WMenuStreams(microphone: true, systemAudio: true) }

    // MARK: Memory that is on and not saving

    @Test func memoryThatIsNotSavingIsHeldBackOnlyForAFewSecondsAfterItWasSwitchedOn() {
        let m = WMemoryState.onNotCapturing
        #expect(WMemoryHold.state(m, since: t0, now: t0.addingTimeInterval(1)) == .on)
        #expect(WMemoryHold.state(m, since: t0, now: t0.addingTimeInterval(4.9)) == .on)
        #expect(WMemoryHold.state(m, since: t0, now: t0.addingTimeInterval(5)) == .onNotCapturing)
        // Hours old (a permission that has been off all day) is called what it is at once.
        #expect(WMemoryHold.state(m, since: t0.addingTimeInterval(-7200), now: t0) == .onNotCapturing)
        // No time known: nothing is held back.
        #expect(WMemoryHold.state(m, since: nil, now: t0) == .onNotCapturing)
    }

    @Test func theHoldEndIsTheMomentToRedrawAndOnlyWhileHolding() {
        #expect(WMemoryHold.holdEnds(capturing: false, since: t0, now: t0.addingTimeInterval(1)) == t0.addingTimeInterval(5))
        #expect(WMemoryHold.holdEnds(capturing: false, since: t0, now: t0.addingTimeInterval(5)) == nil)
        #expect(WMemoryHold.holdEnds(capturing: true, since: t0, now: t0) == nil)
        #expect(WMemoryHold.holdEnds(capturing: false, since: nil, now: t0) == nil)
        #expect(WMemoryHold.holdEnds(capturing: false, since: t0.addingTimeInterval(-7200), now: t0) == nil)
    }

    @Test func theNextRedrawIsTheEarlierOfTheTickAndTheHoldEnd() {
        let anchor = t0
        // Periodic only: the next tick strictly after now.
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(10), anchor: anchor, interval: 60, extra: nil) == t0.addingTimeInterval(60))
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(60), anchor: anchor, interval: 60, extra: nil) == t0.addingTimeInterval(120))
        // The hold ends before the tick: that moment comes first, then the ticks go on.
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(2), anchor: anchor, interval: 60, extra: t0.addingTimeInterval(5)) == t0.addingTimeInterval(5))
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(5), anchor: anchor, interval: 60, extra: t0.addingTimeInterval(5)) == t0.addingTimeInterval(60))
        // An extra in the past, or after the tick, changes nothing.
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(10), anchor: anchor, interval: 60, extra: t0.addingTimeInterval(5)) == t0.addingTimeInterval(60))
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(10), anchor: anchor, interval: 30, extra: t0.addingTimeInterval(45)) == t0.addingTimeInterval(30))
        // Before the anchor still gives a later date.
        #expect(WRedrawClock.next(after: t0.addingTimeInterval(-10), anchor: anchor, interval: 60, extra: nil) > t0.addingTimeInterval(-10))
    }

    @Test func theHoldChangesNothingElse() {
        for m in [WMemoryState.on, .off, .pausedUntil("3:10 PM")] {
            #expect(WMemoryHold.state(m, since: t0, now: t0) == m)
        }
    }

    // MARK: The scroll cue

    @Test func aMenuThatFitsHasNoCue() {
        #expect(WMenuScrollCue.cue(contentHeight: 300, viewportHeight: 300, offset: 0) == WMenuScrollCue(above: false, below: false))
        #expect(WMenuScrollCue.cue(contentHeight: 301, viewportHeight: 300, offset: 0) == WMenuScrollCue(above: false, below: false))
        #expect(WMenuScrollCue.cue(contentHeight: 300, viewportHeight: 0, offset: 0) == WMenuScrollCue(above: false, below: false))
    }

    @Test func aCappedMenuCuesWhereThereIsMoreAndOnlyThere() {
        #expect(WMenuScrollCue.cue(contentHeight: 700, viewportHeight: 400, offset: 0) == WMenuScrollCue(above: false, below: true))
        #expect(WMenuScrollCue.cue(contentHeight: 700, viewportHeight: 400, offset: 120) == WMenuScrollCue(above: true, below: true))
        #expect(WMenuScrollCue.cue(contentHeight: 700, viewportHeight: 400, offset: 300) == WMenuScrollCue(above: true, below: false))
        // Rubber-banding past either end changes nothing.
        #expect(WMenuScrollCue.cue(contentHeight: 700, viewportHeight: 400, offset: -20) == WMenuScrollCue(above: false, below: true))
        #expect(WMenuScrollCue.cue(contentHeight: 700, viewportHeight: 400, offset: 320) == WMenuScrollCue(above: true, below: false))
    }

    // MARK: Focus when the controls change

    @Test func focusStaysOnAControlThatIsStillThere() {
        let order = ["pause", "stop", "memory", "ask"]
        #expect(WMenuFocus.kept("memory", in: order, fallback: "pause") == "memory")
        #expect(WMenuFocus.kept("ask", in: ["memory", "ask"], fallback: "memory") == "ask")
    }

    @Test func focusMovesOnlyWhenItsControlIsGone() {
        // A recording starts: Start Meeting leaves the menu.
        #expect(WMenuFocus.kept("start", in: ["pause", "stop", "memory", "ask"], fallback: "pause") == "pause")
        #expect(WMenuFocus.kept(nil, in: ["memory"], fallback: "memory") == nil)
        #expect(WMenuFocus.kept("stop", in: [], fallback: nil) == nil)
    }

    // MARK: Words


    @Test func theEnglishMenuWordsUseNoDashesAndNameMemoryNotCapture() {
        let s = WMenuStrings.english
        let all = [s.appName, s.menuName, s.needsAttention, s.nextMeeting, s.workingOn, s.pauseRecording, s.resumeRecording, s.stopRecording,
                   s.stateWithTitle("Recording", "Pricing review"), s.recordingProblem, s.startingRecording, s.resumingRecording, s.stopWaitsForMeeting, s.problemUnknown, s.saving,
                   s.streamsBoth, s.streamsMicrophone, s.streamsSystemAudio, s.streamsNone, WStrings.english.memoryNotCapturing,
                   WStrings.english.pausedUntil("3:10 PM"), s.memoryPausedUntil("3:10 PM"), s.turnOnMemory, s.turnOffMemory, s.startMeeting,
                   s.openBrief, s.openBriefFor("x"), s.openTopic("x"), s.askAboutWindow, s.openWhimbrel, s.settings, s.quit, s.cancel,
                   s.tellAFriend, s.tellAFriendCopied, s.tellAFriendHint, s.alertWindowName, s.hotkeyWindowName, s.close, s.copy, s.answering]
        for text in all {
            #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"), "dash in \(text)")
            #expect(!text.lowercased().contains("activity capture"), "old name in \(text)")
        }
    }

    // MARK: Stop while a start waits on its meeting row (#992)

    @Test func stopOfferForEveryStateOfAStart() {
        func offer(meeting: Bool = false, starting: Bool, audio: Bool, canStopStarting: Bool, pending: Bool) -> WStopOffer {
            WMenuRecordingRules.stopOffer(
                hasActiveMeeting: meeting, isStarting: starting, audioLive: audio, canStopStarting: canStopStarting, stopPendingForRow: pending)
        }
        // A meeting is open: Stop, whatever else is true.
        #expect(offer(meeting: true, starting: false, audio: true, canStopStarting: false, pending: false) == .stop)
        #expect(offer(meeting: true, starting: true, audio: false, canStopStarting: false, pending: false) == .stop)
        // A start with its audio ahead of its row: Stop works.
        #expect(offer(starting: true, audio: true, canStopStarting: true, pending: false) == .stop)
        // A Stop has been asked for and the row is not there yet: Saving, never Stop.
        #expect(offer(starting: true, audio: false, canStopStarting: false, pending: true) == .saving)
        #expect(offer(starting: true, audio: true, canStopStarting: false, pending: true) == .saving)
        // Any other start in flight has nothing to stop yet.
        #expect(offer(starting: true, audio: false, canStopStarting: false, pending: false) == .unavailable)
        #expect(offer(starting: true, audio: true, canStopStarting: false, pending: false) == .unavailable)
        // No start and no meeting: audio with no row can be stopped (its insert failed); silence has nothing to stop.
        #expect(offer(starting: false, audio: true, canStopStarting: false, pending: false) == .stop)
        #expect(offer(starting: false, audio: false, canStopStarting: false, pending: false) == .unavailable)
    }

    @Test func theStopSentenceIsOnlyForAStateWhereStopCannotWork() {
        let both = WMenuStreams(microphone: true, systemAudio: true)
        // Stop works: no sentence.
        #expect(!WMenuRecording(state: .starting, streams: both, canStop: true).stopWaitsForMeeting)
        // Saving: no sentence, and the row is flagged so the menu draws Saving where Stop was.
        let saving = WMenuRecording(state: .starting, streams: both, canStop: false, isSaving: true)
        #expect(!saving.stopWaitsForMeeting && saving.isSaving && saving.offersStop)
        #expect(WMenuText.stopWaitsLine(saving, strings: strings) == nil)
        // Audio is live and Stop truly cannot work (no Stop offered, none pending): the sentence stays.
        #expect(WMenuRecording(state: .starting, streams: both, canStop: false).stopWaitsForMeeting)
    }

    @Test func theSavingWordIsTheOldMenusWord() {
        #expect(strings.menu.saving == "Saving\u{2026}")
    }

    @MainActor @Test func theGalleryShowsTheStartingStatesOfStop() {
        let ids = WMenuGallery.entries().map(\.id)
        for id in ["menu-recording-starting-stop-waits", "menu-recording-starting-stop", "menu-recording-starting-saving"] { #expect(ids.contains(id), "\(id)") }
    }

    @MainActor @Test func theSampleModelsCoverEveryHotkeyState() {
        // The gallery draws each state; this keeps the list from losing one.
        let ids = WMenuGallery.entries().map(\.id)
        for id in ["hotkey-reading", "hotkey-ready", "hotkey-blocked", "hotkey-answering", "hotkey-answered", "hotkey-failed",
                   "alert-meeting-detected", "alert-countdown", "checkin-recording", "checkin-paused", "launch-menu-starting"] {
            #expect(ids.contains(id), "\(id)")
        }
    }
}
