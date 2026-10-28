import Foundation
import IOKit.pwr_mgt

/// Holds the Mac awake while Away Mode is on, so a locked-screen sleep doesn't strand the Zoom client.
protocol KeepAwakeControlling {
    func begin()
    func end()
}

/// A power assertion that stops idle display sleep. `begin` and `end` are idempotent.
final class KeepAwake: KeepAwakeControlling {
    private var assertionID: IOPMAssertionID = 0
    private var held = false

    func begin() {
        guard !held else { return }
        held = IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleDisplaySleep as CFString,
                                           IOPMAssertionLevel(kIOPMAssertionLevelOn),
                                           "Whimbrel Away Mode" as CFString,
                                           &assertionID) == kIOReturnSuccess
    }

    func end() {
        guard held else { return }
        IOPMAssertionRelease(assertionID)
        held = false
    }

    deinit { end() }
}
