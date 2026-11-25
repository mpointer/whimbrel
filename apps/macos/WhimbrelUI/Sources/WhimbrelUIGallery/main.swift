import AppKit
import SwiftUI
import WhimbrelUI

// Renders the gallery to PNG files, offscreen. Usage:
//   WhimbrelUIGallery --out <folder> [--width 1200] [--scale 2]
//   WhimbrelUIGallery --home --out <folder>
// renders Home (default skin light and dark, each element choice, empty, loading, error, capture paused, a live
// recording, 200 percent text, high contrast) and the shell, made-up data, at 900 points wide (the shell at 1200), 2x.
//   WhimbrelUIGallery --live --out <folder> --scale 1
// renders the live meeting screen (recording, paused, resuming, starting, processing, problem, read only, with and
// without the transcript column, 200 percent text, high contrast) and the in-meeting window (made-up data).
//   WhimbrelUIGallery --note --out <folder> --scale 1
// renders the finished note (with sources, without, narrow, processing, regenerating, another device's meeting, editing,
// failed, 200 percent text, a larger reading size, high contrast) and the transcript with speaker lanes, a continued
// meeting and a crowded one (made-up data), each in light and dark where it says so.
//   WhimbrelUIGallery --ask --out <folder> --scale 1
// renders Library's Find page (empty, results, answered with sources, streaming, no results, a person with mentions, failed, no model,
// an older saved answer, a source no longer on this device, narrow, 200 percent text, high contrast) and Recent, Episodes and Digest
// (made-up data), in light and dark where it says so. (--find is the same.)
//   WhimbrelUIGallery --conversations --out <folder> --scale 1
// renders the conversation page's header (a Find conversation reopened with its kept Apps and Dates, the same with the People
// filter not kept note, a conversation asked about one topic), made-up data, light and dark.
//   WhimbrelUIGallery --menu --out <folder> --scale 1
// renders the menu bar menu, the launch menu, the meeting alert and check-in windows and the hotkey panel (made-up data),
// each in light and dark, the short list again in high contrast and at 200 percent text.
//   WhimbrelUIGallery --settings --out <folder> --scale 1
// renders the redesigned Settings window: the chrome (search, section list or picker, detail column) with General, Profile
// and Plan & billing from sample models (signed out, free, Plus; search with and without results; narrow; light, dark,
// high contrast, light at 200 percent text). Write to docs/ui-redesign/screenshots/after/mac/settings/.
//   WhimbrelUIGallery --onboarding --out <folder>
// renders the settings parts (nav and search results, rows, secrets, lists, values, chips and notices, sheets and confirms,
// skin cards; made-up data), each page in light, dark, high contrast and light at 200 percent text. Write to
// docs/ui-redesign/screenshots/after/mac/settings-parts/.
//   WhimbrelUIGallery --onboarding --out <folder>
// renders the first run, launch and setup screens (made-up data) at 900 points wide, 2x:
// <screen>-light, -dark, -light-2.0 (text at 200 percent) and -high-contrast.
// Without --onboarding it writes gallery-<mode>-<text scale>.png for the four modes at 1.0 and light at 2.0, plus
// gallery-light-1.0-custom-skin.png, which shows that a skin can change shape and type.

struct Job {
    let name: String
    let mode: WGalleryMode
    let textScale: CGFloat
    var shape: WShape = .thread
    var type: WTypeStyle = .thread
}

var outFolder = "."
var width: CGFloat = 1200
var pixelScale: CGFloat = 2
var onboarding = false
var home = false
var live = false
var note = false
var ask = false
var conversations = false
var menu = false
var settingsParts = false
var settingsWindow = false
var args = Array(CommandLine.arguments.dropFirst())
while !args.isEmpty {
    let flag = args.removeFirst()
    switch flag {
    case "--out": if !args.isEmpty { outFolder = args.removeFirst() }
    case "--width": if !args.isEmpty { width = CGFloat(Double(args.removeFirst()) ?? 1200) }
    case "--onboarding": onboarding = true
    case "--home": home = true
    case "--live": live = true
    case "--note": note = true
    case "--ask", "--find": ask = true
    case "--conversations": conversations = true
    case "--menu": menu = true
    case "--settings-parts": settingsParts = true
    case "--settings": settingsWindow = true
    case "--scale": if !args.isEmpty { pixelScale = CGFloat(Double(args.removeFirst()) ?? 2) }
    default:
        FileHandle.standardError.write(Data("Unknown argument \(flag)\n".utf8))
        exit(2)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)

var jobs: [Job] = WGalleryMode.allCases.map { Job(name: "gallery-\($0.rawValue)-1.0", mode: $0, textScale: 1) }
jobs.append(Job(name: "gallery-light-2.0", mode: .light, textScale: 2))
jobs.append(Job(
    name: "gallery-light-1.0-custom-skin", mode: .light, textScale: 1,
    shape: WShape(controlRadius: 2, cardRadius: 4, surfaceRadius: 6, borderWidth: 1, borderStyle: .dashed, density: .compact),
    type: WTypeStyle(displayFamily: .serif, bodyFamily: .rounded, readingFamily: .monospaced, sectionLabelCaps: false)
))

@MainActor
func render(_ job: Job) throws {
    let root = WGallery(mode: job.mode, textScale: job.textScale, shape: job.shape, type: job.type, scrollable: false, width: width)
    try render(name: job.name, root: root, mode: job.mode, minHeight: 800)
}

/// One window-like render: the view at `width`, at least `minHeight` tall, in the mode's appearance.
@MainActor
func render(name: String, root: some View, mode: WGalleryMode, minHeight: CGFloat) throws {
    let host = NSHostingView(rootView: root)
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: minHeight), styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: mode.variant.isDark ? .darkAqua : .aqua)
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    var size = host.fittingSize
    size.width = width
    window.setContentSize(size)
    host.frame = NSRect(origin: .zero, size: size)
    host.layoutSubtreeIfNeeded()
    RunLoop.current.run(until: Date().addingTimeInterval(0.4))
    host.layoutSubtreeIfNeeded()
    size = host.fittingSize
    size.width = width
    window.setContentSize(size)
    host.frame = NSRect(origin: .zero, size: size)
    host.layoutSubtreeIfNeeded()
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))

    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * pixelScale), pixelsHigh: Int(size.height * pixelScale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { throw NSError(domain: "gallery", code: 1) }
    rep.size = size
    host.cacheDisplay(in: host.bounds, to: rep)
    guard let png = rep.representation(using: .png, properties: [:]) else { throw NSError(domain: "gallery", code: 2) }
    let url = URL(fileURLWithPath: outFolder).appendingPathComponent("\(name).png")
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try png.write(to: url)
    print("\(url.path)  \(Int(size.width * pixelScale)) x \(Int(size.height * pixelScale)) px")
}

