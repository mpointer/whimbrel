//! One window's pixels via `Windows.Graphics.Capture` (docs/12 §4.2), shared by the OCR
//! fallback and meeting snapshots.
//!
//! The OCR fallback keeps a frame in memory and drops it (FR-CAP-3). Meeting snapshots
//! (docs/superpowers/plans/2026-10-28-windows-meeting-snapshots.md) are the one caller
//! that writes a frame to disk, and only while a meeting records.

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;

const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

/// Tightly packed BGRA8 pixels, `width * height * 4` bytes, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A rectangle in frame pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    /// The part of the frame inside `r`, clipped to the frame. `None` if nothing is left.
    pub fn crop(&self, r: Rect) -> Option<Frame> {
        let x1 = r.x.saturating_add(r.width).min(self.width);
        let y1 = r.y.saturating_add(r.height).min(self.height);
        if r.x >= x1 || r.y >= y1 {
            return None;
        }
        let (w, h) = (x1 - r.x, y1 - r.y);
        let mut bgra = Vec::with_capacity(w as usize * h as usize * 4);
        for y in r.y..y1 {
            let row = (y as usize * self.width as usize + r.x as usize) * 4;
            bgra.extend_from_slice(&self.bgra[row..row + w as usize * 4]);
        }
        Some(Frame { bgra, width: w, height: h })
    }

    /// A bilinear downscale to `nw x nh`.
    pub fn downscaled(&self, nw: u32, nh: u32) -> Frame {
        Frame { bgra: downscale_bgra(&self.bgra, self.width, self.height, nw, nh), width: nw, height: nh }
    }

    /// The frame as a `SoftwareBitmap`, which `Windows.Media.Ocr` and `FaceDetector` take.
    pub fn to_bitmap(&self) -> windows::core::Result<SoftwareBitmap> {
        let out = SoftwareBitmap::CreateWithAlpha(BitmapPixelFormat::Bgra8, self.width as i32, self.height as i32, BitmapAlphaMode::Premultiplied)?;
        write_bgra(&out, &self.bgra)?;
        Ok(out)
    }

    pub fn from_bitmap(bitmap: &SoftwareBitmap) -> windows::core::Result<Frame> {
        Ok(Frame { bgra: read_bgra(bitmap)?, width: bitmap.PixelWidth()? as u32, height: bitmap.PixelHeight()? as u32 })
    }
}

/// One grabbed frame and whether `IsBorderRequired = false` was accepted (the error text if not).
pub struct Grab {
    pub bitmap: SoftwareBitmap,
    pub border_disable: Result<(), String>,
}

/// Holds the D3D device the capture API needs; make one per thread that captures.
pub struct Grabber {
    device: IDirect3DDevice,
}

impl Grabber {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            let mut d3d: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let dxgi: IDXGIDevice = d3d.expect("D3D11CreateDevice returned no device").cast()?;
            let device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            Ok(Self { device })
        }
    }

    /// One frame of `hwnd` at full resolution, as a CPU-side BGRA8 `SoftwareBitmap`.
    pub fn grab(&self, hwnd: HWND) -> windows::core::Result<Grab> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            item.Size()?,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let border_disable = session.SetIsBorderRequired(false).map_err(|e| e.message().to_string());
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;

        let deadline = Instant::now() + FRAME_TIMEOUT;
        let frame = loop {
            if let Ok(f) = pool.TryGetNextFrame() {
                break f;
            }
            if Instant::now() > deadline {
                let _ = session.Close();
                let _ = pool.Close();
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "no capture frame within 2 s (window minimized or cloaked?)",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let bitmap = SoftwareBitmap::CreateCopyFromSurfaceAsync(&frame.Surface()?)?.join()?;
        let _ = frame.Close();
        let _ = session.Close();
        let _ = pool.Close();
        Ok(Grab { bitmap, border_disable })
    }

    /// One frame of `hwnd` at full resolution, as packed bytes.
    pub fn grab_window(&self, hwnd: HWND) -> windows::core::Result<Frame> {
        Frame::from_bitmap(&self.grab(hwnd)?.bitmap)
    }
}

/// Tightly packed BGRA8 bytes of a bitmap (strips row padding).
fn read_bgra(bitmap: &SoftwareBitmap) -> windows::core::Result<Vec<u8>> {
    let (w, h) = (bitmap.PixelWidth()? as usize, bitmap.PixelHeight()? as usize);
    let buf = bitmap.LockBuffer(BitmapBufferAccessMode::Read)?;
    let plane = buf.GetPlaneDescription(0)?;
    let reference = buf.CreateReference()?;
    let access: IMemoryBufferByteAccess = reference.cast()?;
    let mut ptr = std::ptr::null_mut();
    let mut cap = 0u32;
    unsafe { access.GetBuffer(&mut ptr, &mut cap)? };
    let raw = unsafe { std::slice::from_raw_parts(ptr, cap as usize) };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = plane.StartIndex as usize + y * plane.Stride as usize;
        out.extend_from_slice(&raw[row..row + w * 4]);
    }
    drop(reference);
    let _ = buf.Close();
    Ok(out)
}

