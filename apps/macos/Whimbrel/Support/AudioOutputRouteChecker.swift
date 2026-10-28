import CoreAudio
import Foundation

/// FR-MTG-4 follow-on: detects whether the Mac's current default audio output is its
/// own built-in speakers — the condition under which mic bleed-through is likely (the
/// mic acoustically picks up meeting audio playing out loud, with no headphones to
/// contain it). See docs/superpowers/specs/2026-07-31-diarization-quality-design.md.
enum AudioOutputRouteChecker {
    /// AirPods, Bluetooth, and USB audio devices all return false. Any CoreAudio query
    /// failure also returns false — this is advisory UI guidance, not a gate, so
    /// silently skipping the warning on a query failure is the right degradation
    /// (matches this codebase's "never throw over a labeling/advisory problem" pattern).
    ///
    /// Known exception: headphones plugged into the built-in analog jack are NOT
    /// distinguished from internal speakers by this check. CoreAudio reports both under
    /// the same `kAudioDeviceTransportTypeBuiltIn` transport type; jack-plugged-in state
    /// is only exposed via the separate `kAudioDevicePropertyDataSource` property, which
    /// this check doesn't query. AirPods/Bluetooth/USB are unaffected since they have
    /// distinct transport types.
    static func isUsingBuiltInSpeakers() -> Bool {
        var deviceID = AudioDeviceID(0)
        var deviceIDSize = UInt32(MemoryLayout<AudioDeviceID>.size)
        var deviceAddress = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let deviceStatus = AudioObjectGetPropertyData(
            AudioObjectID(kAudioObjectSystemObject), &deviceAddress, 0, nil, &deviceIDSize, &deviceID
        )
        guard deviceStatus == noErr else { return false }

        var transportType: UInt32 = 0
        var transportSize = UInt32(MemoryLayout<UInt32>.size)
        var transportAddress = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyTransportType,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let transportStatus = AudioObjectGetPropertyData(
            deviceID, &transportAddress, 0, nil, &transportSize, &transportType
        )
        guard transportStatus == noErr else { return false }

        return transportType == kAudioDeviceTransportTypeBuiltIn
    }
}
