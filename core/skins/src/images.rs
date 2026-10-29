//! Approved images are kept at most 1600 px on their long side, on both platforms, so a skin's
//! sync cost stays in the low single-digit MB (docs/superpowers/specs/2026-10-22-skins-windows-and-sync-design.md).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, ImageResult};

/// The long-side limit for an approved image.
pub const MAX_SIDE: u32 = 1600;

/// `bytes` with its long side at most `max_side`, in the format it came in (JPEG at quality 85). An
/// image that already fits comes back unchanged, so an animated GIF that fits keeps its frames.
pub fn downscale(bytes: &[u8], max_side: u32) -> ImageResult<Vec<u8>> {
    let format = image::guess_format(bytes)?;
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;
    if width.max(height) <= max_side {
        return Ok(bytes.to_vec());
    }
    let small = image::load_from_memory_with_format(bytes, format)?.resize(
        max_side,
        max_side,
        FilterType::Lanczos3,
    );
    let mut out = Vec::new();
    match format {
        ImageFormat::Jpeg => small
            .to_rgb8()
            .write_with_encoder(JpegEncoder::new_with_quality(&mut out, 85))?,
        other => small.write_to(&mut Cursor::new(&mut out), other)?,
    }
    Ok(out)
}

/// The side of the nearest-neighbour sample `scrim::extremes` reads.
pub const SAMPLE_SIDE: u32 = 64;

/// The image's `SAMPLE_SIDE` x `SAMPLE_SIDE` nearest-neighbour sample (straight RGBA), the pixels
/// `scrim::extremes` would pick from the whole image. `extremes` of the sample equals `extremes` of the
/// image, so a caller can keep the sample (16 KB) instead of the decoded image.
pub fn scrim_sample(bytes: &[u8]) -> ImageResult<Vec<u8>> {
    let rgba = image::load_from_memory(bytes)?.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut out = Vec::with_capacity((SAMPLE_SIDE * SAMPLE_SIDE * 4) as usize);
    for y in 0..SAMPLE_SIDE {
        for x in 0..SAMPLE_SIDE {
            let pick = |i: u32, side: u32| {
                ((u64::from(i) * 2 + 1) * u64::from(side) / (u64::from(SAMPLE_SIDE) * 2)) as u32
            };
            out.extend_from_slice(&rgba.get_pixel(pick(x, width), pick(y, height)).0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([40, 90, 160]),
        ));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }

    fn size(bytes: &[u8]) -> (u32, u32) {
        let format = image::guess_format(bytes).unwrap();
        ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .unwrap()
    }

    #[test]
    fn caps_the_long_side_and_keeps_the_format() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::WebP,
        ] {
            // Small sizes keep the debug build quick; the rule is the same at 1600.
            let out = downscale(&encoded(320, 100, format), 160).unwrap();
            assert_eq!(image::guess_format(&out).unwrap(), format);
            assert_eq!(size(&out), (160, 50), "{format:?}");
        }
        let tall = downscale(&encoded(900, 2400, ImageFormat::Png), MAX_SIDE).unwrap();
        assert_eq!(size(&tall), (600, 1600));
    }

    #[test]
    fn an_image_that_fits_is_unchanged() {
        let small = encoded(160, 120, ImageFormat::Png);
        assert_eq!(downscale(&small, 160).unwrap(), small);
    }

    #[test]
    fn the_sample_has_the_extremes_of_the_whole_image() {
        // A gradient, so which pixels are picked matters.
        let img = image::RgbaImage::from_fn(300, 170, |x, y| {
            image::Rgba([(x % 256) as u8, (y * 3 % 256) as u8, ((x + y) % 256) as u8, 255])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let sample = scrim_sample(&bytes).unwrap();
        for page in [0xFFFFFF, 0x101010] {
            assert_eq!(
                crate::scrim::extremes(&sample, SAMPLE_SIDE, SAMPLE_SIDE, page),
                crate::scrim::extremes(img.as_raw(), 300, 170, page)
            );
        }
    }

    #[test]
    fn refuses_what_is_not_an_image() {
        assert!(downscale(b"not an image at all", MAX_SIDE).is_err());
    }
}
