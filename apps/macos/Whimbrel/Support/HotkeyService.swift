import AppKit
import Carbon.HIToolbox
import Combine
import Foundation

/// A global shortcut: a Carbon virtual key code plus a Carbon modifier mask
/// (`cmdKey | shiftKey | optionKey | controlKey`).
struct Shortcut: Codable, Equatable {
    var keyCode: UInt32
    var modifiers: UInt32

    private static let modifierMask = UInt32(cmdKey | shiftKey | optionKey | controlKey)

    /// Why a shortcut can't be a global hotkey.
    enum Problem: Equatable {
        /// No modifier: it would swallow plain typing.
        case noModifier
        /// Command or Shift alone: it would take an app's own shortcut (⌘K) or a capital letter.
        case needsControlOrOption
    }

    /// Nil when usable: Control or Option, or any two modifiers.
    var problem: Problem? {
        let mods = modifiers & Shortcut.modifierMask
        if mods == 0 { return .noModifier }
        if mods & UInt32(controlKey | optionKey) != 0 || mods.nonzeroBitCount >= 2 { return nil }
        return .needsControlOrOption
    }

    var isValid: Bool { problem == nil }

    /// Carbon modifier mask for an AppKit event's flags.
    static func carbonModifiers(_ flags: NSEvent.ModifierFlags) -> UInt32 {
        var m: UInt32 = 0
        if flags.contains(.command) { m |= UInt32(cmdKey) }
        if flags.contains(.shift) { m |= UInt32(shiftKey) }
        if flags.contains(.option) { m |= UInt32(optionKey) }
        if flags.contains(.control) { m |= UInt32(controlKey) }
        return m
    }

    /// Human-readable form such as "⌃⌥Space".
    var displayString: String {
        var s = ""
        if modifiers & UInt32(controlKey) != 0 { s += "⌃" }
        if modifiers & UInt32(optionKey) != 0 { s += "⌥" }
        if modifiers & UInt32(shiftKey) != 0 { s += "⇧" }
        if modifiers & UInt32(cmdKey) != 0 { s += "⌘" }
        return s + Shortcut.keyName(keyCode)
    }

    private static func keyName(_ code: UInt32) -> String {
        let names: [Int: String] = [
            kVK_Space: "Space", kVK_Return: "Return", kVK_Tab: "Tab", kVK_Delete: "Delete",
            kVK_Escape: "Esc", kVK_LeftArrow: "←", kVK_RightArrow: "→", kVK_UpArrow: "↑",
            kVK_DownArrow: "↓", kVK_ANSI_A: "A", kVK_ANSI_B: "B", kVK_ANSI_C: "C",
            kVK_ANSI_D: "D", kVK_ANSI_E: "E", kVK_ANSI_F: "F", kVK_ANSI_G: "G",
            kVK_ANSI_H: "H", kVK_ANSI_I: "I", kVK_ANSI_J: "J", kVK_ANSI_K: "K",
            kVK_ANSI_L: "L", kVK_ANSI_M: "M", kVK_ANSI_N: "N", kVK_ANSI_O: "O",
            kVK_ANSI_P: "P", kVK_ANSI_Q: "Q", kVK_ANSI_R: "R", kVK_ANSI_S: "S",
            kVK_ANSI_T: "T", kVK_ANSI_U: "U", kVK_ANSI_V: "V", kVK_ANSI_W: "W",
            kVK_ANSI_X: "X", kVK_ANSI_Y: "Y", kVK_ANSI_Z: "Z",
            kVK_ANSI_0: "0", kVK_ANSI_1: "1", kVK_ANSI_2: "2", kVK_ANSI_3: "3",
            kVK_ANSI_4: "4", kVK_ANSI_5: "5", kVK_ANSI_6: "6", kVK_ANSI_7: "7",
            kVK_ANSI_8: "8", kVK_ANSI_9: "9",
        ]
        return names[Int(code)] ?? "Key \(code)"
    }
}

enum HotkeyError: Error, Equatable { case alreadyInUse, invalid }

protocol HotkeyRegistrar {
    func register(_ s: Shortcut, handler: @escaping () -> Void) -> Result<Void, HotkeyError>
    func unregister()
}

/// Owns the panel's global hotkey: persisted shortcut, registration and its outcome.
/// A failed registration is surfaced in `status`, never thrown.
@MainActor
final class HotkeyService: ObservableObject {
    enum Status: Equatable { case off, registered, failed(HotkeyError) }

