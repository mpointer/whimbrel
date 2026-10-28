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
    static func isEligible(isOnACPower: Bool, idleSeconds: TimeInterval, thresholdSeconds: TimeInterval = idleThresholdSeconds) -> Bool {
        isOnACPower && idleSeconds >= thresholdSeconds
    }

    static func isEligibleForBackgroundWork() -> Bool {
        isEligible(isOnACPower: currentIsOnACPower(), idleSeconds: currentIdleSeconds())
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
