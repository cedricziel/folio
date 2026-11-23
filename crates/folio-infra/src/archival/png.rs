use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::png::{PngDecoder, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageError, Limits, Rgb, RgbImage};

use super::{SkipReason, MAX_FILE_BYTES};

pub(super) fn flatten_alpha(path: &Path) -> Result<(), SkipReason> {
    let file = File::open(path).map_err(|_| SkipReason::TemporaryStorage)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_FILE_BYTES);
    let decoder = PngDecoder::with_limits(BufReader::new(file), limits).map_err(png_error)?;
    if !decoder.color_type().has_alpha() {
        return Ok(());
    }

    let (width, height) = decoder.dimensions();
    let max_pixels = MAX_FILE_BYTES / u64::from(image::ColorType::Rgba16.bytes_per_pixel());
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(SkipReason::ResourceLimit);
    }

    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(png_error)?
        .into_rgba8();
    let rgb = RgbImage::from_fn(width, height, |x, y| {
        let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
        let alpha = u16::from(alpha);
        // White preserves the appearance of transparent areas on paper.
        Rgb([red, green, blue]
            .map(|channel| ((u16::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8))
    });

    let file = File::create(path).map_err(|_| SkipReason::TemporaryStorage)?;
    PngEncoder::new(file)
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(png_error)
}

fn png_error(error: ImageError) -> SkipReason {
    match error {
        ImageError::Limits(_) => SkipReason::ResourceLimit,
        ImageError::IoError(_) => SkipReason::TemporaryStorage,
        _ => SkipReason::UnsupportedMediaType,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixels_are_composited_over_white() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.png");
        image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 255, 0, 0, 128, 0, 0, 0, 255])
            .unwrap()
            .save(&path)
            .unwrap();

        flatten_alpha(&path).unwrap();

        let image = image::open(path).unwrap();
        assert_eq!(image.color(), image::ColorType::Rgb8);
        assert_eq!(image.as_bytes(), &[255, 255, 255, 255, 127, 127, 0, 0, 0]);
    }

    #[test]
    fn opaque_png_bytes_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.png");
        image::RgbImage::from_pixel(2, 2, Rgb([7, 11, 19]))
            .save(&path)
            .unwrap();
        let original = std::fs::read(&path).unwrap();

        flatten_alpha(&path).unwrap();

        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}