    static let defaultsKey = "com.mpointer.kestrel.hotkeyPanel.shortcut"
    /// Control+Option+Space.
    static let defaultShortcut = Shortcut(keyCode: UInt32(kVK_Space), modifiers: UInt32(controlKey | optionKey))

    @Published private(set) var status: Status = .off
    @Published private(set) var shortcut: Shortcut

    private let registrar: HotkeyRegistrar
    private let defaults: UserDefaults
    private var onTrigger: (() -> Void)?

    init(registrar: HotkeyRegistrar, defaults: UserDefaults = .standard) {
        self.registrar = registrar
        self.defaults = defaults
        if let data = defaults.data(forKey: Self.defaultsKey),
           let saved = try? JSONDecoder().decode(Shortcut.self, from: data), saved.isValid {
            shortcut = saved
        } else {
            shortcut = Self.defaultShortcut
        }
    }

    func start(onTrigger: @escaping () -> Void) {
        self.onTrigger = onTrigger
        apply()
    }

    /// What `setShortcut` did.
    enum Change: Equatable {
        /// Registered (or, before `start`, saved for it) and saved.
        case applied
        /// Not a usable global shortcut: nothing changed.
        case rejected(Shortcut.Problem)
        /// Couldn't be registered: the old shortcut is registered again and stays saved.
        case failed(HotkeyError)
    }

    /// Registers `s` and, only once that works, saves it. If it can't be registered the
    /// old shortcut is registered again and the saved value is left alone (as Windows'
    /// `set_hotkey_shortcut`).
    @discardableResult
    func setShortcut(_ s: Shortcut) -> Change {
        if let problem = s.problem { return .rejected(problem) }
        if let onTrigger {
            let old = shortcut
            let oldStatus = status
            registrar.unregister()
            if case .failure(let e) = registrar.register(s, handler: { onTrigger() }) {
                registrar.unregister()
                if oldStatus == .registered {
                    switch registrar.register(old, handler: { onTrigger() }) {
                    case .success: status = .registered
                    case .failure(let restoreError): status = .failed(restoreError)
                    }
                }
                return .failed(e)
            }
            status = .registered
        }
        shortcut = s
        if let data = try? JSONEncoder().encode(s) { defaults.set(data, forKey: Self.defaultsKey) }
        return .applied
    }

    @discardableResult
    func resetToDefault() -> Change { setShortcut(Self.defaultShortcut) }

    private func apply() {
        registrar.unregister()
        guard let onTrigger else { status = .off; return }
        switch registrar.register(shortcut, handler: { onTrigger() }) {
        case .success: status = .registered
        case .failure(let e): status = .failed(e)
        }
    }
}

/// Carbon `RegisterEventHotKey`: needs no permission, unlike an `NSEvent` global monitor.
final class CarbonHotkeyRegistrar: HotkeyRegistrar {
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private var handler: (() -> Void)?
    private static let signature: OSType = 0x4B535452 // 'KSTR'

    func register(_ s: Shortcut, handler: @escaping () -> Void) -> Result<Void, HotkeyError> {
        guard s.isValid else { return .failure(.invalid) }
        unregister()
        self.handler = handler
        if handlerRef == nil {
            var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
            let status = InstallEventHandler(
                GetApplicationEventTarget(),
                { _, _, userData in
                    guard let userData else { return OSStatus(eventNotHandledErr) }
                    let me = Unmanaged<CarbonHotkeyRegistrar>.fromOpaque(userData).takeUnretainedValue()
                    DispatchQueue.main.async { me.handler?() }
                    return noErr
                },
                1, &spec, Unmanaged.passUnretained(self).toOpaque(), &handlerRef
            )
            guard status == noErr else { return .failure(.alreadyInUse) }
        }
        var ref: EventHotKeyRef?
        let id = EventHotKeyID(signature: Self.signature, id: 1)
        let status = RegisterEventHotKey(s.keyCode, s.modifiers, id, GetApplicationEventTarget(), 0, &ref)
        guard status == noErr, let ref else {
            return .failure(status == OSStatus(eventHotKeyExistsErr) ? .alreadyInUse : .invalid)
        }
        hotKeyRef = ref
        return .success(())
    }

    func unregister() {
        if let hotKeyRef { UnregisterEventHotKey(hotKeyRef) }
        hotKeyRef = nil
        handler = nil
    }

    deinit {
        unregister()
        if let handlerRef { RemoveEventHandler(handlerRef) }
    }
}
