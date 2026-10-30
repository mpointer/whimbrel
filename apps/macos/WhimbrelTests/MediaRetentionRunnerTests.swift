import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import Whimbrel

/// The media retention run (Task 9). Most tests drive the runner's logic through fake ops
/// (store, mover, compressor, thumbnailer) and a fake clock; the last three run the real
/// planner, store and mover in a temp folder with only the encoder faked.
@MainActor
final class MediaRetentionRunnerTests: XCTestCase {
    private var suite: String!
    private var defaults: UserDefaults!
    private var settings: MediaSettings!
    private var tempDir: URL!
    private var media: URL!
    private let fm = FileManager.default
    private var clock = Clock()

    /// Advanced by tests from any queue.
    final class Clock: @unchecked Sendable {
        private let lock = NSLock()
        private var t = Date(timeIntervalSince1970: 1_800_000_000)
        func now() -> Date { lock.withLock { t } }
        func advance(_ s: TimeInterval) { lock.withLock { t += s } }
    }

    /// An in-memory store and file system: records every call and the queue it ran on.
    final class Fake: @unchecked Sendable {
        var work: [CoreMeetingWork] = []
        var paths: [Int64: [String]] = [:]
        var snaps: [Int64: [CoreMeetingSnapshot]] = [:]
        var calls: [String] = []
        var planBusy: [Set<Int64>] = []
        var failCompress: Set<Int64> = []
        var failMove: Set<Int64> = []
        var keep: Set<Int64> = []
        var untranscribed: Set<Int64> = []
        var disconnected = 0
        var planPolicies: [CoreMediaPolicy] = []
        var moveDirs: [URL] = []
        var offQueueCalls = 0
        var onMove: (Int64) -> Void = { _ in }
        var onCompress: () -> Void = {}

        private func note(_ s: String) {
            if DispatchQueue.getSpecific(key: Self.queueKey) == nil { offQueueCalls += 1 }
            calls.append(s)
        }
        static let queueKey: DispatchSpecificKey<Bool> = {
            let k = DispatchSpecificKey<Bool>()
            MediaMover.queue.setSpecific(key: k, value: true)
            return k
        }()

        func ops() -> MediaRetentionRunner.Ops {
            MediaRetentionRunner.Ops(
                plan: { policy, _, _, busy in self.note("plan"); self.planBusy.append(busy); self.planPolicies.append(policy)
                    return CoreMediaPlan(work: self.work, disconnectedMeetings: self.disconnected,
                                         usage: CoreMediaUsage(audioFullBytes: 0, audioCompressedBytes: 0, imageBytes: 0)) },
                audioPaths: { id in self.paths[id] ?? [] },
                keepMedia: { id in self.keep.contains(id) },
                allTranscribed: { id in !self.untranscribed.contains(id) },
                snapshots: { id in self.snaps[id] ?? [] },
                moveAudio: { id, paths, dir in
                    self.note("move \(id) \(paths)")
                    self.moveDirs.append(dir)
                    self.onMove(id)
                    if self.failMove.contains(id) { return false }
                    self.paths[id] = paths.map { dir.appendingPathComponent(URL(fileURLWithPath: $0).lastPathComponent).path }
                    return true
                },
                moveSnapshots: { id, root in
                    self.note("moveSnapshots \(id)")
                    self.snaps[id] = self.snaps[id]?.map {
                        CoreMeetingSnapshot(id: $0.id, meetingId: id, capturedUnixMs: $0.capturedUnixMs,
                                            filePath: root.appendingPathComponent(URL(fileURLWithPath: $0.filePath).lastPathComponent).path,
                                            width: $0.width, height: $0.height, ocrText: $0.ocrText)
                    }
                    return true
                },
                compress: { id, paths in self.note("compress \(id) \(paths)"); self.onCompress(); return !self.failCompress.contains(id) },
                deleteAudio: { path in self.note("delete \(path)") },
                shrink: { snap in self.note("shrink \(snap.id) \(snap.filePath)"); return true }
            )
        }
    }

