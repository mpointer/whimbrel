import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// Renders `view` at scale 2 and compares it with
/// `WhimbrelTests/__Snapshots__/<named>.png`, allowing at most `tolerance` of the pixels to differ
/// (0.2% by default; the standard Today passes 0.01% so small font, weight or radius drift fails).
/// A missing baseline is recorded and fails with "recorded baseline". Skipped unless
/// `WHIMBREL_SNAPSHOTS=1` (pass `TEST_RUNNER_WHIMBREL_SNAPSHOTS=1` to xcodebuild).
// ponytail: local-only; CI runner fonts differ. Pin a runner image if snapshots must gate CI.
@MainActor
func assertSnapshot(
    _ view: some View, named: String, size: CGSize, colorScheme: ColorScheme, tolerance: Double = 0.002,
    file: StaticString = #filePath, line: UInt = #line
) throws {
    guard ProcessInfo.processInfo.environment["WHIMBREL_SNAPSHOTS"] == "1" else {
        throw XCTSkip("Set WHIMBREL_SNAPSHOTS=1 to run snapshot tests")
    }
    guard let rep = OffscreenRender.bitmap(of: view, size: size, colorScheme: colorScheme, scale: 2) else {
        XCTFail("Could not render \(named)", file: file, line: line)
        return
    }
    guard let cgImage = rep.cgImage else {
        XCTFail("Could not render \(named)", file: file, line: line)
        return
    }
    let directory = snapshotsDirectory
    let url = directory.appendingPathComponent("\(named).png")
    guard let png = NSBitmapImageRep(cgImage: cgImage).representation(using: .png, properties: [:]) else {
        XCTFail("Could not encode \(named)", file: file, line: line)
        return
    }
    guard FileManager.default.fileExists(atPath: url.path) else {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try png.write(to: url)
        XCTFail("recorded baseline \(url.lastPathComponent)", file: file, line: line)
        return
    }
    guard let baseline = NSBitmapImageRep(data: try Data(contentsOf: url))?.cgImage else {
        XCTFail("Unreadable baseline \(url.lastPathComponent)", file: file, line: line)
        return
    }
    let differing = differingPixels(cgImage, baseline)
    let total = cgImage.width * cgImage.height
    if differing == nil || Double(differing!) / Double(total) > tolerance {
        let actual = directory.appendingPathComponent("\(named).actual.png")
        try? png.write(to: actual)
        XCTFail(
            "\(named) differs from baseline (\(differing.map { "\($0) of \(total) pixels" } ?? "size mismatch")); wrote \(actual.lastPathComponent)",
            file: file, line: line
        )
    }
}

/// `WhimbrelTests/__Snapshots__`, found from this file's compile-time path (the test host has no source tree).
private let snapshotsDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("__Snapshots__")

/// Count of pixels whose RGBA differs by more than a few levels; nil when the sizes differ.
private func differingPixels(_ a: CGImage, _ b: CGImage) -> Int? {
    guard a.width == b.width, a.height == b.height else { return nil }
    func bytes(_ image: CGImage) -> [UInt8] {
        var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
        data.withUnsafeMutableBytes { buffer in
            let context = CGContext(
                data: buffer.baseAddress, width: image.width, height: image.height, bitsPerComponent: 8,
                bytesPerRow: image.width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
            context?.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        }
        return data
    }
    let (p, q) = (bytes(a), bytes(b))
    var count = 0
    for pixel in 0..<(p.count / 4) {
        let i = pixel * 4
        if (0..<4).contains(where: { abs(Int(p[i + $0]) - Int(q[i + $0])) > 4 }) { count += 1 }
    }
    return count
}