@MainActor
func renderOnboarding() throws {
    let variants: [(suffix: String, mode: WGalleryMode, scale: CGFloat)] = [
        ("light", .light, 1), ("dark", .dark, 1), ("light-2.0", .light, 2), ("high-contrast", .highContrastLight, 1),
    ]
    for screen in WOnboardingGallery.screens() {
        for v in variants {
            let theme = WTheme.thread(variant: v.mode.variant)
            let root = screen.view
                .environment(\.wTheme, theme)
                .environment(\.wTextScale, v.scale)
                .environment(\.wScrollsContent, false)
                .preferredColorScheme(v.mode.variant.colorScheme)
                .frame(width: width)
                .frame(minHeight: 640, alignment: .top)
                .background(theme.backgroundPage)
            try render(name: "\(screen.id)-\(v.suffix)", root: root, mode: v.mode, minHeight: 640)
        }
    }
}

@MainActor
func renderHome() throws { try renderCases(WHomeGallery.cases()) }

@MainActor
func renderLive() throws { try renderCases(WLiveGallery.cases()) }

@MainActor
func renderNote() throws { try renderCases(WNoteGallery.cases()) }

@MainActor
func renderAsk() throws { try renderCases(WFindGallery.cases()) }

@MainActor
func renderConversations() throws { try renderCases(WConversationGallery.cases()) }

@MainActor
func renderMenu() throws {
    for item in WMenuGallery.cases() {
        let theme = WTheme.thread(variant: item.mode.variant)
        let root = item.view
            .padding(24)
            .environment(\.wTheme, theme)
            .environment(\.wTextScale, item.textScale)
            .environment(\.wScrollsContent, false)
            .preferredColorScheme(item.mode.variant.colorScheme)
            .frame(width: item.width)
            .fixedSize(horizontal: false, vertical: true)
            .background(theme.backgroundPage)
        width = item.width
        try render(name: item.id, root: root, mode: item.mode, minHeight: 60)
    }
}

@MainActor
func renderSettingsParts() throws {
    for item in WSettingsPartsGallery.cases() {
        let theme = WTheme.thread(variant: item.mode.variant)
        let root = item.view
            .padding(24)
            .environment(\.wTheme, theme)
            .environment(\.wTextScale, item.textScale)
            .environment(\.wScrollsContent, false)
            .environment(\.wDisabledButtonLook, true)
            .preferredColorScheme(item.mode.variant.colorScheme)
            .frame(width: item.width, alignment: .topLeading)
            .fixedSize(horizontal: false, vertical: true)
            .background(theme.backgroundPage)
        width = item.width
        try render(name: item.id, root: root, mode: item.mode, minHeight: 60)
    }
}

@MainActor
func renderCases(_ cases: [WHomeGalleryCase]) throws {
    for item in cases {
        let theme = WTheme.thread(variant: item.mode.variant)
        let root = item.view
            .environment(\.wTheme, theme)
            .environment(\.wTextScale, item.textScale)
            .environment(\.wScrollsContent, false)
            .preferredColorScheme(item.mode.variant.colorScheme)
            .frame(width: item.width, height: item.height)
            .background(theme.backgroundPage)
        width = item.width
        try render(name: item.id, root: root, mode: item.mode, minHeight: item.height)
    }
}

MainActor.assumeIsolated {
    if settingsWindow {
        do { try renderCases(WSettingsWindowGallery.cases()) } catch {
            FileHandle.standardError.write(Data("Failed settings window render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if settingsParts {
        do { try renderSettingsParts() } catch {
            FileHandle.standardError.write(Data("Failed settings parts render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if menu {
        do { try renderMenu() } catch {
            FileHandle.standardError.write(Data("Failed menu render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if conversations {
        do { try renderConversations() } catch {
            FileHandle.standardError.write(Data("Failed conversations render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if ask {
        do { try renderAsk() } catch {
            FileHandle.standardError.write(Data("Failed ask render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if note {
        do { try renderNote() } catch {
            FileHandle.standardError.write(Data("Failed note render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if live {
        do { try renderLive() } catch {
            FileHandle.standardError.write(Data("Failed live render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if home {
        do { try renderHome() } catch {
            FileHandle.standardError.write(Data("Failed home render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    if onboarding {
        if width == 1200 { width = 900 }
        do { try renderOnboarding() } catch {
            FileHandle.standardError.write(Data("Failed onboarding render: \(error)\n".utf8))
            exit(1)
        }
        exit(0)
    }
    for job in jobs {
        do { try render(job) } catch {
            FileHandle.standardError.write(Data("Failed \(job.name): \(error)\n".utf8))
            exit(1)
        }
    }
}
