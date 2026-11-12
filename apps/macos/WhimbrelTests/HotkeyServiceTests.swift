import Carbon.HIToolbox
import XCTest
@testable import Whimbrel

/// Fake registrar: never registers a real global hotkey.
private final class FakeRegistrar: HotkeyRegistrar {
    var result: Result<Void, HotkeyError> = .success(())
    /// Per-shortcut outcomes, ahead of `result`.
    var failing: [(Shortcut, HotkeyError)] = []
    private(set) var registered: [Shortcut] = []
    private(set) var unregisterCalls = 0
    private(set) var handler: (() -> Void)?
    private(set) var log: [String] = []

    func register(_ s: Shortcut, handler: @escaping () -> Void) -> Result<Void, HotkeyError> {
        log.append("register")
        registered.append(s)
        self.handler = handler
        if let (_, e) = failing.first(where: { $0.0 == s }) { return .failure(e) }
        return result
    }
    func unregister() { unregisterCalls += 1; log.append("unregister") }
}

@MainActor
final class HotkeyServiceTests: XCTestCase {
    private func makeDefaults() -> UserDefaults {
        let name = "HotkeyServiceTests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: name)!
        d.removePersistentDomain(forName: name)
        return d
    }

    private let custom = Shortcut(keyCode: 49, modifiers: UInt32(cmdKey | shiftKey))

    func testStartRegistersDefault() {
        let reg = FakeRegistrar()
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        XCTAssertEqual(svc.status, .off)
        svc.start(onTrigger: {})
        XCTAssertEqual(reg.registered, [HotkeyService.defaultShortcut])
        XCTAssertEqual(svc.status, .registered)
        XCTAssertEqual(HotkeyService.defaultShortcut.modifiers, UInt32(controlKey | optionKey))
        XCTAssertEqual(HotkeyService.defaultShortcut.keyCode, 49)
    }

    func testTriggerInvokesCallback() {
        let reg = FakeRegistrar()
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        var fired = 0
        svc.start(onTrigger: { fired += 1 })
        reg.handler?()
        XCTAssertEqual(fired, 1)
    }

    func testAlreadyInUseSetsFailedStatus() {
        let reg = FakeRegistrar()
        reg.result = .failure(.alreadyInUse)
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        svc.start(onTrigger: {})
        XCTAssertEqual(svc.status, .failed(.alreadyInUse))
    }

