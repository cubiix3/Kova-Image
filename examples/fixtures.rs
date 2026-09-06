//! Deterministic, original test images; no external downloads or private photos.
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{fs::File, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Usage: fixtures <output directory>")?,
    );
    std::fs::create_dir_all(&folder)?;
    let pixels = RgbImage::from_fn(1920, 1080, |x, y| {
        let checker = ((x / 80 + y / 80) % 2) as u8;
        Rgb([
            (x * 255 / 1919) as u8,
            (y * 255 / 1079) as u8,
            70 + checker * 80,
        ])
    });
    let image = DynamicImage::ImageRgb8(pixels);
    for (name, format) in [
        ("image1.jpg", ImageFormat::Jpeg),
        ("image2.png", ImageFormat::Png),
        ("image10.webp", ImageFormat::WebP),
    ] {
        image.save_with_format(folder.join(name), format)?;
    }
    let large = RgbImage::from_fn(6000, 4000, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });
    large.save(folder.join("image20-large.png"))?;
    let mut gif = image::codecs::gif::GifEncoder::new(File::create(folder.join("image3.gif"))?);
    gif.set_repeat(image::codecs::gif::Repeat::Infinite)?;
    for i in 0..12 {
        let buffer = image::RgbaImage::from_fn(400, 240, |x, y| {
            image::Rgba([((x + i * 24) % 256) as u8, y as u8, 160, 255])
        });
        gif.encode_frame(image::Frame::from_parts(
            buffer,
            0,
            0,
            image::Delay::from_numer_denom_ms(40 + i * 5, 1),
        ))?;
    }
    Ok(())
}
