//! OCR fallback without persisting a screenshot — docs/12 §4.2.
//!
//! One frame of one window via `Windows.Graphics.Capture` → a `SoftwareBitmap` in
//! memory → `Windows.Media.Ocr`. The frame is never encoded or written anywhere; it is
//! dropped when [`Ocr::recognize_window`] returns (FR-CAP-3 by construction).
//!
//! `OcrEngine` rejects images larger than `OcrEngine::MaxImageDimension` on either side.
//! That limit is read at runtime, not assumed: it was 2600 px on early Windows 10
//! builds and is 10000 px on the Windows 11 build M3.1 ran on (26340), where no window
//! needed downscaling. Frames that exceed it are downscaled in memory first.

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureItem,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;

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
    device: IDirect3DDevice,
    engine: OcrEngine,
    max_dim: u32,
}

const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

impl Ocr {
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
            let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
            let max_dim = OcrEngine::MaxImageDimension()?;
            Ok(Self { device, engine, max_dim })
        }
    }

    pub fn recognize_window(&self, hwnd: HWND) -> windows::core::Result<OcrOutput> {
        let t0 = Instant::now();
        let (bitmap, border_disable) = self.grab_frame(hwnd)?;
        let (frame_width, frame_height) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let bitmap = self.fit_for_ocr(bitmap)?;
        let (ocr_width, ocr_height) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let capture_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        let result = self.engine.RecognizeAsync(&bitmap)?.join()?;
        let text = result.Text()?.to_string();
        let lines = result.Lines()?.Size()?;
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
            border_disable,
            text,
        })
    }

    /// One frame of `hwnd` as a CPU-side BGRA8 `SoftwareBitmap`.
    fn grab_frame(&self, hwnd: HWND) -> windows::core::Result<(SoftwareBitmap, Result<(), String>)> {
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
        Ok((bitmap, border_disable))
    }

    /// Returns `bitmap` unchanged if it fits `MaxImageDimension`, else a bilinear
    /// downscale that does. All in memory.
    fn fit_for_ocr(&self, bitmap: SoftwareBitmap) -> windows::core::Result<SoftwareBitmap> {
        let (w, h) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let Some((nw, nh)) = fit_within(w, h, self.max_dim) else {
            return Ok(bitmap);
        };
        let src = read_bgra(&bitmap)?;
        let dst = downscale_bgra(&src, w, h, nw, nh);
        let out = SoftwareBitmap::CreateWithAlpha(BitmapPixelFormat::Bgra8, nw as i32, nh as i32, BitmapAlphaMode::Premultiplied)?;
        write_bgra(&out, &dst)?;
        Ok(out)
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

/// The largest size with `w:h`'s aspect ratio whose sides are both `<= max_dim`, or
/// `None` if `w x h` already fits.
fn fit_within(w: u32, h: u32, max_dim: u32) -> Option<(u32, u32)> {
    if w <= max_dim && h <= max_dim {
        return None;
    }
    let scale = max_dim as f64 / w.max(h) as f64;
    Some((((w as f64 * scale).floor() as u32).max(1), ((h as f64 * scale).floor() as u32).max(1)))
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

    #[test]
    fn fit_within_keeps_small_frames_and_scales_the_long_side() {
        assert_eq!(fit_within(1920, 1080, 2600), None);
        assert_eq!(fit_within(2880, 1800, 2600), Some((2600, 1625)));
        assert_eq!(fit_within(1200, 3000, 2600), Some((1040, 2600)));
    }

    #[test]
    fn downscale_of_a_uniform_image_is_uniform() {
        let src: Vec<u8> = std::iter::repeat([10u8, 20, 30, 255]).take(4 * 4).flatten().collect();
        let out = downscale_bgra(&src, 4, 4, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert!(out.chunks(4).all(|px| px == [10, 20, 30, 255]));
    }
}
