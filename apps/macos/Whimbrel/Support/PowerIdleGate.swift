import Foundation
import IOKit.ps
import CoreGraphics

/// AC-power + user-idle gating for background T1 job-queue work
/// (docs/superpowers/specs/2026-08-07-job-queue-design.md's own recommendation --
/// "simplest honest policy to start"). Nothing in this codebase checked power or
/// idle state before this file; there was no existing helper to extend.
enum PowerIdleGate {
    static let idleThresholdSeconds: TimeInterval = 60

    /// Pure predicate, directly testable with synthetic inputs -- real OS state is
    /// read only by `isEligibleForBackgroundWork()` below, which is not itself unit
    /// tested (it can't be, without mocking IOKit/CoreGraphics, which this file
    /// deliberately avoids -- see the design spec's testing section).
    /// `recordingActive` is true while a meeting records: only `exempt` work (Meeting Prep)
    /// runs then, whatever the power and idle state. Same rule as core's
    /// `memory::background_gate::allowed`.
    static func isEligible(isOnACPower: Bool, idleSeconds: TimeInterval, thresholdSeconds: TimeInterval = idleThresholdSeconds,
                           recordingActive: Bool = false, exempt: Bool = false) -> Bool {
        if exempt { return true }
        return !recordingActive && isOnACPower && idleSeconds >= thresholdSeconds
    }

    /// A meeting counts as recording from the moment its start is claimed (before the row lands)
    /// until the active id is cleared, so a start that fails or is cancelled clears with it.
    static func isRecording(activeMeetingId: Int64?, isStartingMeeting: Bool) -> Bool {
        activeMeetingId != nil || isStartingMeeting
    }

    /// Core holds the flag for 10 minutes per call; renew well inside that.
    static let recordingRenewSeconds: TimeInterval = 120

    /// For loops that never waited on AC or idle (segmentation, topics, reply priorities):
    /// true while a meeting records.
    static func recordingPausesBackgroundWork() -> Bool {
        whimbrel_is_recording_active()
    }

    /// Core's rule (`whimbrel_background_work_allowed`) fed this Mac's power and idle state;
    /// core holds the recording flag, so its own batch entry points agree with this answer.
    static func isEligibleForBackgroundWork() -> Bool {
        whimbrel_background_work_allowed(currentIsOnACPower(), currentIdleSeconds(), false)
    }

    private static func currentIsOnACPower() -> Bool {
        guard let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue() else {
            return false
        }
        guard let providingType = IOPSGetProvidingPowerSourceType(snapshot)?.takeUnretainedValue() as String? else {
            return false
        }
        return providingType == (kIOPMACPowerKey as String)
    }

    private static func currentIdleSeconds() -> TimeInterval {
        CGEventSource.secondsSinceLastEventType(.hidSystemState, eventType: .null)
    }
}
