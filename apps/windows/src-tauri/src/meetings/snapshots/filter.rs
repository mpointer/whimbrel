//! Pure per-frame decisions for meeting snapshots: drop junk frames, crop to the shared
//! content, and decide whether a stable frame is a new slide, a build of the last one, or a
//! repeat. A port of the Mac's `SnapshotFrameFilter.swift`, with the same thresholds; the
//! caller runs face detection and OCR and passes the results in.

use std::collections::HashSet;

use crate::capture::frame::{Frame, Rect};

/// dHash Hamming distance at or below this: the same slide.
pub const SAME_SLIDE_DISTANCE: u32 = 6;
/// Consecutive samples at or below this: stable.
pub const STABLE_DISTANCE: u32 = 3;
/// Both frames need this many OCR words to compare by text.
const MIN_TEXT_WORDS: usize = 3;
/// Word-set Jaccard at or above this between samples: stable.
const STABLE_TEXT_SIMILARITY: f64 = 0.9;
/// Word-set Jaccard at or above this to the last kept: the same slide.
const SAME_SLIDE_TEXT_SIMILARITY: f64 = 0.8;

// Measured on the Mac's fixture frames at the 256 px thumbnail. Luminance is 0..=255.
const THUMBNAIL_SIDE: u32 = 256;
/// Every thumbnail pixel within this: the frame hasn't changed.
pub const UNCHANGED_PIXEL_STEP: i32 = 6;
/// The whole frame flatter than this: blank.
const UNIFORM_STD_DEV: f64 = 6.0;
/// |dx| + |dy| above this counts as an edge pixel.
const EDGE_STEP: i32 = 32;
/// Fewer edge pixels: a spinner or loading screen.
const MIN_EDGE_DENSITY: f64 = 0.005;
/// |dx| + |dy| of crisp ink on a contrasting background.
const STRONG_EDGE_STEP: i32 = 128;
/// A cross-fade is mostly mid-grey (64..=191) …
const BLEND_MID_GREY: f64 = 0.5;
/// … has little crisp contrast …
const BLEND_MAX_STRONG_EDGES: f64 = 0.01;
/// … and ink both this much brighter and darker than the median …
const BLEND_INK_STEP: i32 = 48;
/// … each covering at least this share of the frame.
const BLEND_MIN_INK: f64 = 0.002;
/// A face rect grown by this many face sizes per side is about its webcam tile.
const FACE_TILE_MARGIN: f64 = 1.0;
/// The content left beside the faces must cover this much of the frame.
const MIN_CONTENT_AREA: f64 = 0.25;
/// An edge row or column flatter than this is a uniform bar …
const BAR_STD_DEV: f64 = 6.0;
/// … and it ends where the mean luminance steps by this much.
const BAR_STEP: f64 = 40.0;

/// What the service remembers about a sampled frame, after cropping to the shared content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Features {
    /// 64-bit dHash of the cropped frame.
    pub hash: u64,
    /// OCR text of the cropped frame.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Skip,
    WaitForStable,
    KeepNew,
    ReplaceLast,
}

