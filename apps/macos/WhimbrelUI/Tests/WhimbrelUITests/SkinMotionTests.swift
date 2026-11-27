import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Richer skins, row 10 (docs/superpowers/plans/2026-11-25-richer-skins.md, section 6): one motion token set. `none` and Reduce Motion
/// give zero durations, `subtle` gives the plan's values, Thread keeps what it has, and the recording indicator and the consent banner
/// never read the skin's motion. The expected values are `skins/testdata/expected/motion.json`, which Windows tests too.
@Suite struct SkinMotionTests {
    private struct Case: Decodable {
        let motion: String
        let reduceMotion: Bool
        let section: Double
        let rise: Double
        let control: Double
        let marker: Double
        let panel: Double
    }

    private struct Fixture: Decodable { let cases: [Case] }

    private static let repo = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

    private func fixture() throws -> [Case] {
        let data = try Data(contentsOf: Self.repo.appendingPathComponent("skins/testdata/expected/motion.json"))
        return try JSONDecoder().decode(Fixture.self, from: data).cases
    }

    @Test func resolvedValuesMatchTheSharedFixture() throws {
        let cases = try fixture()
        #expect(cases.count == 4)
        for c in cases {
            let m = WMotion.resolve(subtle: c.motion == "subtle", reduceMotion: c.reduceMotion)
            #expect(Double(m.section) == c.section / 1000, "\(c.motion) \(c.reduceMotion)")
            #expect(Double(m.rise) == c.rise)
            #expect(m.control == c.control / 1000)
            #expect(m.marker == c.marker / 1000)
            #expect(m.panel == c.panel / 1000)
        }
    }

    @Test func noneAndReduceMotionGiveZeroDurations() {
        for m in [WMotion.none, WMotion.subtle.respecting(reduceMotion: true), WMotion.resolve(subtle: true, reduceMotion: true), .thread] {
            #expect(m == WMotion())
            #expect(WMotion.animation(m.section) == nil)
            #expect(WMotion.animation(m.control) == nil)
            #expect(WMotion.animation(m.marker) == nil)
            #expect(WMotion.animation(m.panel) == nil)
        }
        #expect(WMotion.subtle.respecting(reduceMotion: false) == .subtle)
        #expect(WMotion.animation(0.12) != nil)
    }

    @Test func aThemeDefaultsToThreadMotion() {
        #expect(WTheme.thread(variant: .light).motion == .thread)
        #expect(WTheme(palette: [:], variant: .light, motion: .subtle).motion == .subtle)
    }

    @Test func theRecordingIndicatorAndConsentBannerNeverReadTheSkinsMotion() throws {
        let sources = Self.repo.appendingPathComponent("apps/macos/WhimbrelUI/Sources/WhimbrelUI")
        let names = try FileManager.default.contentsOfDirectory(atPath: sources.path)
            .filter { file in file.hasSuffix(".swift") && ["Recording", "Capture", "Consent"].contains { file.hasPrefix($0) } }
        #expect(!names.isEmpty)
        for name in names {
            let text = try String(contentsOf: sources.appendingPathComponent(name), encoding: .utf8)
            #expect(!text.contains("theme.motion"), "\(name)")
            #expect(!text.contains("wMotion"), "\(name)")
            #expect(!text.contains("wSectionAppear"), "\(name)")
        }
    }
}