    func testSetShortcutUnregistersThenRegistersAndPersists() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        svc.start(onTrigger: {})
        XCTAssertEqual(svc.setShortcut(custom), .applied)
        XCTAssertEqual(reg.log, ["unregister", "register", "unregister", "register"])
        XCTAssertEqual(reg.registered.last, custom)
        XCTAssertEqual(svc.shortcut, custom)
        let data = defaults.data(forKey: "com.mpointer.kestrel.hotkeyPanel.shortcut")
        XCTAssertNotNil(data)
        XCTAssertEqual(try? JSONDecoder().decode(Shortcut.self, from: data!), custom)
    }

    func testPersistedShortcutLoadedOnNextInit() {
        let defaults = makeDefaults()
        let first = HotkeyService(registrar: FakeRegistrar(), defaults: defaults)
        first.start(onTrigger: {})
        first.setShortcut(custom)
        let reg = FakeRegistrar()
        let second = HotkeyService(registrar: reg, defaults: defaults)
        XCTAssertEqual(second.shortcut, custom)
        second.start(onTrigger: {})
        XCTAssertEqual(reg.registered, [custom])
    }

    func testInvalidShortcutRejected() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        svc.start(onTrigger: {})
        let before = reg.registered.count
        XCTAssertEqual(svc.setShortcut(Shortcut(keyCode: 49, modifiers: 0)), .rejected(.noModifier))
        XCTAssertEqual(reg.registered.count, before)
        XCTAssertEqual(svc.shortcut, HotkeyService.defaultShortcut)
        XCTAssertEqual(svc.status, .registered)
        XCTAssertNil(defaults.data(forKey: "com.mpointer.kestrel.hotkeyPanel.shortcut"))
    }

    func testFailureThenRebindRecovers() {
        let reg = FakeRegistrar()
        reg.result = .failure(.alreadyInUse)
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        svc.start(onTrigger: {})
        reg.result = .success(())
        svc.setShortcut(custom)
        XCTAssertEqual(svc.status, .registered)
    }

    func testResetToDefault() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        svc.start(onTrigger: {})
        svc.setShortcut(custom)
        svc.resetToDefault()
        XCTAssertEqual(svc.shortcut, HotkeyService.defaultShortcut)
        XCTAssertEqual(reg.registered.last, HotkeyService.defaultShortcut)
    }

    // MARK: - Which shortcuts are allowed

    func testCommandOnlyAndShiftOnlyAreRefused() {
        XCTAssertEqual(Shortcut(keyCode: 40, modifiers: UInt32(cmdKey)).problem, .needsControlOrOption)
        XCTAssertEqual(Shortcut(keyCode: 40, modifiers: UInt32(shiftKey)).problem, .needsControlOrOption)
        XCTAssertEqual(Shortcut(keyCode: 40, modifiers: 0).problem, .noModifier)
    }

    func testControlOrOptionOrTwoModifiersAreAllowed() {
        XCTAssertNil(Shortcut(keyCode: 40, modifiers: UInt32(controlKey)).problem)
        XCTAssertNil(Shortcut(keyCode: 40, modifiers: UInt32(optionKey)).problem)
        XCTAssertNil(Shortcut(keyCode: 40, modifiers: UInt32(cmdKey | shiftKey)).problem)
        XCTAssertTrue(HotkeyService.defaultShortcut.isValid)
    }

    func testCommandOnlyShortcutIsRejectedWithoutTouchingRegistration() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        svc.start(onTrigger: {})
        let log = reg.log
        XCTAssertEqual(svc.setShortcut(Shortcut(keyCode: 40, modifiers: UInt32(cmdKey))), .rejected(.needsControlOrOption))
        XCTAssertEqual(reg.log, log)
        XCTAssertEqual(svc.shortcut, HotkeyService.defaultShortcut)
        XCTAssertNil(defaults.data(forKey: HotkeyService.defaultsKey))
    }

    func testSavedCommandOnlyShortcutFallsBackToDefault() {
        let defaults = makeDefaults()
        defaults.set(try? JSONEncoder().encode(Shortcut(keyCode: 40, modifiers: UInt32(cmdKey))), forKey: HotkeyService.defaultsKey)
        XCTAssertEqual(HotkeyService(registrar: FakeRegistrar(), defaults: defaults).shortcut, HotkeyService.defaultShortcut)
    }

    // MARK: - A change that can't register

    func testAShortcutThatFailsToRegisterKeepsAndReRegistersTheOldOne() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        var fired = 0
        svc.start(onTrigger: { fired += 1 })
        svc.setShortcut(custom)
        let saved = defaults.data(forKey: HotkeyService.defaultsKey)
        let taken = Shortcut(keyCode: 40, modifiers: UInt32(controlKey | optionKey))
        reg.failing = [(taken, .alreadyInUse)]

        XCTAssertEqual(svc.setShortcut(taken), .failed(.alreadyInUse))
        XCTAssertEqual(svc.shortcut, custom, "the old shortcut is still the current one")
        XCTAssertEqual(reg.registered.suffix(2), [taken, custom], "tried the new one, then put the old one back")
        XCTAssertEqual(svc.status, .registered)
        XCTAssertEqual(defaults.data(forKey: HotkeyService.defaultsKey), saved, "the saved value is unchanged")
        reg.handler?()
        XCTAssertEqual(fired, 1, "the restored shortcut still opens the panel")
    }

    func testAFailedChangeWhenTheOldOneCannotComeBackSaysSo() {
        let reg = FakeRegistrar()
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        svc.start(onTrigger: {})
        reg.result = .failure(.alreadyInUse)
        XCTAssertEqual(svc.setShortcut(custom), .failed(.alreadyInUse))
        XCTAssertEqual(svc.shortcut, HotkeyService.defaultShortcut)
        XCTAssertEqual(svc.status, .failed(.alreadyInUse))
    }

    func testAFailedChangeWhileTheOldOneWasNotRegisteredDoesNotRetryIt() {
        let reg = FakeRegistrar()
        reg.result = .failure(.alreadyInUse)
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        svc.start(onTrigger: {})
        let before = reg.registered.count
        XCTAssertEqual(svc.setShortcut(custom), .failed(.alreadyInUse))
        XCTAssertEqual(reg.registered.count, before + 1, "only the new shortcut was tried")
        XCTAssertEqual(svc.shortcut, HotkeyService.defaultShortcut)
        XCTAssertNil(defaults.data(forKey: HotkeyService.defaultsKey))
    }

    func testBeforeStartAChangeIsSavedForStartToRegister() {
        let reg = FakeRegistrar()
        let defaults = makeDefaults()
        let svc = HotkeyService(registrar: reg, defaults: defaults)
        XCTAssertEqual(svc.setShortcut(custom), .applied)
        XCTAssertTrue(reg.registered.isEmpty)
        XCTAssertEqual(svc.status, .off)
        svc.start(onTrigger: {})
        XCTAssertEqual(reg.registered, [custom])
    }

    func testResetToDefaultThatFailsKeepsTheCustomShortcut() {
        let reg = FakeRegistrar()
        let svc = HotkeyService(registrar: reg, defaults: makeDefaults())
        svc.start(onTrigger: {})
        svc.setShortcut(custom)
        reg.failing = [(HotkeyService.defaultShortcut, .alreadyInUse)]
        XCTAssertEqual(svc.resetToDefault(), .failed(.alreadyInUse))
        XCTAssertEqual(svc.shortcut, custom)
        XCTAssertEqual(svc.status, .registered)
    }
}
