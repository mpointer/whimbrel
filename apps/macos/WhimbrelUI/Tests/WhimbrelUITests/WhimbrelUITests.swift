import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

@Suite struct RecordingTextTests {
    @Test func labelPerState() {
        #expect(WRecordingText.label(state: .recording, elapsedSeconds: 848) == "Recording, 14 minutes 8 seconds")
        #expect(WRecordingText.label(state: .paused, elapsedSeconds: 848) == "Paused, 14 minutes 8 seconds")
        #expect(WRecordingText.label(state: .recording, elapsedSeconds: 3723) == "Recording, 1 hour 2 minutes 3 seconds")
        #expect(WRecordingText.label(state: .recording, elapsedSeconds: 61) == "Recording, 1 minute 1 second")
        #expect(WRecordingText.label(state: .recording, elapsedSeconds: nil) == "Recording")
        #expect(WRecordingText.label(state: .processing, elapsedSeconds: nil) == "Processing")
        #expect(WRecordingText.label(state: .off, elapsedSeconds: nil) == "Not recording")
        #expect(WRecordingText.label(state: .problem("Microphone not found"), elapsedSeconds: nil) == "Problem, Microphone not found")
    }

    @Test func clock() {
        #expect(WRecordingText.clock(seconds: 848) == "14:08")
        #expect(WRecordingText.clock(seconds: 3723) == "1:02:03")
    }

    @Test func everyStateHasItsOwnGlyphAndWord() {
        let states: [WRecordingState] = [.recording, .paused, .processing, .off, .problem("x")]
        #expect(Set(states.map(\.glyph)).count == states.count)
        #expect(Set(states.map(\.word)).count == states.count)
    }

    @Test func captureStateTitles() {
        #expect(WMemoryState.on.title == "Memory on")
        #expect(WMemoryState.pausedUntil("10:52").title == "Memory paused until 10:52")
        #expect(WMemoryState.off.title == "Memory off")
    }
}

@Suite struct ConsentTests {
    @Test func neverClaimsAnyoneWasToldOrAgreed() {
        for text in WConsentStrings.all {
            let lower = text.lowercased()
            for word in ["notified", "consented", "agreed"] {
                #expect(!lower.contains(word), "\(text) contains \(word)")
            }
        }
        #expect(WConsentStrings.message == "You are recording. Tell the others before you go on.")
        #expect(WStrings.english.consentMessage == WConsentStrings.message)
    }
}

@Suite struct StringsTests {
    /// A WStrings that marks every string, so a test can see that a component read it from there.
    static let marked: WStrings = {
        var s = WStrings.english
        s.recording = "[recording]"
        s.paused = "[paused]"
        s.problem = "[problem]"
        s.off = "[off]"
        s.pausedUntil = { "[until \($0)]" }
        s.hours = { "[\($0)h]" }
        s.minutes = { "[\($0)m]" }
        s.seconds = { "[\($0)s]" }
        s.joinDetails = { $0.joined(separator: " / ") }
        s.joinDuration = { $0.joined(separator: "+") }
        s.sourceNumber = { "[source \($0)]" }
        s.quoted = { "<<\($0)>>" }
        return s
    }()

    @Test func theDefaultIsEnglishAndTextComesFromTheValue() {
        #expect(EnvironmentValues().wStrings.recording == "Recording")
        #expect(WRecordingText.spoken(seconds: 3723) == "1 hour 2 minutes 3 seconds")
        #expect(WRecordingText.spoken(seconds: 3723, strings: Self.marked) == "[1h]+[2m]+[3s]")
        #expect(WRecordingText.label(state: .recording, elapsedSeconds: 61, strings: Self.marked) == "[recording] / [1m]+[1s]")
        #expect(WRecordingText.label(state: .problem("x"), elapsedSeconds: nil, strings: Self.marked) == "[problem] / x")
        #expect(WRecordingText.label(state: .off, elapsedSeconds: nil, strings: Self.marked) == "[off]")
        #expect(WRecordingState.paused.word(using: Self.marked) == "[paused]")
        #expect(WMemoryState.pausedUntil("10:52").title(using: Self.marked) == "[until 10:52]")
        #expect(WSourceKind.seenOnScreen.label(using: .english) == "Seen on screen")
        #expect(WSourceKind.email.label(using: .english) == "Email")
        #expect(WSourceKind.routineRun.label(using: .english) == "Routine run")
        #expect(Self.marked.sourceNumber(2) == "[source 2]")
        #expect(Self.marked.quoted("a") == "<<a>>")
    }

