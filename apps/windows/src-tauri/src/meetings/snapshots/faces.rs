//! Face rectangles for the snapshot crop (`filter::content_crop`), from
//! `Windows.Media.FaceAnalysis.FaceDetector`: the Mac's Vision face pass, on Windows.
//!
//! The detector takes `Gray8` (or `Nv12`) bitmaps, and its time grows with the image, so a
//! frame is downscaled to `DETECT_SIDE` on its long side first and the boxes scaled back.
//! Webcam tiles in a filmstrip are small, so the smallest detectable face is lowered to
//! `MIN_FACE` pixels at that size.

use windows::Graphics::Imaging::{BitmapPixelFormat, BitmapSize, SoftwareBitmap};
use windows::Media::FaceAnalysis::FaceDetector;

use crate::capture::frame::{Frame, Rect};

/// The long side frames are detected at.
const DETECT_SIDE: u32 = 1280;
/// The smallest face looked for, in pixels at `DETECT_SIDE`: a filmstrip tile's face.
const MIN_FACE: u32 = 24;

pub struct Faces {
    detector: FaceDetector,
}

impl Faces {
    /// `Err` where Windows has no face detector (some N editions): the caller then crops
    /// without faces, which keeps grids out only by their lack of text.
    pub fn new() -> windows::core::Result<Self> {
        let detector = FaceDetector::CreateAsync()?.join()?;
        let min = detector.MinDetectableFaceSize()?;
        if min.Width > MIN_FACE {
            let _ = detector.SetMinDetectableFaceSize(BitmapSize { Width: MIN_FACE, Height: MIN_FACE });
        }
        Ok(Self { detector })
    }

    /// The faces in `frame`, in its own pixels.
    pub fn detect(&self, frame: &Frame) -> windows::core::Result<Vec<Rect>> {
        let (small, factor) = match detect_size(frame.width, frame.height) {
            Some((w, h)) => (frame.downscaled(w, h), frame.width as f64 / w as f64),
            None => (frame.clone(), 1.0),
        };
        let grey = SoftwareBitmap::Convert(&small.to_bitmap()?, BitmapPixelFormat::Gray8)?;
        let found = self.detector.DetectFacesAsync(&grey)?.join()?;
        let mut boxes = Vec::new();
        for face in found {
            let b = face.FaceBox()?;
            boxes.push(Rect { x: b.X, y: b.Y, width: b.Width, height: b.Height });
        }
        Ok(scale_back(&boxes, factor, frame.width, frame.height))
    }
}

/// The size to detect at, or `None` if the frame is already small enough.
fn detect_size(w: u32, h: u32) -> Option<(u32, u32)> {
    let long = w.max(h);
    if long <= DETECT_SIDE {
        return None;
    }
    let scale = DETECT_SIDE as f64 / long as f64;
    Some((((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1)))
}

/// Boxes found on a frame downscaled by `factor`, in the original frame's pixels and
/// clipped to it.
fn scale_back(boxes: &[Rect], factor: f64, width: u32, height: u32) -> Vec<Rect> {
    boxes
        .iter()
        .filter_map(|b| {
            let x = ((b.x as f64 * factor).round() as u32).min(width);
            let y = ((b.y as f64 * factor).round() as u32).min(height);
            let w = ((b.width as f64 * factor).round() as u32).min(width - x);
            let h = ((b.height as f64 * factor).round() as u32).min(height - y);
            (w > 0 && h > 0).then_some(Rect { x, y, width: w, height: h })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::snapshots::fixtures as fx;

    #[test]
    fn a_frame_up_to_the_detect_size_is_not_scaled() {
        assert_eq!(detect_size(1280, 720), None);
        assert_eq!(detect_size(3840, 2160), Some((1280, 720)));
        assert_eq!(detect_size(1440, 2560), Some((720, 1280)));
    }

    #[test]
    fn boxes_scale_back_to_the_frame_and_are_clipped_to_it() {
        let found = [Rect { x: 100, y: 50, width: 40, height: 40 }, Rect { x: 1270, y: 700, width: 30, height: 30 }];
        assert_eq!(
            scale_back(&found, 3.0, 3840, 2160),
            vec![Rect { x: 300, y: 150, width: 120, height: 120 }, Rect { x: 3810, y: 2100, width: 30, height: 60 }]
        );
        assert_eq!(scale_back(&[Rect { x: 1280, y: 0, width: 10, height: 10 }], 3.0, 3840, 2160), vec![], "nothing left inside");
    }

    #[test]
    fn frames_without_people_have_no_faces() {
        let faces = Faces::new().expect("the Windows face detector");
        assert_eq!(faces.detect(&fx::solid(0.5)).unwrap(), vec![]);
        assert_eq!(faces.detect(&fx::slide(&["Roadmap", "Ship meeting snapshots", "Calendar sync"])).unwrap(), vec![]);
        assert_eq!(faces.detect(&fx::bar_chart()).unwrap(), vec![]);
    }

    #[test]
    fn a_large_frame_is_detected_without_error() {
        let faces = Faces::new().expect("the Windows face detector");
        let big = fx::slide(&["Roadmap"]).downscaled(3840, 2160);
        assert_eq!(faces.detect(&big).unwrap(), vec![]);
    }
}
