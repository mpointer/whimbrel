import CoreGraphics

/// What the snapshot service remembers about a sampled frame, after cropping to the shared content.
struct FrameFeatures: Equatable {
    let hash: UInt64          // 64-bit dHash of the cropped frame
    let text: String          // OCR text of the cropped frame
    let crop: CGRect          // content rect in source-image pixels
}

enum SnapshotDecision: Equatable { case skip, waitForStable, keepNew, replaceLast }

/// Pure per-frame decisions for meeting snapshots: drop junk frames, crop to shared content, and
/// decide whether a stable frame is a new slide, a build of the last one, or a repeat. No Vision here;
/// the caller runs face detection and OCR and passes the results in.
enum SnapshotFrameFilter {
    static let sameSlideDistance = 6       // dHash Hamming distance ≤ this ⇒ same slide
    static let stableDistance = 3          // consecutive samples ≤ this ⇒ stable
    static let minTextWords = 3            // both frames need this many OCR words to compare by text
    static let stableTextSimilarity = 0.9  // word-set Jaccard ≥ this between samples ⇒ stable
    static let sameSlideTextSimilarity = 0.8  // word-set Jaccard ≥ this to the last kept ⇒ same slide

    // Thresholds below are measured on the SnapshotFixtures frames at the 256 px thumbnail; see the
    // Task 6 report for the numbers. Luminance is 0...255.
    static let thumbnailSide = 256
    static let unchangedPixelStep = 6      // every thumbnail pixel within this ⇒ the frame hasn't changed
    static let uniformStdDev = 6.0          // whole frame flatter than this ⇒ blank
    static let edgeStep = 32                // |dx| + |dy| above this counts as an edge pixel
    static let minEdgeDensity = 0.005       // fewer edge pixels ⇒ spinner / loading screen
    static let strongEdgeStep = 128         // |dx| + |dy| of crisp ink on a contrasting background
    static let blendMidGrey = 0.5           // a cross-fade is mostly mid-grey (64...191) …
    static let blendMaxStrongEdges = 0.01   // … has little crisp contrast …
    static let blendInkStep = 48            // … and ink both this much brighter and darker than the median …
    static let blendMinInk = 0.002          // … each covering at least this share of the frame
    static let faceTileMargin = 1.0         // face rect grown by this many face sizes per side ≈ its tile
    static let minContentArea = 0.25        // content left beside faces must cover this much of the frame
    static let barStdDev = 6.0              // an edge row/column flatter than this is a uniform bar
    static let barStep = 40.0               // … and ends where the mean luminance steps by this much

    /// Difference hash: each bit says whether a cell of a 9x8 box-averaged grayscale grid is darker
    /// than its right neighbour.
    static func dHash(_ image: CGImage) -> UInt64 {
        guard let luma = Luma(thumbnailOf: image), luma.width >= 9, luma.height >= 8 else { return 0 }
        let cell = { (cx: Int, cy: Int) in
            luma.mean(in: (cx * luma.width / 9..<(cx + 1) * luma.width / 9, cy * luma.height / 8..<(cy + 1) * luma.height / 8))
        }
        var hash: UInt64 = 0
        for y in 0..<8 {
            for x in 0..<8 {
                hash = hash << 1 | (cell(x, y) < cell(x + 1, y) ? 1 : 0)
            }
        }
        return hash
    }

    static func hamming(_ a: UInt64, _ b: UInt64) -> Int { (a ^ b).nonzeroBitCount }

    /// Blank, loading/spinner, and mid-cross-fade frames.
    static func isJunk(_ image: CGImage) -> Bool {
        guard let luma = Luma(thumbnailOf: image) else { return true }
        let s = luma.stats(in: luma.bounds)
        if s.stdDev < uniformStdDev || s.edgeDensity < minEdgeDensity { return true }
        // Ink on both sides of a mid-grey median is what separates a light/dark cross-fade from a
        // dark-theme slide, whose soft-contrast text is often lower contrast than the fade itself.
        return s.midGreyFraction > blendMidGrey && s.strongEdgeDensity < blendMaxStrongEdges
            && s.brighterInk >= blendMinInk && s.darkerInk >= blendMinInk
    }

