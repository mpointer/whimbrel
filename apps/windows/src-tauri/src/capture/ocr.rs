//! OCR fallback without persisting a screenshot — docs/12 §4.2.
//!
//! One frame of one window via `Windows.Graphics.Capture` (`capture::frame`) → a
//! `SoftwareBitmap` in memory → `Windows.Media.Ocr`. The frame is never encoded or written
//! anywhere; it is dropped when [`Ocr::recognize_window`] returns (FR-CAP-3 by
//! construction). [`Ocr::recognize`] reads a frame someone else grabbed: meeting snapshots
//! OCR the slide they crop out of the meeting window.
//!
//! `OcrEngine` rejects images larger than `OcrEngine::MaxImageDimension` on either side.
//! That limit is read at runtime, not assumed: it was 2600 px on early Windows 10
//! builds and is 10000 px on the Windows 11 build M3.1 ran on (26340), where no window
//! needed downscaling. Frames that exceed it are downscaled in memory first.

use std::time::Instant;

use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
use windows::Graphics::Imaging::SoftwareBitmap;
use windows::Media::Ocr::OcrEngine;
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Foundation::HWND;

use crate::capture::frame::{Frame, Grabber};

/// The one-time borderless-capture consent (§4.2). Returns the status Windows reports;
/// whether an *unpackaged* exe gets `Allowed` (and whether the border then actually
/// disappears) is one of the things M3.1 records rather than assumes.
pub fn request_borderless_access() -> windows::core::Result<AppCapabilityAccessStatus> {
    GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)?.join()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OcrOutput {
    pub chars: usize,
    pub lines: u32,
    pub capture_ms: f64,
    pub ocr_ms: f64,
    pub frame_width: u32,
    pub frame_height: u32,
    /// Size handed to the OCR engine (smaller than the frame when downscaled).
    pub ocr_width: u32,
    pub ocr_height: u32,
    /// `Ok` if `IsBorderRequired = false` was accepted, else the error text.
    pub border_disable: Result<(), String>,
    #[serde(skip)]
    pub text: String,
}

pub struct Ocr {
    grabber: Grabber,
    engine: OcrEngine,
    max_dim: u32,
}

impl Ocr {
    pub fn new() -> windows::core::Result<Self> {
        let grabber = Grabber::new()?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
        let max_dim = OcrEngine::MaxImageDimension()?;
        Ok(Self { grabber, engine, max_dim })
    }

    pub fn recognize_window(&self, hwnd: HWND) -> windows::core::Result<OcrOutput> {
        let t0 = Instant::now();
        let grab = self.grabber.grab(hwnd)?;
        let (frame_width, frame_height) = (grab.bitmap.PixelWidth()? as u32, grab.bitmap.PixelHeight()? as u32);
        let bitmap = self.fit_for_ocr(grab.bitmap)?;
        let (ocr_width, ocr_height) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let capture_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        let (text, lines) = self.read(&bitmap)?;
        let ocr_ms = t1.elapsed().as_secs_f64() * 1000.0;

        Ok(OcrOutput {
            chars: crate::capture::uia::count_chars(&text),
            lines,
            capture_ms,
            ocr_ms,
            frame_width,
            frame_height,
            ocr_width,
            ocr_height,
            border_disable: grab.border_disable,
            text,
        })
    }

    /// The text in a frame already in memory.
    pub fn recognize(&self, frame: &Frame) -> windows::core::Result<String> {
        let frame = match fit_within(frame.width, frame.height, self.max_dim) {
            Some((nw, nh)) => frame.downscaled(nw, nh),
            None => frame.clone(),
        };
        Ok(self.read(&frame.to_bitmap()?)?.0)
    }

    fn read(&self, bitmap: &SoftwareBitmap) -> windows::core::Result<(String, u32)> {
        let result = self.engine.RecognizeAsync(bitmap)?.join()?;
        Ok((result.Text()?.to_string(), result.Lines()?.Size()?))
    }

    /// Returns `bitmap` unchanged if it fits `MaxImageDimension`, else a bilinear
    /// downscale that does. All in memory.
    fn fit_for_ocr(&self, bitmap: SoftwareBitmap) -> windows::core::Result<SoftwareBitmap> {
        let (w, h) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let Some((nw, nh)) = fit_within(w, h, self.max_dim) else {
            return Ok(bitmap);
        };
        Frame::from_bitmap(&bitmap)?.downscaled(nw, nh).to_bitmap()
    }
}

/// The largest size with `w:h`'s aspect ratio whose sides are both `<= max_dim`, or
/// `None` if `w x h` already fits.
fn fit_within(w: u32, h: u32, max_dim: u32) -> Option<(u32, u32)> {
    if w <= max_dim && h <= max_dim {
        return None;
    }
    let scale = max_dim as f64 / w.max(h) as f64;
    Some((((w as f64 * scale).floor() as u32).max(1), ((h as f64 * scale).floor() as u32).max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_keeps_small_frames_and_scales_the_long_side() {
        assert_eq!(fit_within(1920, 1080, 2600), None);
        assert_eq!(fit_within(2880, 1800, 2600), Some((2600, 1625)));
        assert_eq!(fit_within(1200, 3000, 2600), Some((1040, 2600)));
    }

    /// `cargo test --lib -- --ignored recognizes_text_in_a_frame`: needs an OCR language
    /// pack (any Windows install with an English UI has one).
    #[test]
    #[ignore = "needs the Windows OCR engine"]
    fn recognizes_text_in_a_frame() {
        let ocr = Ocr::new().unwrap();
        // A white frame with no text reads as empty, not as an error.
        let blank = Frame { bgra: vec![255; 400 * 100 * 4], width: 400, height: 100 };
        assert_eq!(ocr.recognize(&blank).unwrap().trim(), "");
    }
}