/// Difference hash: each bit says whether a cell of a 9x8 box-averaged grayscale grid is
/// darker than its right neighbour.
pub fn dhash(frame: &Frame) -> u64 {
    let Some(luma) = Luma::thumbnail(frame) else { return 0 };
    let (w, h) = (luma.width, luma.height);
    if w < 9 || h < 8 {
        return 0;
    }
    let cell = |cx: usize, cy: usize| luma.mean(Box2 { x: cx * w / 9..(cx + 1) * w / 9, y: cy * h / 8..(cy + 1) * h / 8 });
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash = hash << 1 | u64::from(cell(x, y) < cell(x + 1, y));
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Blank, loading or spinner, and mid-cross-fade frames.
pub fn is_junk(frame: &Frame) -> bool {
    let Some(luma) = Luma::thumbnail(frame) else { return true };
    let s = luma.stats(luma.bounds());
    if s.std_dev < UNIFORM_STD_DEV || s.edge_density < MIN_EDGE_DENSITY {
        return true;
    }
    // Ink on both sides of a mid-grey median is what separates a light/dark cross-fade from
    // a dark-theme slide, whose soft-contrast text is often lower contrast than the fade.
    s.mid_grey_fraction > BLEND_MID_GREY
        && s.strong_edge_density < BLEND_MAX_STRONG_EDGES
        && s.brighter_ink >= BLEND_MIN_INK
        && s.darker_ink >= BLEND_MIN_INK
}

/// The shared-content rect in frame pixels, or `None` for a participant grid or speaker
/// view. `faces` are in frame pixels too.
pub fn content_crop(frame: &Frame, faces: &[Rect]) -> Option<Rect> {
    let luma = Luma::thumbnail(frame)?;
    let full = RectF { x0: 0.0, y0: 0.0, x1: frame.width as f64, y1: frame.height as f64 };
    let mut region = full;
    if !faces.is_empty() {
        // A face box grown by one face size per side stands in for its webcam tile; a tile
        // much wider than that leaks into the crop (as on the Mac).
        let all = faces
            .iter()
            .map(|f| {
                let (dx, dy) = (f.width as f64 * FACE_TILE_MARGIN, f.height as f64 * FACE_TILE_MARGIN);
                RectF { x0: f.x as f64 - dx, y0: f.y as f64 - dy, x1: (f.x + f.width) as f64 + dx, y1: (f.y + f.height) as f64 + dy }
            })
            .reduce(RectF::union)?;
        let sides = [
            RectF { x1: all.x0, ..full },
            RectF { x0: all.x1, ..full },
            RectF { y1: all.y0, ..full },
            RectF { y0: all.y1, ..full },
        ];
        let largest = sides.into_iter().filter_map(|s| s.intersection(full)).max_by(|a, b| a.area().total_cmp(&b.area()))?;
        if largest.area() < MIN_CONTENT_AREA * full.area() {
            return None;
        }
        region = largest;
    }

    let scale = luma.width as f64 / frame.width as f64;
    let b = luma.trimming_bars(luma.inner(region, scale));
    if b.x.is_empty() || b.y.is_empty() {
        return None;
    }
    // Beside any face, a region with little text is webcam backdrop (grid, speaker view, or
    // one small tile), not shared content. Edge density stands in for "has text", since OCR
    // runs after the crop.
    if !faces.is_empty() && luma.stats(b.clone()).edge_density < MIN_EDGE_DENSITY {
        return None;
    }
    // Back to frame pixels; a cell index at the thumbnail's far edge maps to the exact edge.
    let to_frame = |cell: usize, cells: usize, pixels: u32| if cell == cells { pixels } else { (cell as f64 / scale).round() as u32 };
    let (x0, x1) = (to_frame(b.x.start, luma.width, frame.width), to_frame(b.x.end, luma.width, frame.width));
    let (y0, y1) = (to_frame(b.y.start, luma.height, frame.height), to_frame(b.y.end, luma.height, frame.height));
    Some(Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
}

/// Text first: frames compare by OCR word sets when both have `MIN_TEXT_WORDS`, by dHash
/// otherwise. A 9x8 dHash barely separates same-template text slides, so text decides
/// wherever there is enough of it.
pub fn decide(current: &Features, previous_sample: Option<&Features>, last_kept: Option<&Features>) -> Decision {
    let Some(previous) = previous_sample else { return Decision::WaitForStable };
    let now = words(&current.text);
    if !similar(&now, current.hash, &words(&previous.text), previous.hash, STABLE_TEXT_SIMILARITY, STABLE_DISTANCE) {
        return Decision::WaitForStable;
    }
    let Some(kept) = last_kept else { return Decision::KeepNew };
    let kept_words = words(&kept.text);
    // A build keeps the look of the slide it grows from; a later slide that merely repeats
    // an earlier title (title slide, then content slide) is a new slide, not a build.
    if !kept_words.is_empty()
        && kept_words.is_subset(&now)
        && kept_words.len() < now.len()
        && hamming(current.hash, kept.hash) <= SAME_SLIDE_DISTANCE
    {
        return Decision::ReplaceLast;
    }
    if similar(&now, current.hash, &kept_words, kept.hash, SAME_SLIDE_TEXT_SIMILARITY, SAME_SLIDE_DISTANCE) {
        return Decision::Skip;
    }
    Decision::KeepNew
}

fn similar(a: &HashSet<String>, hash_a: u64, b: &HashSet<String>, hash_b: u64, min_jaccard: f64, max_distance: u32) -> bool {
    if a.len() < MIN_TEXT_WORDS || b.len() < MIN_TEXT_WORDS {
        return hamming(hash_a, hash_b) <= max_distance;
    }
    a.intersection(b).count() as f64 / a.union(b).count() as f64 >= min_jaccard
}

/// Lowercased letter-and-digit runs.
fn words(text: &str) -> HashSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// A rectangle in frame pixels, as floats for the face-tile arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RectF {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl RectF {
    fn union(self, o: RectF) -> RectF {
        RectF { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    /// `None` when they don't overlap with a positive area.
    fn intersection(self, o: RectF) -> Option<RectF> {
        let r = RectF { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    fn area(self) -> f64 {
        (self.x1 - self.x0) * (self.y1 - self.y0)
    }
}

/// A range of thumbnail columns and rows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Box2 {
    x: std::ops::Range<usize>,
    y: std::ops::Range<usize>,
}

struct Stats {
    std_dev: f64,
    edge_density: f64,
    strong_edge_density: f64,
    mid_grey_fraction: f64,
    /// Share more than `BLEND_INK_STEP` above the median.
    brighter_ink: f64,
    /// Share more than `BLEND_INK_STEP` below the median.
    darker_ink: f64,
}

/// An 8-bit grayscale copy of a frame; row 0 is the top row.
#[derive(Debug, Clone)]
pub struct Luma {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Luma {
    /// Area-averaged so the long side is at most `THUMBNAIL_SIDE`.
    pub fn thumbnail(frame: &Frame) -> Option<Luma> {
        let (w, h) = (frame.width as usize, frame.height as usize);
        if w == 0 || h == 0 {
            return None;
        }
        let scale = (THUMBNAIL_SIDE as f64 / w.max(h) as f64).min(1.0);
        let (tw, th) = (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1));
        let mut pixels = Vec::with_capacity(tw * th);
        for ty in 0..th {
            let (sy0, sy1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
            for tx in 0..tw {
                let (sx0, sx1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
                let mut sum = 0u64;
                for y in sy0..sy1 {
                    for x in sx0..sx1 {
                        let p = (y * w + x) * 4;
                        let (b, g, r) = (frame.bgra[p] as u64, frame.bgra[p + 1] as u64, frame.bgra[p + 2] as u64);
                        sum += (299 * r + 587 * g + 114 * b) / 1000;
                    }
                }
                pixels.push((sum / ((sy1 - sy0) * (sx1 - sx0)) as u64) as u8);
            }
        }
        Some(Luma { width: tw, height: th, pixels })
    }

    fn bounds(&self) -> Box2 {
        Box2 { x: 0..self.width, y: 0..self.height }
    }

    fn at(&self, x: usize, y: usize) -> i32 {
        self.pixels[y * self.width + x] as i32
    }

    /// Same size and every pixel within `tolerance` grey levels: the same frame, give or take
    /// capture noise. Far stricter than a dHash match, which a one-bullet build can pass.
    pub fn matches(&self, other: &Luma, tolerance: i32) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.pixels.iter().zip(&other.pixels).all(|(a, b)| (*a as i32 - *b as i32).abs() <= tolerance)
    }

    fn stats(&self, b: Box2) -> Stats {
        let (mut sum, mut sum_squares, mut edges, mut strong_edges) = (0.0, 0.0, 0usize, 0usize);
        let mut histogram = [0usize; 256];
        for y in b.y.clone() {
            for x in b.x.clone() {
                let v = self.at(x, y);
                sum += v as f64;
                sum_squares += (v * v) as f64;
                histogram[v as usize] += 1;
                if x + 1 >= b.x.end || y + 1 >= b.y.end {
                    continue;
                }
                let gradient = (self.at(x + 1, y) - v).abs() + (self.at(x, y + 1) - v).abs();
                if gradient > EDGE_STEP {
                    edges += 1;
                }
                if gradient > STRONG_EDGE_STEP {
                    strong_edges += 1;
                }
            }
        }
        let count = b.x.len() * b.y.len();
        let n = count.max(1) as f64;
        let mean = sum / n;
        let (mut median, mut seen) = (0i32, histogram[0]);
        while seen * 2 < count && median < 255 {
            median += 1;
            seen += histogram[median as usize];
        }
        let share = |lo: i32, hi: i32| histogram[lo as usize..=hi as usize].iter().sum::<usize>() as f64 / n;
        Stats {
            std_dev: (sum_squares / n - mean * mean).max(0.0).sqrt(),
            edge_density: edges as f64 / n,
            strong_edge_density: strong_edges as f64 / n,
            mid_grey_fraction: share(64, 191),
            brighter_ink: if median + BLEND_INK_STEP < 255 { share(median + BLEND_INK_STEP + 1, 255) } else { 0.0 },
            darker_ink: if median - BLEND_INK_STEP > 0 { share(0, median - BLEND_INK_STEP - 1) } else { 0.0 },
        }
    }

    fn mean(&self, b: Box2) -> f64 {
        let mut sum = 0i64;
        for y in b.y.clone() {
            for x in b.x.clone() {
                sum += self.at(x, y) as i64;
            }
        }
        sum as f64 / (b.x.len() * b.y.len()).max(1) as f64
    }

    /// Thumbnail cells wholly inside `r` (frame pixels), so the crop never reaches past it.
    fn inner(&self, r: RectF, scale: f64) -> Box2 {
        let x0 = ((r.x0 * scale).ceil().max(0.0) as usize).min(self.width);
        let x1 = ((r.x1 * scale).floor().max(0.0) as usize).min(self.width).max(x0);
        let y0 = ((r.y0 * scale).ceil().max(0.0) as usize).min(self.height);
        let y1 = ((r.y1 * scale).floor().max(0.0) as usize).min(self.height).max(y0);
        Box2 { x: x0..x1, y: y0..y1 }
    }

    /// Drops uniform bars at each edge of `b`. A bar is a run of flat rows or columns that
    /// ends in a step to a different luminance; a flat run that fades into same-coloured
    /// content is a margin of that content and stays.
    fn trimming_bars(&self, mut b: Box2) -> Box2 {
        fn bar_length(indices: &[usize], flat: &dyn Fn(usize) -> bool, level: &dyn Fn(usize) -> f64) -> usize {
            let Some(&first) = indices.first() else { return 0 };
            if !flat(first) {
                return 0;
            }
            let base = level(first);
            let mut k = 1;
            while k < indices.len() && flat(indices[k]) && (level(indices[k]) - base).abs() < BAR_STEP {
                k += 1;
            }
            if k < indices.len() && (level(indices[k]) - base).abs() >= BAR_STEP {
                k
            } else {
                0
            }
        }
        let ys = b.y.clone();
        let flat_column = |x: usize| self.stats(Box2 { x: x..x + 1, y: ys.clone() }).std_dev < BAR_STD_DEV;
        let column_level = |x: usize| self.mean(Box2 { x: x..x + 1, y: ys.clone() });
        let xs: Vec<usize> = b.x.clone().collect();
        let left = bar_length(&xs, &flat_column, &column_level);
        let reversed: Vec<usize> = xs.iter().rev().copied().collect();
        let right = bar_length(&reversed, &flat_column, &column_level);
        let start = b.x.start + left;
        b.x = start..b.x.end.saturating_sub(right).max(start);

        let xr = b.x.clone();
        let flat_row = |y: usize| self.stats(Box2 { x: xr.clone(), y: y..y + 1 }).std_dev < BAR_STD_DEV;
        let row_level = |y: usize| self.mean(Box2 { x: xr.clone(), y: y..y + 1 });
        let ys: Vec<usize> = b.y.clone().collect();
        let top = bar_length(&ys, &flat_row, &row_level);
        let reversed: Vec<usize> = ys.iter().rev().copied().collect();
        let bottom = bar_length(&reversed, &flat_row, &row_level);
        let start = b.y.start + top;
        b.y = start..b.y.end.saturating_sub(bottom).max(start);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::snapshots::fixtures as fx;

    const ROADMAP: [&str; 4] = ["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"];

    fn contains(outer: Rect, inner: Rect) -> bool {
        inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.width <= outer.x + outer.width && inner.y + inner.height <= outer.y + outer.height
    }

    fn overlap_area(a: Rect, b: Rect) -> u64 {
        let w = (a.x + a.width).min(b.x + b.width).saturating_sub(a.x.max(b.x)) as u64;
        let h = (a.y + a.height).min(b.y + b.height).saturating_sub(a.y.max(b.y)) as u64;
        w * h
    }

    // --- is_junk

    #[test]
    fn a_solid_frame_is_junk() {
        assert!(is_junk(&fx::solid(0.15)));
    }

    #[test]
    fn a_spinner_frame_is_junk() {
        assert!(is_junk(&fx::spinner()));
    }

    #[test]
    fn a_text_slide_is_not_junk() {
        assert!(!is_junk(&fx::slide(&ROADMAP)));
    }

    #[test]
    fn a_dark_theme_slide_is_not_junk() {
        assert!(!is_junk(&fx::slide_on(&ROADMAP, 0.35, 0.95)));
    }

    #[test]
    fn a_cross_fade_is_junk() {
        let dark = fx::slide_on(&["Agenda", "Intros", "Demo", "Questions"], 0.0, 1.0);
        assert!(is_junk(&fx::blend(&dark, &fx::slide(&ROADMAP))));
    }

    // --- content_crop

    #[test]
    fn a_face_grid_has_no_crop() {
        let (image, faces) = fx::face_grid(2, 2);
        assert_eq!(faces.len(), 4);
        assert_eq!(content_crop(&image, &faces), None);
    }

    #[test]
    fn a_slide_with_a_side_filmstrip_crops_to_the_slide() {
        let (image, faces, slide) = fx::slide_with_filmstrip();
        let crop = content_crop(&image, &faces).expect("a crop");
        assert!(contains(slide, crop), "crop {crop:?} leaves slide {slide:?}");
        assert!(overlap_area(crop, slide) as f64 >= 0.9 * (slide.width as f64 * slide.height as f64), "{crop:?}");
    }

    #[test]
    fn a_slide_with_a_top_filmstrip_crops_to_the_slide() {
        let (image, faces, slide) = fx::slide_with_top_filmstrip();
        let crop = content_crop(&image, &faces).expect("a crop");
        assert!(contains(slide, crop), "crop {crop:?} leaves slide {slide:?}");
        assert!(overlap_area(crop, slide) as f64 >= 0.9 * (slide.width as f64 * slide.height as f64), "{crop:?}");
    }

    #[test]
    fn a_single_large_face_speaker_view_has_no_crop() {
        // 420² of 1280x720 is about 19% of the frame.
        let (image, faces) = fx::webcam(420.0);
        assert_eq!(content_crop(&image, &faces), None);
    }

    #[test]
    fn a_single_small_face_on_a_webcam_background_has_no_crop() {
        // 120² is about 1.6% of the frame; its tile leaves a 460 px strip (36%) beside it.
        let (image, faces) = fx::webcam(120.0);
        assert_eq!(content_crop(&image, &faces), None);
    }

    #[test]
    fn a_face_leaving_under_a_quarter_of_the_frame_has_no_crop() {
        // A mid-size face on a text slide: its 900 px tile leaves 190 px strips (15%) each side.
        let face = Rect { x: 490, y: 210, width: 300, height: 300 };
        assert_eq!(content_crop(&fx::slide(&ROADMAP), &[face]), None);
    }

    #[test]
    fn a_slide_without_faces_keeps_the_whole_frame() {
        let image = fx::slide(&ROADMAP);
        assert_eq!(content_crop(&image, &[]), Some(Rect { x: 0, y: 0, width: image.width, height: image.height }));
    }

    // --- decide

    fn features(hash: u64, text: &str) -> Features {
        Features { hash, text: text.into() }
    }

    #[test]
    fn the_first_sample_waits_for_stable() {
        assert_eq!(decide(&features(1, ""), None, None), Decision::WaitForStable);
    }

    #[test]
    fn a_changing_frame_waits_for_stable() {
        assert_eq!(decide(&features(0, ""), Some(&features(0xFF, "")), None), Decision::WaitForStable);
    }

    #[test]
    fn the_first_stable_frame_keeps_new() {
        assert_eq!(decide(&features(0, ""), Some(&features(0, "")), None), Decision::KeepNew);
    }

    #[test]
    fn an_unchanged_slide_is_skipped() {
        let slide = features(0xF0F0, "Roadmap Q1");
        assert_eq!(decide(&slide, Some(&slide), Some(&slide)), Decision::Skip);
    }

    #[test]
    fn a_build_replaces_the_last() {
        // 4 bits from the kept slide: a build looks alike.
        let current = features(0xF, "Roadmap Q1 Q2");
        assert_eq!(decide(&current, Some(&current), Some(&features(0, "Roadmap Q1"))), Decision::ReplaceLast);
    }

    #[test]
    fn a_new_slide_keeps_new() {
        let current = features(0xFFFF, "Hiring plan");
        assert_eq!(decide(&current, Some(&current), Some(&features(0, "Roadmap Q1"))), Decision::KeepNew);
    }

    #[test]
    fn the_same_words_are_not_a_build() {
        let current = features(0xFFFF, "roadmap, Q1!");
        assert_eq!(decide(&current, Some(&current), Some(&features(0, "Roadmap Q1"))), Decision::KeepNew);
    }

    #[test]
    fn a_build_beats_same_slide_when_hashes_match() {
        let current = features(0, "Roadmap Q1 Q2");
        assert_eq!(decide(&current, Some(&current), Some(&features(0, "Roadmap Q1"))), Decision::ReplaceLast);
    }

    #[test]
    fn text_decides_stability_when_both_have_enough_words() {
        let current = features(0, "Hiring plan for next quarter");
        assert_eq!(decide(&current, Some(&features(0, "Roadmap Q1 Q2 Q3")), None), Decision::WaitForStable);
    }

    // --- decide on rendered frames, with Windows OCR as the service will run it

    /// Features as the service computes them: dHash plus OCR of the whole frame.
    fn rendered(ocr: &crate::capture::ocr::Ocr, frame: &Frame) -> Features {
        Features { hash: dhash(frame), text: ocr.recognize(frame).unwrap() }
    }

    /// The decision for `next` sampled twice in a row after `kept` was saved.
    fn decide_recaptured(next: &Frame, kept: &Frame) -> Decision {
        let ocr = crate::capture::ocr::Ocr::new().expect("the Windows OCR engine");
        let current = rendered(&ocr, next);
        decide(&current, Some(&rendered(&ocr, next)), Some(&rendered(&ocr, kept)))
    }

    #[test]
    fn ocr_reads_the_fixture_slides_so_text_decides() {
        // Without this, the rendered-frame tests could pass on dHash alone.
        let ocr = crate::capture::ocr::Ocr::new().expect("the Windows OCR engine");
        let text = rendered(&ocr, &fx::slide(&ROADMAP)).text.to_lowercase();
        for word in ["roadmap", "meeting", "snapshots", "calendar", "faster", "search"] {
            assert!(text.contains(word), "OCR missed {word:?} in {text:?}");
        }
        let dark = rendered(&ocr, &fx::slide_on(&ROADMAP, 0.35, 0.95)).text.to_lowercase();
        assert!(dark.contains("roadmap"), "dark theme: {dark:?}");
    }

    #[test]
    fn two_different_text_slides_on_one_template_are_both_kept() {
        let hiring = fx::slide(&["Hiring plan", "Two engineers", "One designer", "Support lead"]);
        assert_eq!(decide_recaptured(&hiring, &fx::slide(&ROADMAP)), Decision::KeepNew);
    }

    #[test]
    fn a_build_step_replaces_the_last_with_real_frames() {
        assert_eq!(decide_recaptured(&fx::slide(&ROADMAP[..3]), &fx::slide(&ROADMAP[..2])), Decision::ReplaceLast);
    }

    #[test]
    fn a_title_slide_then_a_content_slide_with_the_same_title_keeps_new() {
        let content = fx::slide(&["Q3 Review", "Revenue up 12%", "Churn down to 3%", "Two launches shipped", "Hiring on plan", "Next: Q4 goals"]);
        assert_eq!(decide_recaptured(&content, &fx::slide(&["Q3 Review"])), Decision::KeepNew);
    }

    #[test]
    fn the_same_slide_recaptured_is_skipped_with_real_frames() {
        assert_eq!(decide_recaptured(&fx::slide(&ROADMAP), &fx::slide(&ROADMAP)), Decision::Skip);
    }

    #[test]
    fn two_different_diagrams_with_little_text_are_both_kept() {
        assert_eq!(decide_recaptured(&fx::bar_chart(), &fx::flowchart()), Decision::KeepNew);
    }

    // --- dhash

    #[test]
    fn identical_frames_hash_zero_apart() {
        assert_eq!(hamming(dhash(&fx::slide(&ROADMAP)), dhash(&fx::slide(&ROADMAP))), 0);
    }

    #[test]
    fn hamming_counts_differing_bits() {
        assert_eq!(hamming(0b1011, 0b0110), 3);
    }

    #[test]
    fn an_unchanged_frame_matches_and_a_build_does_not() {
        let a = Luma::thumbnail(&fx::slide(&ROADMAP[..2])).unwrap();
        assert!(a.matches(&Luma::thumbnail(&fx::slide(&ROADMAP[..2])).unwrap(), UNCHANGED_PIXEL_STEP));
        assert!(!a.matches(&Luma::thumbnail(&fx::slide(&ROADMAP[..3])).unwrap(), UNCHANGED_PIXEL_STEP));
    }
}
