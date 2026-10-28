import AVFoundation
import XCTest
@testable import Whimbrel

final class RecordingWriterTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("rw-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    /// 0.25 s of 16 kHz mono Float32.
    private func quarterSecond() -> AVAudioPCMBuffer {
        let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4_000)!
        buffer.frameLength = 4_000
        for i in 0..<4_000 { buffer.floatChannelData![0][i] = 0.1 }
        return buffer
    }

    private func makeWriter(cap: AVAudioFramePosition = 16_000) throws -> (RecordingWriter, URL) {
        let url = dir.appendingPathComponent("a.caf")
        return (RecordingWriter(file: try RecordingWriter.makeFile(at: url), capFrames: cap), url)
    }

    func testWritesUpToTheCapAndNoFurther() throws {
        var (writer, url): (RecordingWriter?, URL) = try makeWriter()
        let buf = quarterSecond()
        for _ in 0..<5 { writer!.write(buf) }
        XCTAssertEqual(writer!.write(buf), 0)
        XCTAssertEqual(writer!.framesWritten, 16_000)
        XCTAssertTrue(writer!.capReached)
        writer = nil
        XCTAssertEqual(try AVAudioFile(forReading: url).length, 16_000)
    }

    func testPartialBufferIsTruncatedWithoutMutatingCaller() throws {
        let (writer, _) = try makeWriter(cap: 6_000)
        let buf = quarterSecond()
        XCTAssertEqual(writer.write(buf), 4_000)
        XCTAssertEqual(writer.write(buf), 2_000)
        XCTAssertTrue(writer.capReached)
        XCTAssertEqual(buf.frameLength, 4_000)
    }

    func testSilencePadLargerThanTheRoomLeftIsTruncated() throws {
        let (writer, _) = try makeWriter()
        writer.write(quarterSecond())
        XCTAssertEqual(writer.writeSilence(frames: 1_000_000), 12_000)
        XCTAssertEqual(writer.framesWritten, 16_000)
        XCTAssertTrue(writer.capReached)
    }

    /// A pad whose chunk writes fail must not claim the cap: nothing reached the file.
    func testFailedSilenceWriteDoesNotReportCap() throws {
        let url = dir.appendingPathComponent("r.caf")
        var made: RecordingWriter? = RecordingWriter(file: try RecordingWriter.makeFile(at: url), capFrames: 16_000)
        made!.write(quarterSecond())
        made = nil
        // Opened for reading, so every write throws.
        let writer = RecordingWriter(file: try AVAudioFile(forReading: url), capFrames: 1_000)
        XCTAssertEqual(writer.writeSilence(frames: 5_000), 0)
        XCTAssertEqual(writer.framesWritten, 0)
        XCTAssertFalse(writer.capReached)
    }

    func testFileIsSixteenKilohertzMonoInt16() throws {
        var (writer, url): (RecordingWriter?, URL) = try makeWriter()
        writer!.write(quarterSecond())
        writer = nil
        let format = try AVAudioFile(forReading: url).fileFormat
        XCTAssertEqual(format.sampleRate, 16_000)
        XCTAssertEqual(format.channelCount, 1)
        XCTAssertEqual(format.commonFormat, .pcmFormatInt16)
    }

    #if DEBUG
    func testDebugFreeBytesOverride() {
        UserDefaults.standard.set(123, forKey: "com.mpointer.kestrel.debugFreeBytes")
        defer { UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.debugFreeBytes") }
        XCTAssertEqual(RecordingLimits.freeBytes(), 123)
    }
    #endif

    func testDefaultCapIsSixHours() throws {
        let file = try RecordingWriter.makeFile(at: dir.appendingPathComponent("b.caf"))
        XCTAssertEqual(RecordingWriter(file: file).capFrames, 6 * 3600 * 16_000)
    }

    func testCanStartRefusesBelowFiveGB() {
        XCTAssertFalse(RecordingLimits.canStart(freeBytes: 4_999_999_999))
        XCTAssertTrue(RecordingLimits.canStart(freeBytes: 5_000_000_000))
        XCTAssertTrue(RecordingLimits.canStart(freeBytes: nil))
    }

    func testCheckDiskStopsBelowTwoGB() {
        XCTAssertEqual(RecordingLimits.checkDisk(freeBytes: 1_999_999_999), .lowDisk)
        XCTAssertNil(RecordingLimits.checkDisk(freeBytes: 2_000_000_000))
        XCTAssertNil(RecordingLimits.checkDisk(freeBytes: nil))
    }

    func testFileStoppedMidStreamIsReadable() throws {
        var (writer, url): (RecordingWriter?, URL) = try makeWriter()
        writer!.write(quarterSecond()); writer!.write(quarterSecond())
        writer = nil
        XCTAssertEqual(try AVAudioFile(forReading: url).length, 8_000)
    }
}
