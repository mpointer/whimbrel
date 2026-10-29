//! Meeting frames drawn in code for the snapshot tests, the Mac's `SnapshotFixtures.swift`
//! in top-left coordinates. Slide text is real text (GDI, Arial), so OCR can read it. Every
//! returned rect is in frame pixels.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GdiFlush, SelectObject, SetBkMode, SetTextAlign,
    SetTextColor, TextOutW, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DIB_RGB_COLORS, FF_SWISS, FW_NORMAL, OUT_DEFAULT_PRECIS, TA_BASELINE, TA_LEFT, TRANSPARENT,
};
use windows::Win32::Foundation::COLORREF;

use crate::capture::frame::{Frame, Rect};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

/// A BGRA canvas drawn in greys from 0.0 (black) to 1.0 (white).
struct Canvas {
    frame: Frame,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { frame: Frame { bgra: vec![255; (WIDTH * HEIGHT * 4) as usize], width: WIDTH, height: HEIGHT } }
    }

    fn set(&mut self, x: i64, y: i64, grey: f64) {
        if x < 0 || y < 0 || x >= WIDTH as i64 || y >= HEIGHT as i64 {
            return;
        }
        let v = (grey * 255.0).round().clamp(0.0, 255.0) as u8;
        let p = ((y as u32 * WIDTH + x as u32) * 4) as usize;
        self.frame.bgra[p..p + 4].copy_from_slice(&[v, v, v, 255]);
    }

    fn fill(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, grey: f64) {
        for y in y0.round() as i64..y1.round() as i64 {
            for x in x0.round() as i64..x1.round() as i64 {
                self.set(x, y, grey);
            }
        }
    }

    fn fill_ellipse(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, grey: f64) {
        let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
        for y in y0.floor() as i64..y1.ceil() as i64 {
            for x in x0.floor() as i64..x1.ceil() as i64 {
                let (dx, dy) = ((x as f64 + 0.5 - cx) / rx, (y as f64 + 0.5 - cy) / ry);
                if dx * dx + dy * dy <= 1.0 {
                    self.set(x, y, grey);
                }
            }
        }
    }

    /// An outlined rectangle, `line` pixels wide, centred on its edges.
    fn stroke_rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, line: f64, grey: f64) {
        let h = line / 2.0;
        self.fill(x0 - h, y0 - h, x1 + h, y0 + h, grey);
        self.fill(x0 - h, y1 - h, x1 + h, y1 + h, grey);
        self.fill(x0 - h, y0 - h, x0 + h, y1 + h, grey);
        self.fill(x1 - h, y0 - h, x1 + h, y1 + h, grey);
    }

    /// Text with its baseline at `baseline`, in Arial at `px` pixels.
    fn text(&mut self, x: i32, baseline: i32, text: &str, px: i32, ink: f64) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let face: Vec<u16> = "Arial\0".encode_utf16().collect();
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: WIDTH as i32,
                    biHeight: -(HEIGHT as i32), // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("a DIB section");
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, self.frame.bgra.len());
            pixels.copy_from_slice(&self.frame.bgra);
            let old_bitmap = SelectObject(dc, dib.into());
            let font = CreateFontW(
                -px,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                FF_SWISS.0 as u32,
                PCWSTR(face.as_ptr()),
            );
            let old_font = SelectObject(dc, font.into());
            let v = (ink * 255.0).round() as u32;
            SetTextColor(dc, COLORREF(v | v << 8 | v << 16));
            SetBkMode(dc, TRANSPARENT);
            SetTextAlign(dc, TA_BASELINE | TA_LEFT);
            let _ = TextOutW(dc, x, baseline, &wide);
            let _ = GdiFlush();
            self.frame.bgra.copy_from_slice(pixels);
            SelectObject(dc, old_font);
            SelectObject(dc, old_bitmap);
            let _ = DeleteObject(font.into());
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(dc);
        }
        // GDI leaves alpha at 0 where it drew; every pixel here is opaque.
        for px in self.frame.bgra.chunks_mut(4) {
            px[3] = 255;
        }
    }

    /// A slide in `(x0, y0, x1, y1)`: a title line then bullet lines, `ink` on `background`.
    fn slide(&mut self, lines: &[&str], r: (f64, f64, f64, f64), background: f64, ink: f64) {
        let (x0, y0, x1, y1) = r;
        self.fill(x0, y0, x1, y1, background);
        let x = (x0 + (x1 - x0) * 0.08).round() as i32;
        let mut baseline = y0 + (y1 - y0) * 0.18;
        for (i, line) in lines.iter().enumerate() {
            let text = if i == 0 { line.to_string() } else { format!("• {line}") };
            self.text(x, baseline.round() as i32, &text, if i == 0 { 56 } else { 36 }, ink);
            baseline += if i == 0 { 96.0 } else { 64.0 };
        }
    }

    /// A webcam tile stand-in (backdrop, shoulders, face oval); returns the face rect.
    fn tile(&mut self, r: (f64, f64, f64, f64), face_side: Option<f64>) -> Rect {
        let (x0, y0, x1, y1) = r;
        self.fill(x0, y0, x1, y1, 0.3);
        let side = face_side.unwrap_or((x1 - x0).min(y1 - y0) * 0.45);
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (face_top, face_bottom) = (my - side * 0.7, my + side * 0.3);
        self.fill_ellipse(mx - side, face_bottom, mx + side, y1, 0.2);
        self.fill_ellipse(mx - side / 2.0, face_top, mx + side / 2.0, face_bottom, 0.7);
        Rect { x: (mx - side / 2.0).round() as u32, y: face_top.round() as u32, width: side.round() as u32, height: side.round() as u32 }
    }
}