    /// The shared-content rect, or nil for a participant grid / speaker view.
    ///
    /// `faces` and the result are in source-image pixels with a top-left origin, the convention of
    /// `CGImage.cropping(to:)`. Callers convert Vision's normalized bottom-left face boxes first.
    static func contentCrop(_ image: CGImage, faces: [CGRect]) -> CGRect? {
        guard let luma = Luma(thumbnailOf: image) else { return nil }
        let full = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        var region = full
        if !faces.isEmpty {
            // ponytail: a face box grown by one face size per side stands in for its webcam tile; a
            // tile much wider than that leaks into the crop. Detect tile borders if that shows up.
            let tiles = faces.map { $0.insetBy(dx: -$0.width * faceTileMargin, dy: -$0.height * faceTileMargin) }
            let all = tiles.dropFirst().reduce(tiles[0]) { $0.union($1) }
            let sides = [
                CGRect(x: full.minX, y: full.minY, width: all.minX - full.minX, height: full.height),
                CGRect(x: all.maxX, y: full.minY, width: full.maxX - all.maxX, height: full.height),
                CGRect(x: full.minX, y: full.minY, width: full.width, height: all.minY - full.minY),
                CGRect(x: full.minX, y: all.maxY, width: full.width, height: full.maxY - all.maxY),
            ].map { $0.intersection(full) }.filter { !$0.isNull && $0.width > 0 && $0.height > 0 }
            guard let largest = sides.max(by: { area($0) < area($1) }),
                  area(largest) >= minContentArea * area(full) else { return nil }
            region = largest
        }

        let scale = Double(luma.width) / Double(image.width)
        var box = luma.inner(region, scale: scale)
        box = luma.trimmingBars(box)
        guard !box.x.isEmpty, !box.y.isEmpty else { return nil }

        // Beside any face, a region with little text is webcam backdrop (grid, speaker view, or one
        // small tile), not shared content.
        // ponytail: edge density stands in for "has text" since OCR runs after the crop; a textured
        // webcam backdrop beside the faces reads as text. Feed OCR coverage in if grids slip through.
        if !faces.isEmpty, luma.stats(in: box).edgeDensity < minEdgeDensity { return nil }

        // Back to source pixels; a cell index at the thumbnail's far edge maps to the exact image edge.
        let toSource = { (cell: Int, cells: Int, pixels: Int) in
            cell == cells ? Double(pixels) : (Double(cell) / scale).rounded()
        }
        let x0 = toSource(box.x.lowerBound, luma.width, image.width), x1 = toSource(box.x.upperBound, luma.width, image.width)
        let y0 = toSource(box.y.lowerBound, luma.height, image.height), y1 = toSource(box.y.upperBound, luma.height, image.height)
        return CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)
    }

    /// Text-first: frames compare by OCR word sets when both have `minTextWords`, by dHash otherwise.
    /// A 9x8 dHash barely separates same-template text slides (unrelated ones measured 3 apart), so
    /// text decides wherever there is enough of it.
    static func decide(current: FrameFeatures, previousSample: FrameFeatures?, lastKept: FrameFeatures?) -> SnapshotDecision {
        guard let previousSample else { return .waitForStable }
        let now = words(current.text)
        guard similar(now, current.hash, words(previousSample.text), previousSample.hash,
                      minJaccard: stableTextSimilarity, maxDistance: stableDistance) else { return .waitForStable }
        guard let lastKept else { return .keepNew }
        let kept = words(lastKept.text)
        // A build keeps the look of the slide it grows from; a later slide that merely repeats an
        // earlier title (title slide → content slide) is a new slide, not a build.
        if !kept.isEmpty, kept.isStrictSubset(of: now), hamming(current.hash, lastKept.hash) <= sameSlideDistance {
            return .replaceLast
        }
        if similar(now, current.hash, kept, lastKept.hash, minJaccard: sameSlideTextSimilarity, maxDistance: sameSlideDistance) {
            return .skip
        }
        return .keepNew
    }

    private static func similar(_ a: Set<String>, _ hashA: UInt64, _ b: Set<String>, _ hashB: UInt64,
                                minJaccard: Double, maxDistance: Int) -> Bool {
        guard a.count >= minTextWords, b.count >= minTextWords else { return hamming(hashA, hashB) <= maxDistance }
        return Double(a.intersection(b).count) / Double(a.union(b).count) >= minJaccard
    }

    private static func words(_ text: String) -> Set<String> {
        Set(text.lowercased().split { !$0.isLetter && !$0.isNumber }.map(String.init))
    }

    private static func area(_ r: CGRect) -> Double { r.width * r.height }
}

/// An 8-bit grayscale copy of an image; row 0 is the top row.
struct Luma {
    typealias Box = (x: Range<Int>, y: Range<Int>)

    struct Stats {
        let stdDev: Double
        let edgeDensity: Double
        let strongEdgeDensity: Double
        let midGreyFraction: Double
        let brighterInk: Double         // share more than `blendInkStep` above the median
        let darkerInk: Double           // share more than `blendInkStep` below the median
    }

    let width: Int
    let height: Int
    private var pixels: [UInt8]

    var bounds: Box { (0..<width, 0..<height) }

