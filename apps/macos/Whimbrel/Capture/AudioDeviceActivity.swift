import CoreAudio
import Foundation

/// FR-MTG-2: "sustained bidirectional audio (mic in use + system audio out)" — checking
/// whether the default input/output devices are in use by *any* process, not just this
/// app. This is a device-state query (Core Audio HAL's `kAudioDevicePropertyDeviceIsRunningSomewhere`),
/// not audio capture itself, so it needs no microphone TCC grant and no running
/// `AVAudioEngine`/`SCStream` of our own — genuinely answers "is the mic busy right
/// now," which macOS has no simpler public API for.
enum AudioDeviceActivity {
    static func isDefaultInputInUse() -> Bool {
        isDeviceRunningSomewhere(defaultDeviceSelector: kAudioHardwarePropertyDefaultInputDevice)
    }

    /// The current default input device, so the mic capture can notice a switch that
    /// arrives without an `AVAudioEngineConfigurationChange` (seen live: switching
    /// back to the built-in mic posted none).
    static func defaultInputDeviceID() -> AudioDeviceID? {
        defaultDevice(selector: kAudioHardwarePropertyDefaultInputDevice)
    }

    static func isDefaultOutputInUse() -> Bool {
        isDeviceRunningSomewhere(defaultDeviceSelector: kAudioHardwarePropertyDefaultOutputDevice)
    }

    /// #84: is this app still using the microphone, by Core Audio's per-process
    /// objects? A meeting app that drops back to its lobby keeps running but stops it;
    /// the end rules read that as the call having ended. Matches the app's own bundle
    /// id and its helpers (`<id>.helper…`). Nil, meaning "can't tell, don't use this
    /// rule", when the query fails or no audio process matches: some apps play and
    /// record from a process that doesn't carry their bundle id, and treating that as
    /// "released" would end every such recording. Reads device state only, so it needs
    /// no TCC grant.
    static func isAppCapturingAudio(bundleID: String) -> Bool? {
        let system = AudioObjectID(kAudioObjectSystemObject)
        var listAddress = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyProcessObjectList,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &listAddress, 0, nil, &size) == noErr else { return nil }
        var processes = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(system, &listAddress, 0, nil, &size, &processes) == noErr else { return nil }

        var matched = false
        for process in processes {
            guard let id = processBundleID(process), id == bundleID || id.hasPrefix(bundleID + ".") else { continue }
            matched = true
            // Input only. Measured 2026-09-18 on real calls: Zoom (`us.zoom.xos`) and
            // Teams (`com.microsoft.teams2`) keep input running through mute and stop
            // it when the call is left. Output doesn't work as a signal: a Teams helper
            // keeps its output running in the lobby after the call.
            if processFlag(process, kAudioProcessPropertyIsRunningInput) {
                return true
            }
        }
        return matched ? false : nil
    }

    private static func processBundleID(_ process: AudioObjectID) -> String? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioProcessPropertyBundleID,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var value: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(process, &address, 0, nil, &size, &value) == noErr,
              let id = value?.takeRetainedValue() as String?, !id.isEmpty else { return nil }
        return id
    }

    private static func processFlag(_ process: AudioObjectID, _ selector: AudioObjectPropertySelector) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var flag: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        return AudioObjectGetPropertyData(process, &address, 0, nil, &size, &flag) == noErr && flag != 0
    }

    private static func isDeviceRunningSomewhere(defaultDeviceSelector: AudioObjectPropertySelector) -> Bool {
        guard let deviceID = defaultDevice(selector: defaultDeviceSelector) else { return false }

        var isRunning: UInt32 = 0
        var isRunningSize = UInt32(MemoryLayout<UInt32>.size)
        var runningAddress = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyDeviceIsRunningSomewhere,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let status = AudioObjectGetPropertyData(deviceID, &runningAddress, 0, nil, &isRunningSize, &isRunning)
        guard status == noErr else { return false }
        return isRunning != 0
    }

    private static func defaultDevice(selector: AudioObjectPropertySelector) -> AudioDeviceID? {
        var deviceID = AudioDeviceID(0)
        var deviceIDSize = UInt32(MemoryLayout<AudioDeviceID>.size)
        var deviceAddress = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &deviceAddress, 0, nil, &deviceIDSize, &deviceID)
        guard status == noErr, deviceID != kAudioObjectUnknown else { return nil }
        return deviceID
    }
}