    @Test func englishHasNoEmptyString() {
        let mirror = Mirror(reflecting: WStrings.english)
        var count = 0
        for child in mirror.children {
            if let text = child.value as? String {
                count += 1
                #expect(!text.trimmingCharacters(in: .whitespaces).isEmpty, "\(child.label ?? "?") is empty")
            }
        }
        #expect(count > 30)
    }

    @Test func theEnglishConsentStringsNeverClaimAnyoneWasToldOrAgreed() {
        for text in WConsentStrings.texts(in: .english) {
            for word in ["notified", "consented", "agreed", "were told", "was told"] {
                #expect(!text.lowercased().contains(word), "\(text) contains \(word)")
            }
        }
    }
}

@Suite struct ThemeTests {
    @Test func threadReturnsTheGeneratedValuesForEachVariant() {
        let expected: [(ColorScheme, ColorSchemeContrast, [String: UInt32], WVariant)] = [
            (.light, .standard, WThreadPalette.light, .light),
            (.dark, .standard, WThreadPalette.dark, .dark),
            (.light, .increased, WThreadPalette.highContrastLight, .highContrastLight),
            (.dark, .increased, WThreadPalette.highContrastDark, .highContrastDark),
        ]
        for (scheme, contrast, palette, variant) in expected {
            let theme = WTheme.thread(colorScheme: scheme, contrast: contrast)
            #expect(theme.variant == variant)
            #expect(theme.palette == palette)
            for (name, fixed) in WTheme.fixedColors {
                #expect(theme.fixed[name] == fixed.value(for: variant), "\(name) in \(variant)")
            }
            #expect(theme.fixed.count == 12)
            #expect(theme.palette.count == 15)
        }
    }

    @Test func fixedValuesMatchTokens() {
        #expect(WTheme.thread(variant: .light).fixed["recording"] == WFixed.recording.light)
        #expect(WTheme.thread(variant: .highContrastDark).fixed["aiInk"] == WFixed.aiInk.highContrastDark)
    }

    @Test func appPaletteOverridesAndFallsBack() {
        let theme = WTheme(palette: ["accentPrimary": 0x112233], colorScheme: .dark, contrast: .standard)
        #expect(theme.palette["accentPrimary"] == 0x112233)
        #expect(theme.palette["backgroundPage"] == WThreadPalette.dark["backgroundPage"])
        #expect(theme.fixed["recording"] == WFixed.recording.dark)
    }

    @Test func shapeAndTypeDefaultToThreadTokens() {
        let theme = WTheme.thread(variant: .light)
        #expect(theme.shape.controlRadius == WTokens.Radius.control)
        #expect(theme.shape.cardRadius == WTokens.Radius.card)
        #expect(theme.shape.surfaceRadius == WTokens.Radius.surface)
        #expect(theme.shape.density == .regular)
        #expect(theme.type.displayFamily == .system)
        #expect(theme.type.bodyFamily == .system)
        #expect(theme.type.readingFamily == .serif)
        #expect(theme.type.sectionLabelCaps)
    }

    @Test func skinShapeAndTypeAreCarried() {
        let shape = WShape(controlRadius: 2, cardRadius: 3, borderWidth: 2, borderStyle: .dashed, density: .roomy)
        let type = WTypeStyle(displayFamily: .named("Avenir Next"), bodyFamily: .rounded, sectionLabelCaps: false)
        let theme = WTheme(palette: [:], variant: .light, shape: shape, type: type)
        #expect(theme.shape.controlRadius == 2)
        #expect(theme.space(10) == 12.5)
        #expect(theme.borderWidth == 2)
        #expect(theme.sectionLabel("Risks") == "Risks")
        #expect(WTheme.thread(variant: .light).sectionLabel("Risks") == "RISKS")
        // high contrast always draws a border, even for a skin with none
        let none = WTheme(palette: [:], variant: .highContrastLight, shape: WShape(borderStyle: .none))
        #expect(none.borderWidth > 0)
        #expect(WTheme(palette: [:], variant: .light, shape: WShape(borderStyle: .none)).borderWidth == 0)
    }
}

@Suite struct TextScaleTests {
    @Test func scaleMultipliesSizesAndLineHeights() {
        for role in WTextRole.allCases {
            #expect(role.size(scale: 2) == role.token.size * 2)
            #expect(role.lineHeight(scale: 2) == role.token.lineHeight * 2)
            #expect(role.size() == role.token.size)
        }
    }