    override func setUp() {
        super.setUp()
        suite = "MediaRetentionRunnerTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
        tempDir = fm.temporaryDirectory.appendingPathComponent("MediaRetentionRunnerTests-\(UUID().uuidString)", isDirectory: true)
        media = tempDir.appendingPathComponent("media", isDirectory: true)
        try! fm.createDirectory(at: media, withIntermediateDirectories: true)
        settings = MediaSettings(defaults: defaults)
        settings.folderPath = media.path
        settings.neverDeleteAudio = false  // off by default; these tests exercise it on
        clock = Clock()
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        try? fm.removeItem(at: tempDir)
        super.tearDown()
    }

    private func runner(_ fake: Fake, busy: Set<Int64> = [], isBusy: ((Int64) -> Bool)? = nil) -> MediaRetentionRunner {
        let clock = clock
        return MediaRetentionRunner(
            store: nil, settings: settings, deviceId: "dev",
            isBusy: isBusy ?? { busy.contains($0) }, busyIds: { busy },
            now: { clock.now() }, defaults: defaults, ops: fake.ops()
        )
    }

    private func runToEnd(_ r: MediaRetentionRunner, _ mode: MediaRetentionRunner.Mode = .daily) async {
        r.run(mode: mode)
        await idle(r)
    }

    /// Waits for the run and any re-run it requested.
    private func idle(_ r: MediaRetentionRunner) async {
        while let t = r.current { await t.value }
    }

    private func work(_ id: Int64, _ steps: [CoreMediaStep], snaps: [Int64] = []) -> CoreMeetingWork {
        CoreMeetingWork(meetingId: id, steps: steps, audioPaths: ["/rec/system-audio-\(id).caf"], shrinkSnapshotIds: snaps)
    }

    private func fake(_ items: [CoreMeetingWork]) -> Fake {
        let f = Fake()
        f.work = items
        for w in items { f.paths[w.meetingId] = w.audioPaths }
        return f
    }

    private func touched(_ f: Fake) -> [Int64] {
        var ids: [Int64] = []
        for c in f.calls where c != "plan" {
            let id = Int64(c.split(separator: " ")[1].filter(\.isNumber))!
            if !ids.contains(id) { ids.append(id) }
        }
        return ids
    }

    // MARK: Skips, caps, backoff, order, re-entrancy

