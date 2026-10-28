import CoreGraphics
import CoreText
import Foundation

/// Meeting frames drawn in code for the snapshot filter and service tests. Every returned rect is in
/// source-image pixels with a top-left origin, the convention `CGImage.cropping(to:)` and
/// `SnapshotFrameFilter.contentCrop` use.
enum SnapshotFixtures {
    static let size = CGSize(width: 1280, height: 720)

    /// A flat frame, the shape of a blank or "waiting for host" screen.
    static func solid(gray: CGFloat = 0.15, size: CGSize = size) -> CGImage {
        draw(size) { ctx in fill(ctx, CGRect(origin: .zero, size: size), gray: gray) }
    }

    /// A small loading arc on a flat background.
    static func spinner(size: CGSize = size) -> CGImage {
        draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 0.12)
            ctx.setStrokeColor(gray: 0.85, alpha: 1)
            ctx.setLineWidth(6)
            ctx.addArc(center: CGPoint(x: size.width / 2, y: size.height / 2), radius: 24,
                       startAngle: 0, endAngle: .pi * 1.4, clockwise: false)
            ctx.strokePath()
        }
    }

    /// A slide: a title line then bullet lines, dark text on `background` (white by default).
    static func slide(_ lines: [String], background: CGFloat = 1, ink: CGFloat = 0,
                      size: CGSize = size) -> CGImage {
        draw(size) { ctx in drawSlide(ctx, lines, in: CGRect(origin: .zero, size: size), background: background, ink: ink) }
    }

    /// Step `shown` of a slide that reveals `lines` one at a time.
    static func slideBuild(_ lines: [String], shown: Int, size: CGSize = size) -> CGImage {
        slide(Array(lines.prefix(shown)), size: size)
    }

    /// A text-free flowchart: three boxes left to right joined by arrows, on white.
    static func flowchart(size: CGSize = size) -> CGImage {
        draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 1)
            ctx.setStrokeColor(gray: 0.1, alpha: 1)
            ctx.setLineWidth(6)
            let box = CGSize(width: size.width * 0.2, height: size.height * 0.25)
            for i in 0..<3 {
                let x = size.width * (0.08 + 0.32 * CGFloat(i))
                ctx.stroke(CGRect(x: x, y: size.height / 2 - box.height / 2, width: box.width, height: box.height))
                guard i < 2 else { continue }
                ctx.move(to: CGPoint(x: x + box.width, y: size.height / 2))
                ctx.addLine(to: CGPoint(x: x + size.width * 0.32, y: size.height / 2))
                ctx.strokePath()
            }
        }
    }

    /// A text-free bar chart: five filled bars of varying height rising from a baseline, on white.
    static func barChart(size: CGSize = size) -> CGImage {
        draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 1)
            for (i, height) in [0.3, 0.55, 0.4, 0.75, 0.6].enumerated() {
                fill(ctx, CGRect(x: size.width * (0.12 + 0.16 * CGFloat(i)), y: size.height * 0.12,
                                 width: size.width * 0.1, height: size.height * height), gray: 0.25)
            }
        }
    }

    /// A participant grid of webcam stand-ins: one tile per participant, a face oval in each.
    static func faceGrid(columns: Int = 2, rows: Int = 2, size: CGSize = size) -> (image: CGImage, faces: [CGRect]) {
        var faces: [CGRect] = []
        let tile = CGSize(width: size.width / CGFloat(columns), height: size.height / CGFloat(rows))
        let image = draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 0.1)
            for c in 0..<columns {
                for r in 0..<rows {
                    let t = CGRect(x: CGFloat(c) * tile.width, y: CGFloat(r) * tile.height,
                                   width: tile.width, height: tile.height).insetBy(dx: 4, dy: 4)
                    faces.append(drawTile(ctx, t, canvas: size))
                }
            }
        }
        return (image, faces)
    }

    /// A shared slide on the left 75% and a filmstrip of three webcam tiles on the right.
    static func slideWithFilmstrip(size: CGSize = size) -> (image: CGImage, faces: [CGRect], slide: CGRect) {
        var faces: [CGRect] = []
        let slideRect = CGRect(x: 0, y: 0, width: size.width * 0.75, height: size.height)
        let image = draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 0.1)
            drawSlide(ctx, ["Quarterly roadmap", "Ship meeting snapshots", "Calendar sync",
                            "Faster search", "Windows parity"], in: slideRect, background: 1, ink: 0)
            let strip = CGRect(x: slideRect.maxX, y: 0, width: size.width - slideRect.maxX, height: size.height)
            let tileHeight = strip.height / 3
            for i in 0..<3 {
                let t = CGRect(x: strip.minX, y: CGFloat(i) * tileHeight, width: strip.width, height: tileHeight)
                    .insetBy(dx: 8, dy: 8)
                faces.append(drawTile(ctx, t, canvas: size))
            }
        }
        return (image, faces, slideRect)  // slideRect starts at y 0 and spans the height, so no flip needed
    }

    /// Zoom's default layout: a row of four webcam tiles across the top, the shared slide below.
    static func slideWithTopFilmstrip(size: CGSize = size) -> (image: CGImage, faces: [CGRect], slide: CGRect) {
        var faces: [CGRect] = []
        let stripHeight = size.height * 0.2
        // Context coordinates: the strip is the top band, so the slide is everything beneath it.
        let slideRect = CGRect(x: 0, y: 0, width: size.width, height: size.height - stripHeight)
        let image = draw(size) { ctx in
            fill(ctx, CGRect(origin: .zero, size: size), gray: 0.1)
            drawSlide(ctx, ["Launch checklist", "Docs updated", "Pricing page live", "Support briefed"],
                      in: slideRect, background: 1, ink: 0)
            let tileWidth = size.width / 4
            for i in 0..<4 {
                let t = CGRect(x: CGFloat(i) * tileWidth, y: slideRect.maxY, width: tileWidth, height: stripHeight)
                    .insetBy(dx: 8, dy: 8)
                faces.append(drawTile(ctx, t, canvas: size))
            }
        }
        return (image, faces, CGRect(x: 0, y: stripHeight, width: size.width, height: size.height - stripHeight))
    }

    /// One webcam filling the frame, with a face oval `faceSide` pixels across at its centre.
    static func webcam(faceSide: CGFloat, size: CGSize = size) -> (image: CGImage, faces: [CGRect]) {
        var faces: [CGRect] = []
        let image = draw(size) { ctx in
            faces.append(drawTile(ctx, CGRect(origin: .zero, size: size), canvas: size, faceSide: faceSide))
        }
        return (image, faces)
    }

    /// `a` and `b` composited at 50%, the frame caught mid cross-fade.
    static func blend(_ a: CGImage, _ b: CGImage) -> CGImage {
        let size = CGSize(width: a.width, height: a.height)
        return draw(size) { ctx in
            let rect = CGRect(origin: .zero, size: size)
            ctx.draw(a, in: rect)
            ctx.setAlpha(0.5)
            ctx.draw(b, in: rect)
        }
    }

    // MARK: - Drawing

    private static func draw(_ size: CGSize, _ body: (CGContext) -> Void) -> CGImage {
        let ctx = CGContext(data: nil, width: Int(size.width), height: Int(size.height), bitsPerComponent: 8,
                            bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        body(ctx)
        return ctx.makeImage()!
    }

    private static func fill(_ ctx: CGContext, _ rect: CGRect, gray: CGFloat) {
        ctx.setFillColor(gray: gray, alpha: 1)
        ctx.fill(rect)
    }

    /// Draws a slide into `rect` (context coordinates, bottom-left origin).
    private static func drawSlide(_ ctx: CGContext, _ lines: [String], in rect: CGRect, background: CGFloat, ink: CGFloat) {
        fill(ctx, rect, gray: background)
        let color = CGColor(gray: ink, alpha: 1)
        var baseline = rect.maxY - rect.height * 0.18
        for (i, text) in lines.enumerated() {
            let font = CTFontCreateWithName("Helvetica" as CFString, i == 0 ? 56 : 36, nil)
            let attributed = NSAttributedString(string: i == 0 ? text : "• " + text, attributes: [
                NSAttributedString.Key(kCTFontAttributeName as String): font,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): color,
            ])
            ctx.textPosition = CGPoint(x: rect.minX + rect.width * 0.08, y: baseline)
            CTLineDraw(CTLineCreateWithAttributedString(attributed), ctx)
            baseline -= i == 0 ? 96 : 64
        }
    }

    /// Draws a webcam tile stand-in (backdrop, shoulders, face oval) and returns the face rect in
    /// top-left-origin pixels, as the caller's Vision pass would report it.
    private static func drawTile(_ ctx: CGContext, _ tile: CGRect, canvas: CGSize, faceSide: CGFloat? = nil) -> CGRect {
        fill(ctx, tile, gray: 0.3)
        let faceSide = faceSide ?? min(tile.width, tile.height) * 0.45
        let face = CGRect(x: tile.midX - faceSide / 2, y: tile.midY - faceSide * 0.3, width: faceSide, height: faceSide)
        ctx.setFillColor(gray: 0.2, alpha: 1)
        ctx.fillEllipse(in: CGRect(x: tile.midX - faceSide, y: tile.minY, width: faceSide * 2, height: face.minY - tile.minY))
        ctx.setFillColor(gray: 0.7, alpha: 1)
        ctx.fillEllipse(in: face)
        return CGRect(x: face.minX, y: canvas.height - face.maxY, width: face.width, height: face.height)
    }
}