    @Test func rolesFollowTheirFamily() {
        let thread = WTypeStyle.thread
        #expect(WTextRole.reading.family(in: thread) == .serif)
        #expect(WTextRole.readingTitle.family(in: thread) == .serif)
        #expect(WTextRole.body.family(in: thread) == .system)
        #expect(WTextRole.title.family(in: thread) == .system)
        let skin = WTypeStyle(displayFamily: .rounded, bodyFamily: .monospaced, readingFamily: .system)
        #expect(WTextRole.title.family(in: skin) == .rounded)
        #expect(WTextRole.body.family(in: skin) == .monospaced)
        #expect(WTextRole.reading.family(in: skin) == .system)
    }

    @Test func defaultScaleIsOne() {
        #expect(EnvironmentValues().wTextScale == 1)
    }
}

@Suite struct CopyTests {
    private func sourceFiles() throws -> [URL] {
        let here = URL(fileURLWithPath: #filePath)
        let sources = here.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Sources")
        let enumerator = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil)
        var files: [URL] = []
        while let url = enumerator?.nextObject() as? URL { if url.pathExtension == "swift" { files.append(url) } }
        return files
    }

    @Test func noEmOrEnDashInTheSources() throws {
        let files = try sourceFiles()
        #expect(files.count > 5)
        for file in files {
            let text = try String(contentsOf: file, encoding: .utf8)
            #expect(!text.contains("\u{2014}"), "em dash in \(file.lastPathComponent)")
            #expect(!text.contains("\u{2013}"), "en dash in \(file.lastPathComponent)")
        }
    }

    /// The rule, in plain words. In every file of the package except the three below, a string literal
    /// must not be words a person reads. After comments are removed:
    /// 1. A literal with no letters once `\(...)`, `\u{...}` and `%` formats are taken out ("%d:%02d",
    ///    "\u{2022}", "\(number)") is not text.
    /// 2. A literal that is a symbol name or a token key (`circle.fill`, `backgroundPage`: no space, no
    ///    capital first letter) is not text, unless it sits directly in `Text(`, `Button(`, `WButton(`,
    ///    `Label(`, `Toggle(`, `TextField(`, `accessibilityLabel(`, `accessibilityHint(`,
    ///    `accessibilityValue(`, `Announcement(` or `.help(`, or is the default of a `String` parameter.
    /// 3. Everything else is text, and text lives in WStrings.swift.
    /// Exempt files: WStrings.swift and SettingsStrings.swift and SettingsPaneStrings.swift (the home of the English), WGallery.swift, SettingsPartsGallery.swift and SettingsWindowGallery.swift (developer-only labels and
    /// sample data) and Tokens.generated.swift (generated, no UI). Exempt literals: the keyboard hint.
    static let exemptFiles: Set<String> = ["WStrings.swift", "WGallery.swift", "Tokens.generated.swift", "SettingsStrings.swift", "SettingsPartsGallery.swift", "SettingsPaneStrings.swift", "SettingsWindowGallery.swift", "SettingsMemoryStrings.swift", "SettingsMemoryGallery.swift", "SettingsMeetingsAIStrings.swift", "SettingsMeetingsAIGallery.swift", "SettingsSyncStrings.swift", "SettingsSyncGallery.swift", "FindQuestionWords.swift"]  // FindQuestionWords: words matched against a typed query, never shown
    static let exemptLiterals: Set<String> = ["\\u{2318}K", "\\u{21E7}\\u{2318}M", "\\u{2318}T"]  // the command-K keycap: notation, not a word

    static func withoutComments(_ source: String) -> String {
        var out = ""
        var inString = false
        let chars = Array(source)
        var i = 0
        while i < chars.count {
            let c = chars[i]
            if inString {
                out.append(c)
                if c == "\\", i + 1 < chars.count { out.append(chars[i + 1]); i += 2; continue }
                if c == "\"" { inString = false }
            } else if c == "\"" {
                inString = true
                out.append(c)
            } else if c == "/", i + 1 < chars.count, chars[i + 1] == "/" {
                while i < chars.count, chars[i] != "\n" { i += 1 }
                continue
            } else {
                out.append(c)
            }
            i += 1
        }
        return out
    }