fn inset(r: (f64, f64, f64, f64), d: f64) -> (f64, f64, f64, f64) {
    (r.0 + d, r.1 + d, r.2 - d, r.3 - d)
}

const FULL: (f64, f64, f64, f64) = (0.0, 0.0, WIDTH as f64, HEIGHT as f64);

/// A flat frame, the shape of a blank or "waiting for host" screen.
pub fn solid(grey: f64) -> Frame {
    let mut c = Canvas::new();
    c.fill(0.0, 0.0, WIDTH as f64, HEIGHT as f64, grey);
    c.frame
}

/// A small loading arc on a flat background.
pub fn spinner() -> Frame {
    let mut c = Canvas::new();
    c.fill(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 0.12);
    let (cx, cy, r) = (WIDTH as f64 / 2.0, HEIGHT as f64 / 2.0, 24.0);
    for y in (cy - r - 4.0) as i64..(cy + r + 4.0) as i64 {
        for x in (cx - r - 4.0) as i64..(cx + r + 4.0) as i64 {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
            if ((dx * dx + dy * dy).sqrt() - r).abs() <= 3.0 && angle <= std::f64::consts::PI * 1.4 {
                c.set(x, y, 0.85);
            }
        }
    }
    c.frame
}

/// A slide: dark text on white.
pub fn slide(lines: &[&str]) -> Frame {
    slide_on(lines, 1.0, 0.0)
}

pub fn slide_on(lines: &[&str], background: f64, ink: f64) -> Frame {
    let mut c = Canvas::new();
    c.slide(lines, FULL, background, ink);
    c.frame
}

/// A text-free flowchart: three boxes left to right joined by lines, on white.
pub fn flowchart() -> Frame {
    let mut c = Canvas::new();
    c.fill(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0);
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    let (bw, bh) = (w * 0.2, h * 0.25);
    for i in 0..3 {
        let x = w * (0.08 + 0.32 * i as f64);
        c.stroke_rect(x, h / 2.0 - bh / 2.0, x + bw, h / 2.0 + bh / 2.0, 6.0, 0.1);
        if i < 2 {
            c.fill(x + bw, h / 2.0 - 3.0, x + w * 0.32, h / 2.0 + 3.0, 0.1);
        }
    }
    c.frame
}

/// A text-free bar chart: five filled bars of varying height rising from a baseline.
pub fn bar_chart() -> Frame {
    let mut c = Canvas::new();
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    c.fill(0.0, 0.0, w, h, 1.0);
    for (i, height) in [0.3, 0.55, 0.4, 0.75, 0.6].into_iter().enumerate() {
        let x = w * (0.12 + 0.16 * i as f64);
        let bottom = h - h * 0.12;
        c.fill(x, bottom - h * height, x + w * 0.1, bottom, 0.25);
    }
    c.frame
}

/// A participant grid: one webcam tile per participant, a face oval in each.
pub fn face_grid(columns: u32, rows: u32) -> (Frame, Vec<Rect>) {
    let mut c = Canvas::new();
    c.fill(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 0.1);
    let (tw, th) = (WIDTH as f64 / columns as f64, HEIGHT as f64 / rows as f64);
    let mut faces = Vec::new();
    for col in 0..columns {
        for row in 0..rows {
            let t = (col as f64 * tw, row as f64 * th, (col + 1) as f64 * tw, (row + 1) as f64 * th);
            faces.push(c.tile(inset(t, 4.0), None));
        }
    }
    (c.frame, faces)
}

/// A shared slide on the left 75% and a filmstrip of three webcam tiles on the right.
pub fn slide_with_filmstrip() -> (Frame, Vec<Rect>, Rect) {
    let mut c = Canvas::new();
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    c.fill(0.0, 0.0, w, h, 0.1);
    let slide_right = w * 0.75;
    c.slide(&["Quarterly roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search", "Windows parity"], (0.0, 0.0, slide_right, h), 1.0, 0.0);
    let tile_h = h / 3.0;
    let faces = (0..3).map(|i| c.tile(inset((slide_right, i as f64 * tile_h, w, (i + 1) as f64 * tile_h), 8.0), None)).collect();
    (c.frame, faces, Rect { x: 0, y: 0, width: slide_right as u32, height: HEIGHT })
}

/// Zoom's default layout: a row of four webcam tiles across the top, the shared slide below.
pub fn slide_with_top_filmstrip() -> (Frame, Vec<Rect>, Rect) {
    let mut c = Canvas::new();
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    c.fill(0.0, 0.0, w, h, 0.1);
    let strip = h * 0.2;
    c.slide(&["Launch checklist", "Docs updated", "Pricing page live", "Support briefed"], (0.0, strip, w, h), 1.0, 0.0);
    let tile_w = w / 4.0;
    let faces = (0..4).map(|i| c.tile(inset((i as f64 * tile_w, 0.0, (i + 1) as f64 * tile_w, strip), 8.0), None)).collect();
    (c.frame, faces, Rect { x: 0, y: strip as u32, width: WIDTH, height: HEIGHT - strip as u32 })
}

/// One webcam filling the frame, with a face oval `face_side` pixels across at its centre.
pub fn webcam(face_side: f64) -> (Frame, Vec<Rect>) {
    let mut c = Canvas::new();
    let face = c.tile(FULL, Some(face_side));
    (c.frame, vec![face])
}

/// `a` and `b` composited at 50%, the frame caught mid cross-fade.
pub fn blend(a: &Frame, b: &Frame) -> Frame {
    let bgra = a.bgra.iter().zip(&b.bgra).map(|(x, y)| ((*x as u16 + *y as u16) / 2) as u8).collect();
    Frame { bgra, width: a.width, height: a.height }
}