    init?(_ image: CGImage, width: Int, height: Int) {
        guard width > 0, height > 0 else { return nil }
        self.width = width
        self.height = height
        pixels = [UInt8](repeating: 0, count: width * height)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let ctx = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width, space: CGColorSpaceCreateDeviceGray(),
                                      bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return false }
            ctx.interpolationQuality = .medium
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drawn else { return nil }
    }

    /// Downsampled so the long side is at most `SnapshotFrameFilter.thumbnailSide`.
    init?(thumbnailOf image: CGImage) {
        let scale = min(1, Double(SnapshotFrameFilter.thumbnailSide) / Double(max(image.width, image.height, 1)))
        self.init(image, width: Int((Double(image.width) * scale).rounded()),
                  height: Int((Double(image.height) * scale).rounded()))
    }

    subscript(x: Int, y: Int) -> Int { Int(pixels[y * width + x]) }

    /// Same size and every pixel within `tolerance` grey levels: the same frame, give or take
    /// capture noise. Far stricter than a dHash match, which a one-bullet build can pass.
    func matches(_ other: Luma, tolerance: Int) -> Bool {
        width == other.width && height == other.height
            && zip(pixels, other.pixels).allSatisfy { abs(Int($0) - Int($1)) <= tolerance }
    }

    func stats(in box: Box) -> Stats {
        var sum = 0.0, sumSquares = 0.0, edges = 0, strongEdges = 0
        var histogram = [Int](repeating: 0, count: 256)
        for y in box.y {
            for x in box.x {
                let v = self[x, y]
                sum += Double(v)
                sumSquares += Double(v * v)
                histogram[v] += 1
                guard x + 1 < box.x.upperBound, y + 1 < box.y.upperBound else { continue }
                let gradient = abs(self[x + 1, y] - v) + abs(self[x, y + 1] - v)
                if gradient > SnapshotFrameFilter.edgeStep { edges += 1 }
                if gradient > SnapshotFrameFilter.strongEdgeStep { strongEdges += 1 }
            }
        }
        let count = box.x.count * box.y.count, n = Double(max(count, 1))
        let mean = sum / n
        var median = 0, seen = histogram[0]
        while seen * 2 < count, median < 255 { median += 1; seen += histogram[median] }
        let share = { (levels: ClosedRange<Int>) in
            levels.isEmpty ? 0 : Double(histogram[levels].reduce(0, +)) / n
        }
        let step = SnapshotFrameFilter.blendInkStep
        return Stats(stdDev: (max(sumSquares / n - mean * mean, 0)).squareRoot(),
                     edgeDensity: Double(edges) / n, strongEdgeDensity: Double(strongEdges) / n,
                     midGreyFraction: share(64...191),
                     brighterInk: median + step < 255 ? share((median + step + 1)...255) : 0,
                     darkerInk: median - step > 0 ? share(0...(median - step - 1)) : 0)
    }

    func mean(in box: Box) -> Double {
        var sum = 0
        for y in box.y { for x in box.x { sum += self[x, y] } }
        return Double(sum) / Double(max(box.x.count * box.y.count, 1))
    }

    /// Thumbnail cells wholly inside `rect` (source pixels), so the crop never reaches past it.
    func inner(_ rect: CGRect, scale: Double) -> Box {
        let x0 = min(width, Int((rect.minX * scale).rounded(.up))), x1 = max(x0, min(width, Int((rect.maxX * scale).rounded(.down))))
        let y0 = min(height, Int((rect.minY * scale).rounded(.up))), y1 = max(y0, min(height, Int((rect.maxY * scale).rounded(.down))))
        return (x0..<x1, y0..<y1)
    }

    /// Drops uniform bars at each edge of `box`. A bar is a run of flat rows/columns that ends in a
    /// step to a different luminance; a flat run that fades into same-coloured content is a margin
    /// of that content and stays.
    func trimmingBars(_ box: Box) -> Box {
        var b = box
        let column = { (x: Int) in self.stats(in: (x..<x + 1, b.y)) }
        let row = { (y: Int) in self.stats(in: (b.x, y..<y + 1)) }
        func barLength(_ indices: [Int], flat: (Int) -> Bool, level: (Int) -> Double) -> Int {
            guard let first = indices.first, flat(first) else { return 0 }
            let base = level(first)
            var k = 1
            while k < indices.count, flat(indices[k]), abs(level(indices[k]) - base) < SnapshotFrameFilter.barStep { k += 1 }
            return k < indices.count && abs(level(indices[k]) - base) >= SnapshotFrameFilter.barStep ? k : 0
        }
        let flatColumn = { (x: Int) in column(x).stdDev < SnapshotFrameFilter.barStdDev }
        let flatRow = { (y: Int) in row(y).stdDev < SnapshotFrameFilter.barStdDev }
        let columnLevel = { (x: Int) in self.mean(in: (x..<x + 1, b.y)) }
        let rowLevel = { (y: Int) in self.mean(in: (b.x, y..<y + 1)) }

        let left = barLength(Array(b.x), flat: flatColumn, level: columnLevel)
        let right = barLength(Array(b.x.reversed()), flat: flatColumn, level: columnLevel)
        b.x = (b.x.lowerBound + left)..<max(b.x.lowerBound + left, b.x.upperBound - right)
        let top = barLength(Array(b.y), flat: flatRow, level: rowLevel)
        let bottom = barLength(Array(b.y.reversed()), flat: flatRow, level: rowLevel)
        b.y = (b.y.lowerBound + top)..<max(b.y.lowerBound + top, b.y.upperBound - bottom)
        return b
    }
}