    /// Every violation of the rule in one file's source, as "line N: literal".
    static func userFacingLiterals(in source: String) -> [String] {
        let clean = withoutComments(source)
        let literal = try! NSRegularExpression(pattern: #""((?:[^"\\]|\\.)*)""#)
        let sinks = try! NSRegularExpression(pattern: #"(?:\b(?:Text|Button|WButton|Label|Toggle|TextField|accessibilityLabel|accessibilityHint|accessibilityValue|Announcement)\(|\.help\(|\bString\??\s*=\s*)\s*$"#)
        let symbolDefault = try! NSRegularExpression(pattern: #"(?i)(?:glyph|icon|symbol)\s*:\s*String\??\s*=\s*$"#)
        let symbolOrKey = try! NSRegularExpression(pattern: #"^[a-z][A-Za-z0-9]*(\.[a-z0-9]+)*$"#)
        let ns = clean as NSString
        var found: [String] = []
        for match in literal.matches(in: clean, range: NSRange(location: 0, length: ns.length)) {
            let body = ns.substring(with: match.range(at: 1))
            var letters = body
            for pattern in [#"\\\(.*?\)"#, #"\\u\{[0-9A-Fa-f]+\}"#, #"%(?:\d+\$)?[-+ #0']*\d*(?:\.\d+)?[a-zA-Z@]{1,2}"#] {
                letters = letters.replacingOccurrences(of: pattern, with: "", options: .regularExpression)
            }
            guard letters.range(of: "[A-Za-z]", options: .regularExpression) != nil else { continue }
            if exemptLiterals.contains(body) { continue }
            let before = ns.substring(to: match.range.location)
            let beforeRange = NSRange(location: 0, length: (before as NSString).length)
            // A parameter named glyph, icon or symbol that defaults to an SF Symbol name is not text.
            let isSymbolDefault = symbolDefault.firstMatch(in: before, range: beforeRange) != nil
            let isSink = !isSymbolDefault && sinks.firstMatch(in: before, range: beforeRange) != nil
            let isSymbolOrKey = symbolOrKey.firstMatch(in: body, range: NSRange(location: 0, length: (body as NSString).length)) != nil
            if isSymbolOrKey && !isSink { continue }
            let line = before.reduce(1) { $1 == "\n" ? $0 + 1 : $0 }
            found.append("line \(line): \"\(body)\"")
        }
        return found
    }

    @Test func noUserFacingLiteralsOutsideWStrings() throws {
        // Only the library target: the gallery executable is a developer tool with its own command line.
        let files = try sourceFiles().filter {
            !Self.exemptFiles.contains($0.lastPathComponent) && $0.pathComponents.contains("WhimbrelUI") && !$0.pathComponents.contains("WhimbrelUIGallery")
        }
        #expect(files.count >= 8)
        for file in files {
            let text = try String(contentsOf: file, encoding: .utf8)
            let violations = Self.userFacingLiterals(in: text)
            #expect(violations.isEmpty, "\(file.lastPathComponent) has user-facing text outside WStrings: \(violations)")
        }
    }

    @Test func theScanRuleCatchesWhatItShould() {
        // Each of these is a way a word could slip back into a component.
        #expect(!Self.userFacingLiterals(in: #"Text("Recording")"#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #".accessibilityHint("Selects this source")"#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #"let x = state == .live ? ", listening" : """#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #"public init(title: String = "Loading") {}"#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #"Label("Pause", systemImage: "pause")"#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #"Text("listening")"#).isEmpty)
        #expect(!Self.userFacingLiterals(in: #"let s = "Whimbrel wrote: \(text)""#).isEmpty)
        // And these are fine: symbols, token keys, numbers, comments.
        #expect(Self.userFacingLiterals(in: #"WGlyph(name: "circle.fill")"#).isEmpty)
        #expect(Self.userFacingLiterals(in: #"init(appGlyph: String = "waveform")"#).isEmpty)
        #expect(Self.userFacingLiterals(in: #"palette["backgroundPage"]"#).isEmpty)
        #expect(Self.userFacingLiterals(in: #"String(format: "%d:%02d", m, s)"#).isEmpty)
        #expect(Self.userFacingLiterals(in: #"Text("\(number)")"#).isEmpty)
        #expect(Self.userFacingLiterals(in: "// Text(\"Recording\")\n/// \"Now\" on the time line\nlet a = 1").isEmpty)
        #expect(Self.userFacingLiterals(in: #"shortcutHint: String? = "\u{2318}K""#).isEmpty)
    }

    @Test func initialsFromNames() {
        #expect(WPerson(name: "Alex Rivera").initials == "AR")
        #expect(WPerson(name: "Jo at Northwind").initials == "JN")
        #expect(WPerson(name: "Sam Okafor").initials == "SO")
    }
}