fn write_bgra(bitmap: &SoftwareBitmap, packed: &[u8]) -> windows::core::Result<()> {
    let (w, h) = (bitmap.PixelWidth()? as usize, bitmap.PixelHeight()? as usize);
    let buf = bitmap.LockBuffer(BitmapBufferAccessMode::Write)?;
    let plane = buf.GetPlaneDescription(0)?;
    let reference = buf.CreateReference()?;
    let access: IMemoryBufferByteAccess = reference.cast()?;
    let mut ptr = std::ptr::null_mut();
    let mut cap = 0u32;
    unsafe { access.GetBuffer(&mut ptr, &mut cap)? };
    let raw = unsafe { std::slice::from_raw_parts_mut(ptr, cap as usize) };
    for y in 0..h {
        let row = plane.StartIndex as usize + y * plane.Stride as usize;
        raw[row..row + w * 4].copy_from_slice(&packed[y * w * 4..(y + 1) * w * 4]);
    }
    drop(reference);
    let _ = buf.Close();
    Ok(())
}

/// Bilinear BGRA8 downscale over tightly packed rows.
fn downscale_bgra(src: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let (w, h, nw, nh) = (w as usize, h as usize, nw as usize, nh as usize);
    let mut out = vec![0u8; nw * nh * 4];
    let sx = w as f64 / nw as f64;
    let sy = h as f64 / nh as f64;
    for y in 0..nh {
        let fy = ((y as f64 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f64);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..nw {
            let fx = ((x as f64 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f64);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..4 {
                let p = |xx: usize, yy: usize| src[(yy * w + xx) * 4 + c] as f64;
                let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
                let bot = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
                out[(y * nw + x) * 4 + c] = (top * (1.0 - ty) + bot * ty).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose pixel (x, y) is `[x, y, 0, 255]`, so a crop can be checked by value.
    fn ramp(w: u32, h: u32) -> Frame {
        let bgra = (0..h).flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, 0, 255])).collect();
        Frame { bgra, width: w, height: h }
    }

    #[test]
    fn downscale_of_a_uniform_image_is_uniform() {
        let src: Vec<u8> = std::iter::repeat([10u8, 20, 30, 255]).take(4 * 4).flatten().collect();
        let out = downscale_bgra(&src, 4, 4, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert!(out.chunks(4).all(|px| px == [10, 20, 30, 255]));
    }

    #[test]
    fn a_crop_keeps_exactly_the_pixels_inside_it() {
        let c = ramp(8, 6).crop(Rect { x: 2, y: 1, width: 3, height: 2 }).unwrap();
        assert_eq!((c.width, c.height), (3, 2));
        assert_eq!(&c.bgra[0..4], &[2, 1, 0, 255], "top-left is (2, 1)");
        assert_eq!(&c.bgra[c.bgra.len() - 4..], &[4, 2, 0, 255], "bottom-right is (4, 2)");
    }

    #[test]
    fn a_crop_is_clipped_to_the_frame_and_an_empty_one_is_none() {
        let c = ramp(8, 6).crop(Rect { x: 6, y: 4, width: 10, height: 10 }).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(ramp(8, 6).crop(Rect { x: 8, y: 0, width: 1, height: 1 }), None);
        assert_eq!(ramp(8, 6).crop(Rect { x: 0, y: 0, width: 0, height: 3 }), None);
    }

    #[test]
    fn a_frame_round_trips_through_a_software_bitmap() {
        let f = ramp(5, 3);
        assert_eq!(Frame::from_bitmap(&f.to_bitmap().unwrap()).unwrap(), f);
    }

    /// `cargo test --lib -- --ignored grabs_a_real_window`: captures the foreground window
    /// and checks the frame is the window's visible size (DWM's extended frame bounds,
    /// which leave out the invisible resize borders), in physical pixels.
    #[test]
    #[ignore = "captures a real window"]
    fn grabs_a_real_window_at_full_size() {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
        use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        // Physical pixels, as the capture returns them (the app is per-monitor aware).
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let hwnd = unsafe { GetForegroundWindow() };
        let mut bounds = windows::Win32::Foundation::RECT::default();
        let size = std::mem::size_of_val(&bounds) as u32;
        unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut bounds as *mut _ as *mut _, size).unwrap() };
        let frame = Grabber::new().unwrap().grab_window(hwnd).unwrap();
        let (w, h) = ((bounds.right - bounds.left) as i64, (bounds.bottom - bounds.top) as i64);
        assert!((frame.width as i64 - w).abs() <= 2 && (frame.height as i64 - h).abs() <= 2, "frame {}x{} vs window {w}x{h}", frame.width, frame.height);
        assert_eq!(frame.bgra.len(), frame.width as usize * frame.height as usize * 4);
    }
}
