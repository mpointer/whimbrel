import Foundation

/// When a calendar meeting starts recording on its own. Only a Mac that's in the call
/// auto-starts: its mic is in use and it plays the other people. A Mac in the same room
/// that only overhears the call (mic busy, nothing playing) keeps the prompt without a
/// countdown, and Start still works there. Detected meetings already require both.
/// docs/superpowers/specs/2026-11-02-meeting-dedupe-design.md §1.
enum CallAutoStart {
    /// Test seam for the device check.
    static var isInCall: () -> Bool = {
        inCall(mic: AudioDeviceActivity.isDefaultInputInUse(), output: AudioDeviceActivity.isDefaultOutputInUse())
    }

    static func inCall(mic: Bool, output: Bool) -> Bool { mic && output }

    /// `waitForCall`: show the prompt without a countdown and start the countdown once
    /// `isInCall` turns true.
    static func decide(autoStartEnabled: Bool, source: MeetingSource, inCall: Bool) -> (autoStart: Bool, waitForCall: Bool) {
        let waitForCall = autoStartEnabled && source != .detected && !inCall
        return (autoStartEnabled && !waitForCall, waitForCall)
    }
}