    func testBusyMeetingIsUntouched() async {
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio])])
        await runToEnd(runner(f, busy: [7]))
        XCTAssertEqual(f.planBusy, [[7]], "the planner gets the busy ids")

        // Became busy after the plan was made: the re-check right before the meeting skips it.
        let g = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio])])
        await runToEnd(runner(g, isBusy: { $0 == 1 }))
        XCTAssertEqual(touched(g), [2])
    }

    func testBusyIsReCheckedBeforeEachMeeting() async {
        var busyNow: Set<Int64> = []
        let f = fake([work(1, [.moveAudio]), work(2, [.moveAudio])])
        // Meeting 2 starts transcribing while meeting 1 is being moved.
        f.onMove = { id in if id == 1 { DispatchQueue.main.sync { _ = busyNow.insert(2) } } }
        await runToEnd(runner(f, isBusy: { busyNow.contains($0) }))
        XCTAssertEqual(touched(f), [1])
    }

    func testDailyRunStopsAfter10Meetings() async {
        let f = fake((1...12).map { work(Int64($0), [.moveAudio]) })
        let r = runner(f)
        await runToEnd(r)
        XCTAssertEqual(touched(f), Array(1...10))
        XCTAssertNil(r.progress, "idle again")
    }

    func testBusyMeetingsDoNotUseUpTheBatch() async {
        let f = fake((1...12).map { work(Int64($0), [.moveAudio]) })
        await runToEnd(runner(f, busy: [1, 2]))
        XCTAssertEqual(touched(f), Array(3...12))
    }

    func testAMoveThatFailsDoesNotUseUpTheBatch() async {
        // Meetings 1 and 2 can never move (say, a source the mover refuses every time).
        let f = fake((1...12).map { work(Int64($0), [.moveAudio]) })
        f.failMove = [1, 2]
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), Array(1...12), "the two failures leave all 10 slots to the meetings after them")
    }

    func testDailyRunStopsAfter10Minutes() async {
        let f = fake((1...6).map { work(Int64($0), [.moveAudio]) })
        f.onMove = { [clock] _ in clock.advance(4 * 60) }
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1, 2, 3], "checked between meetings: 0, 4 and 8 minutes in, then stop at 12")
    }

    func testMoveOnlyIgnoresAgeStepsAndHasNoCap() async {
        let f = fake((1...12).map { work(Int64($0), [.moveAudio, .moveSnapshots, .compress, .shrinkImages], snaps: [5]) })
        f.onMove = { [clock] _ in clock.advance(4 * 60) }
        await runToEnd(runner(f), .moveOnly)
        XCTAssertEqual(touched(f), Array(1...12))
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("compress") || $0.hasPrefix("shrink") || $0.hasPrefix("delete") })
    }

    func testMoveThenCompressCarriesThePathsForward() async {
        let f = fake([work(1, [.moveAudio, .compress])])
        f.paths[1] = ["/rec/system-audio-A.caf", "/rec/system-audio-B.caf", "/rec/system-audio-B.caf"]
        await runToEnd(runner(f))
        let audio = settings.audioURL(deviceId: "dev").path
        XCTAssertEqual(f.calls.dropFirst(), [
            "move 1 [\"/rec/system-audio-A.caf\", \"/rec/system-audio-B.caf\"]",
            "compress 1 [\"\(audio)/system-audio-A.caf\", \"\(audio)/system-audio-B.caf\"]",
        ], "compress gets the paths re-read after the move, deduped")
        XCTAssertEqual(f.offQueueCalls, 0, "every step runs on MediaMover.queue")
    }

    func testFailedMoveSkipsTheRestOfThatMeetingOnlyAndIsNotBackedOff() async {
        // A missing media folder: the mover says false. Skipped this time, not retried in the run.
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio, .compress])])
        f.failMove = [1]
        let r = runner(f)
        await runToEnd(r)
        XCTAssertEqual(f.calls.filter { $0.hasPrefix("move 1") }.count, 1)
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("compress 1") })
        XCTAssertTrue(f.calls.contains { $0.hasPrefix("compress 2") })
        f.calls = []
        await runToEnd(r)
        XCTAssertTrue(f.calls.contains { $0.hasPrefix("move 1") }, "the next trigger (a mount) tries again")
    }

    func testFailedCompressBacksOff() async {
        let f = fake([work(1, [.compress]), work(2, [.compress])])
        f.failCompress = [1]
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1, 2])

        f.calls = []
        await runToEnd(runner(f))  // a new runner: the backoff is persisted
        XCTAssertEqual(touched(f), [2], "not retried on the next trigger")

        clock.advance(25 * 3600)
        f.calls = []
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1, 2], "retried after a day")

        clock.advance(25 * 3600)
        f.calls = []
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [2], "the second failure waits two days")

        f.failCompress = []
        clock.advance(25 * 3600)
        f.calls = []
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1, 2])
        f.calls = []
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1, 2], "a success clears the backoff")
    }

    func testTriggersDuringARunRunOnceMoreAfterItInTheStrongerMode() async {
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio])])
        let r = runner(f)
        f.onMove = { id in
            if id == 1, f.calls.filter({ $0 == "plan" }).count == 1 {
                DispatchQueue.main.sync { MainActor.assumeIsolated { r.run(mode: .moveOnly); r.run(mode: .daily); r.refreshPlan() } }
            }
        }
        r.run(mode: .moveOnly)
        r.run(mode: .moveOnly)
        await idle(r)
        XCTAssertEqual(f.calls.map { String($0.prefix(6)) }, ["plan", "move 1", "move 2", "plan", "move 1", "compre", "move 2"],
                       "never in parallel; one re-run after, as .daily since one request was")
    }

    func testTriggerDuringAPlanRefreshRunsAfterIt() async {
        let f = fake([work(1, [.moveAudio])])
        let r = runner(f)
        r.refreshPlan()
        r.run(mode: .daily)
        await idle(r)
        XCTAssertEqual(f.calls.filter { $0 == "plan" }.count, 2)
        XCTAssertEqual(touched(f), [1])
    }

    func testPolicyChangeMidRunEndsTheRunAndRunsAgain() async {
        let f = fake([work(1, [.moveAudio]), work(2, [.moveAudio])])
        let settings = settings!
        f.onMove = { id in
            if id == 1, f.planPolicies.count == 1 { DispatchQueue.main.sync { settings.compressAfterDays = 45 } }
        }
        await runToEnd(runner(f))
        XCTAssertEqual(f.planPolicies.map(\.compressAfterDays), [30, 45])
        XCTAssertEqual(f.calls.filter { $0 != "plan" }.map { String($0.prefix(6)) }, ["move 1", "move 1", "move 2"],
                       "meeting 2 is not worked on under the old ages")
    }

    func testMediaFolderChangeMidRunEndsTheRunAndMovesToTheNewFolder() async {
        let f = fake([work(1, [.moveAudio]), work(2, [.moveAudio])])
        let settings = settings!, other = tempDir.appendingPathComponent("other", isDirectory: true)
        f.onMove = { id in
            if id == 1, f.moveDirs.count == 1 { DispatchQueue.main.sync { settings.folderPath = other.path } }
        }
        await runToEnd(runner(f))
        XCTAssertEqual(f.moveDirs.map(\.path), [media, other, other].map { $0.appendingPathComponent("Audio/dev").path })
    }

    func testKeepMediaTurnedOnMidRunLeavesOnlyMoves() async {
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio, .moveSnapshots, .deleteAudio, .shrinkImages], snaps: [9])])
        f.snaps[2] = [CoreMeetingSnapshot(id: 9, meetingId: 2, capturedUnixMs: 0, filePath: "/old/9.heic", width: 1, height: 1, ocrText: "")]
        f.onMove = { id in if id == 1 { f.keep = [1, 2] } }
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.filter { $0 != "plan" }.map { String($0.prefix(13)) },
                       ["move 1 [\"/rec", "move 2 [\"/rec", "moveSnapshots"])
    }

    func testBackoffIsPerStep() async {
        let f = fake([work(1, [.moveAudio, .compress, .shrinkImages], snaps: [9])])
        f.snaps[1] = [CoreMeetingSnapshot(id: 9, meetingId: 1, capturedUnixMs: 0, filePath: "/old/9.heic", width: 1, height: 1, ocrText: "")]
        f.failCompress = [1]
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.filter { $0 != "plan" }.map { String($0.prefix(5)) }, ["move ", "compr", "shrin"],
                       "a failed compress skips only itself")
        f.calls = []
        f.work = [work(1, [.moveAudio, .compress, .shrinkImages], snaps: [9])]
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.filter { $0 != "plan" }.map { String($0.prefix(5)) }, ["move ", "shrin"])
        // Past its delete age: the compress backoff doesn't hold back the delete.
        f.calls = []
        f.work = [work(1, [.deleteAudio])]
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.filter { $0 != "plan" }.map { String($0.prefix(6)) }, ["delete"])
    }

    func testBackoffForAMeetingNoLongerPlannedIsPruned() async {
        let f = fake([work(1, [.compress])])
        f.failCompress = [1]
        await runToEnd(runner(f))
        f.work = []
        await runToEnd(runner(f))
        f.work = [work(1, [.compress])]
        f.calls = []
        await runToEnd(runner(f))
        XCTAssertEqual(touched(f), [1], "pruned while not planned, so tried again at once")
    }

    func testNeverDeleteChosenDuringACompressStopsThatMeetingsDelete() async {
        let f = fake([work(1, [.compress, .deleteAudio, .shrinkImages], snaps: [9])])
        f.snaps[1] = [CoreMeetingSnapshot(id: 9, meetingId: 1, capturedUnixMs: 0, filePath: "/old/9.heic", width: 1, height: 1, ocrText: "")]
        let settings = settings!
        f.onCompress = { DispatchQueue.main.sync { settings.neverDeleteAudio = true; settings.shrinkImagesAfterDays = 60 } }
        await runToEnd(runner(f))
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("delete") }, "the delete age is off now")
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("shrink") }, "a longer shrink age may not be due")
    }

    func testCancelDropsAPendingReRun() async {
        let f = fake([work(1, [.moveAudio]), work(2, [.moveAudio])])
        let r = runner(f)
        f.onMove = { id in
            if id == 1 { DispatchQueue.main.sync { MainActor.assumeIsolated { r.run(mode: .daily); r.cancel() } } }
        }
        await runToEnd(r)
        XCTAssertEqual(f.calls.filter { $0 == "plan" }.count, 1)
        XCTAssertEqual(touched(f), [1])
    }

    func testPolicyChangeDuringAMoveOnlyRunReRunsMoveOnly() async {
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio, .compress])])
        let settings = settings!
        f.onMove = { id in
            if id == 1, f.planPolicies.count == 1 { DispatchQueue.main.sync { settings.compressAfterDays = 45 } }
        }
        await runToEnd(runner(f), .moveOnly)
        XCTAssertEqual(f.planPolicies.count, 2)
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("compress") }, "the next daily run does the age steps")
    }

    // MARK: Mount trigger

    func testMountOfAnUnrelatedVolumeDoesNothing() async {
        let f = fake([work(1, [.moveAudio, .compress])])
        let r = runner(f)
        r.volumeMounted(URL(fileURLWithPath: "/Volumes/USBSTICK"))
        await idle(r)
        XCTAssertEqual(f.calls, [])
    }

    func testMountOfTheMediaFolderVolumeRunsMovesOnly() async {
        let f = fake([work(1, [.moveAudio, .compress])])
        let r = runner(f)
        r.volumeMounted(tempDir)
        await idle(r)
        XCTAssertEqual(touched(f), [1])
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("compress") })
    }

    func testMountThatReconnectsAStoredPathRunsMovesOnly() async {
        let f = fake([work(1, [.moveAudio, .compress])])
        let r = runner(f)
        f.disconnected = 1
        r.refreshPlan()
        await idle(r)
        // Mounted, but nothing came back: no file work.
        r.volumeMounted(URL(fileURLWithPath: "/Volumes/Other"))
        await idle(r)
        XCTAssertEqual(f.calls, ["plan", "plan"])
        // This one brings a meeting's files back.
        f.disconnected = 0
        r.volumeMounted(URL(fileURLWithPath: "/Volumes/Archive"))
        await idle(r)
        XCTAssertEqual(touched(f), [1])
        XCTAssertFalse(f.calls.contains { $0.hasPrefix("compress") })
    }

    func testCancelStopsBeforeTheNextMeeting() async {
        let f = fake([work(1, [.moveAudio, .compress]), work(2, [.moveAudio])])
        let r = runner(f)
        f.onMove = { id in if id == 1 { DispatchQueue.main.sync { MainActor.assumeIsolated { r.cancel() } } } }
        await runToEnd(r)
        XCTAssertEqual(touched(f), [1])
        XCTAssertTrue(f.calls.contains { $0.hasPrefix("compress 1") }, "the current meeting finishes")
    }

    func testDeleteAudioDeletesEachStoredPathAndRewritesNothing() async {
        let f = fake([work(1, [.deleteAudio])])
        f.paths[1] = ["/rec/system-audio-A.caf", "/rec/system-audio-B.caf", "/rec/system-audio-B.caf"]
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.dropFirst(), ["delete /rec/system-audio-A.caf", "delete /rec/system-audio-B.caf"])
        XCTAssertEqual(f.paths[1]?.count, 3, "the stored paths are left as they were")
    }

    func testDeleteAudioReChecksTranscriptionRightBeforeDeleting() async {
        // Planned while transcribed; then continued, and the new part's transcription failed.
        let f = fake([work(1, [.deleteAudio]), work(2, [.deleteAudio])])
        f.untranscribed = [1]
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.dropFirst(), ["delete /rec/system-audio-2.caf"])
    }

    func testShrinkUsesTheSnapshotRowsReadAfterTheMove() async {
        let f = fake([work(1, [.moveSnapshots, .shrinkImages], snaps: [10])])
        f.snaps[1] = [10, 11].map {
            CoreMeetingSnapshot(id: $0, meetingId: 1, capturedUnixMs: 0, filePath: "/old/\($0).heic", width: 1, height: 1, ocrText: "t")
        }
        await runToEnd(runner(f))
        XCTAssertEqual(f.calls.dropFirst(), ["moveSnapshots 1", "shrink 10 \(settings.snapshotsURL.path)/10.heic"])
    }

    func testRefreshPlanDoesNoFileWork() async {
        let f = fake([work(1, [.moveAudio])])
        let r = runner(f)
        r.refreshPlan()
        await idle(r)
        XCTAssertEqual(r.lastPlan?.work.count, 1)
        XCTAssertEqual(f.calls, ["plan"])
    }

    // MARK: Real planner, store and mover (the encoder is faked)

    private var store: CoreMemoryStore!
    private var recordings: URL!

    private func realSetUp() {
        recordings = tempDir.appendingPathComponent("recordings", isDirectory: true)
        try! fm.createDirectory(at: recordings, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    private let day: Int64 = 86_400_000

    /// A completed, transcribed one-part meeting `daysOld` days before the clock, with a
    /// `.caf` pair in `recordings`.
    private func makeMeeting(daysOld: Int64, stamp: String) -> Int64 {
        let start = Int64(clock.now().timeIntervalSince1970 * 1000) - daysOld * day - 1
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: start, endUnixMs: nil, title: "M", attendees: [],
            trigger: .manual, audioPath: nil, status: .recording
        ))!
        let sys = recordings.appendingPathComponent("system-audio-\(stamp).caf")
        try! Data("sys \(stamp)".utf8).write(to: sys)
        try! Data("mic \(stamp)".utf8).write(to: recordings.appendingPathComponent("mic-audio-\(stamp).caf"))
        let part = store.beginMeetingPart(meetingId: id, startUnixMs: start, continueKey: nil)!
        XCTAssertTrue(store.completeMeetingPart(meetingId: id, partIndex: part, endUnixMs: start + 60_000, audioPath: sys.path))
        XCTAssertTrue(store.completeMeeting(id: id, endUnixMs: start + 60_000, audioPath: sys.path))
        XCTAssertTrue(store.setMeetingPartTranscript(meetingId: id, partIndex: part, transcriptPath: tempDir.appendingPathComponent("t\(stamp).txt").path))
        return id
    }

    /// Real ops, with an encoder stand-in that renames each `.caf` to `.m4a` and rewrites the paths.
    private func realRunner() -> MediaRetentionRunner {
        var ops = MediaRetentionRunner.Ops.live(store)
        let store = store!
        ops.compress = { id, paths in
            var moves: [(old: String, new: String)] = []
            for p in paths {
                let sys = URL(fileURLWithPath: p)
                for t in [sys, sys.deletingLastPathComponent().appendingPathComponent(sys.lastPathComponent.replacingOccurrences(of: "system-", with: "mic-"))] {
                    try? FileManager.default.moveItem(at: t, to: t.deletingPathExtension().appendingPathExtension("m4a"))
                }
                moves.append((p, sys.deletingPathExtension().appendingPathExtension("m4a").path))
            }
            return CoreMemoryStore.accessQueue.sync { store.setMeetingAudioPaths(meetingId: id, moves: moves) }
        }
        let clock = clock
        return MediaRetentionRunner(store: store, settings: settings, deviceId: "dev", isBusy: { _ in false }, busyIds: { [] },
                                    now: { clock.now() }, defaults: defaults, ops: ops)
    }

    private func storedPaths(_ id: Int64) -> [String?] {
        [store.getMeeting(id: id)?.audioPath] + (store.meetingParts(meetingId: id) ?? []).map(\.audioPath)
    }

    func testDailyRunMovesThenCompressesA40DayOldMeeting() async {
        realSetUp()
        let id = makeMeeting(daysOld: 40, stamp: "A")
        await runToEnd(realRunner())
        let audio = settings.audioURL(deviceId: "dev")
        XCTAssertEqual(((try? fm.contentsOfDirectory(atPath: audio.path)) ?? []).sorted(), ["mic-audio-A.m4a", "system-audio-A.m4a"])
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: recordings.path)) ?? [], [])
        let want = audio.appendingPathComponent("system-audio-A.m4a").path
        XCTAssertEqual(storedPaths(id).map { $0.map { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path } },
                       [want, want].map { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path })
    }

    func testDailyRunDeletesAudioPast90DaysAndKeepsAudioPath() async {
        realSetUp()
        let id = makeMeeting(daysOld: 91, stamp: "A")
        let before = storedPaths(id)
        await runToEnd(realRunner())
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: recordings.path)) ?? [], [], "both tracks deleted, not moved")
        XCTAssertFalse(fm.fileExists(atPath: settings.audioURL(deviceId: "dev").path))
        XCTAssertEqual(storedPaths(id), before, "audio_path stays set")
        XCTAssertNotNil(before.first ?? nil)
    }

    func testKeepMediaMeetingIsOnlyMoved() async {
        realSetUp()
        let id = makeMeeting(daysOld: 91, stamp: "A")
        XCTAssertTrue(store.setKeepMedia(meetingId: id, keep: true))
        await runToEnd(realRunner())
        XCTAssertEqual(((try? fm.contentsOfDirectory(atPath: settings.audioURL(deviceId: "dev").path)) ?? []).sorted(),
                       ["mic-audio-A.caf", "system-audio-A.caf"], "moved, never compressed or deleted")
    }

    func testShrinkReplacesImageWithThumbnailAndKeepsOCR() async throws {
        realSetUp()
        let start = Int64(clock.now().timeIntervalSince1970 * 1000) - 40 * day
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: start, endUnixMs: start + 60_000, title: "Standup",
            attendees: [], trigger: .calendar, audioPath: nil, status: .completed
        ))!
        let folder = try SnapshotStorage.create("2026-09-20 10.14 Standup", under: tempDir.appendingPathComponent("OldSnapshots"))
        let full = folder.appendingPathComponent("10.14.03.png")
        let ctx = CGContext(data: nil, width: 1920, height: 1080, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.setFillColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: 960, height: 540))
        let dest = CGImageDestinationCreateWithURL(full as CFURL, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
        XCTAssertTrue(CGImageDestinationFinalize(dest))
        let snapId = store.insertMeetingSnapshot(meetingId: id, capturedUnixMs: start + 1_000, filePath: full.path,
                                                 width: 1920, height: 1080, ocrText: "Roadmap calendar sync")!

        await runToEnd(realRunner())

        let row = try XCTUnwrap(store.listMeetingSnapshots(meetingId: id).first { $0.id == snapId } ?? store.listMeetingSnapshots(meetingId: id).first)
        XCTAssertTrue(row.filePath.hasSuffix("/10.14.03.thumb.jpg"), row.filePath)
        XCTAssertTrue(URL(fileURLWithPath: row.filePath).resolvingSymlinksInPath().path
            .hasPrefix(settings.snapshotsURL.resolvingSymlinksInPath().path), "moved, then shrunk")
        XCTAssertEqual(row.width, 320)
        XCTAssertEqual(row.height, 180)
        XCTAssertEqual(row.ocrText, "Roadmap calendar sync")
        let left = (try? fm.contentsOfDirectory(atPath: URL(fileURLWithPath: row.filePath).deletingLastPathComponent().path)) ?? []
        XCTAssertEqual(left.filter { !$0.hasPrefix(".") }, ["10.14.03.thumb.jpg"], "the full image is gone")
        XCTAssertFalse(fm.fileExists(atPath: full.path))
    }
}
