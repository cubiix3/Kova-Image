//! Formats that are decoded by Kova Image's own codecs rather than the `image`
//! crate. The fixtures in `tests/fixtures` come from `scripts/image-fixtures.py`
//! and all show the same synthetic 32 x 24 picture.
use kova_image::{
    decoder::{self, Decoded, Format, Target},
    error::Error,
    security::Generation,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const W: u32 = 32;
const H: u32 = 24;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kova-image-formats-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn load(path: &Path) -> Result<Decoded, Error> {
    decoder::load(path, &Generation::default().next())
}
fn load_fit(path: &Path, max: u32) -> Result<Decoded, Error> {
    decoder::load_target(
        path,
        &Generation::default().next(),
        Target {
            max_width: max,
            max_height: max,
        },
        &mut |_| {},
    )
}

/// The picture every fixture shows.
fn expected(x: u32, y: u32, alpha: bool) -> [u8; 4] {
    let a = if alpha {
        (x * 255 / (W - 1)) as u8
    } else {
        255
    };
    if x < 6 && y < 6 {
        [255, 255, 255, a]
    } else if x >= W - 6 && y >= H - 6 {
        [0, 0, 0, a]
    } else {
        [(x * 8) as u8, (y * 10) as u8, 96, a]
    }
}
/// Largest per-channel difference from the expected picture.
fn deviation(image: &Decoded, alpha: bool) -> u32 {
    assert_eq!((image.width, image.height), (W, H));
    assert_eq!(image.frames.len(), 1);
    let mut worst = 0;
    for y in 0..H {
        for x in 0..W {
            let at = ((y * W + x) * 4) as usize;
            let pixel = &image.frames[0].rgba[at..at + 4];
            let want = expected(x, y, alpha);
            for c in 0..4 {
                worst = worst.max(u32::from(pixel[c].abs_diff(want[c])));
            }
        }
    }
    worst
}

#[test]
fn jpeg_xl_lossless_is_exact_with_and_without_alpha() {
    let image = load(&fixture("rgb.jxl")).unwrap();
    assert_eq!(image.format, Format::JpegXl);
    assert_eq!(deviation(&image, false), 0);
    assert!(!image.alpha);
    let image = load(&fixture("rgba.jxl")).unwrap();
    assert_eq!(deviation(&image, true), 0);
    assert!(image.alpha);
}
#[test]
fn jpeg_xl_lossy_stays_close() {
    let image = load(&fixture("lossy.jxl")).unwrap();
    assert_eq!(image.format, Format::JpegXl);
    assert!(
        deviation(&image, false) <= 24,
        "{}",
        deviation(&image, false)
    );
}
#[test]
fn jpeg_xl_is_fitted_to_the_view() {
    let image = load_fit(&fixture("rgb.jxl"), 16).unwrap();
    assert_eq!((image.width, image.height), (16, 12));
    assert_eq!((image.source_width, image.source_height), (W, H));
}
#[test]
fn jpeg_xl_damage_is_an_error_not_a_panic() {
    let temp = Temp::new();
    let good = std::fs::read(fixture("lossy.jxl")).unwrap();
    let ticket = Generation::default().next();
    // Every truncation must fail cleanly.
    for length in [2, 12, 40, good.len() / 2, good.len() - 1] {
        let path = temp.write("cut.jxl", &good[..length]);
        assert!(decoder::load(&path, &ticket).is_err(), "{length}");
    }
    // Flipping bytes may or may not be noticed, but must never abort.
    for at in (0..good.len()).step_by(7) {
        let mut bad = good.clone();
        bad[at] ^= 0xff;
        let _ = decoder::load(&temp.write("flip.jxl", &bad), &ticket);
    }
}

fn svg(body: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' \
         width='32' height='24' viewBox='0 0 32 24'>{body}</svg>"
    )
}
fn pixel(image: &Decoded, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    image.frames[0].rgba[at..at + 4].try_into().unwrap()
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn red_png() -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        4,
        4,
        image::Rgba([255, 0, 0, 255]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    png.into_inner()
}

#[test]
fn svg_renders_to_the_size_the_view_asks_for() {
    let temp = Temp::new();
    let path = temp.write(
        "picture.svg",
        svg("<rect width='16' height='24' fill='#f00'/><rect x='16' width='16' height='24' fill='#00f' fill-opacity='.5'/>").as_bytes(),
    );
    // A vector is enlarged to fill the view and stays sharp; 100% is its own size.
    let image = load_fit(&path, 128).unwrap();
    assert_eq!(image.format, Format::Svg);
    assert_eq!((image.source_width, image.source_height), (32, 24));
    assert_eq!((image.width, image.height), (128, 96));
    assert_eq!(pixel(&image, 10, 10), [255, 0, 0, 255]);
    // Translucent colour is returned as straight alpha, not premultiplied.
    let half = pixel(&image, 100, 10);
    assert!((125..=130).contains(&half[3]), "{half:?}");
    assert!(half[2] > 250 && half[0] < 5 && half[1] < 5, "{half:?}");
    assert!(image.serves(Target {
        max_width: 128,
        max_height: 96
    }));
    // Without a view size it is still rendered large enough to zoom into.
    let image = load(&path).unwrap();
    assert_eq!((image.source_width, image.source_height), (32, 24));
    assert!(image.width >= 2048, "{}", image.width);
    // An empty drawing is transparent, so the checker grid can show through.
    let empty = load(&temp.write("empty.svg", svg("").as_bytes())).unwrap();
    assert!(empty.alpha);
    assert_eq!(pixel(&empty, 5, 5)[3], 0);
}
#[test]
fn svg_never_reads_files_or_addresses() {
    let temp = Temp::new();
    let png = temp.write("red.png", &red_png());
    let escaped = png.to_string_lossy().replace('\\', "/");
    for href in [
        "red.png".to_string(),
        escaped.clone(),
        format!("file:///{escaped}"),
        "http://127.0.0.1:9/red.png".to_string(),
    ] {
        let path = temp.write(
            "outside.svg",
            svg(&format!(
                "<image width='32' height='24' xlink:href='{href}'/>"
            ))
            .as_bytes(),
        );
        let image = load(&path).unwrap();
        assert_eq!(
            pixel(&image, image.width / 2, image.height / 2)[3],
            0,
            "{href}"
        );
    }
    // A picture inside the document is drawn.
    let inline = svg(&format!(
        "<image width='32' height='24' xlink:href='data:image/png;base64,{}'/>",
        base64(&red_png())
    ));
    let image = load(&temp.write("inline.svg", inline.as_bytes())).unwrap();
    assert_eq!(
        pixel(&image, image.width / 2, image.height / 2),
        [255, 0, 0, 255]
    );
}
#[test]
fn svg_limits_hold_for_hostile_documents() {
    let temp = Temp::new();
    let ticket = Generation::default().next();
    // An absurd size is fitted to the pixel limit instead of allocated.
    let huge = "<svg xmlns='http://www.w3.org/2000/svg' width='1000000' height='1000000'/>";
    let image = load(&temp.write("huge.svg", huge.as_bytes())).unwrap();
    assert!(u64::from(image.width) * u64::from(image.height) <= kova_image::security::MAX_PIXELS);
    assert!(image.source_width <= kova_image::security::MAX_DIMENSION);
    // A tiny archive that unpacks to far more than the limit.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &vec![b' '; 20 * 1024 * 1024]).unwrap();
    let bomb = gz.finish().unwrap();
    assert!(bomb.len() < 100 * 1024);
    assert!(decoder::load(&temp.write("bomb.svgz", &bomb), &ticket).is_err());
    // Nested entity definitions must not expand without bound.
    let mut laughs = String::from("<?xml version='1.0'?><!DOCTYPE svg [<!ENTITY a0 'lol'>");
    for level in 1..10 {
        let previous = level - 1;
        laughs += &format!(
            "<!ENTITY a{level} '{}'>",
            format!("&a{previous};").repeat(10)
        );
    }
    laughs +=
        "]><svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><text>&a9;</text></svg>";
    let started = std::time::Instant::now();
    let _ = decoder::load(&temp.write("laughs.svg", laughs.as_bytes()), &ticket);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    assert!(
        decoder::load(
            &temp.write("cut.svg", b"<svg xmlns='http://www.w3.org/2000/svg'><rect"),
            &ticket
        )
        .is_err()
    );
}

/// Mean absolute difference over all channels, for lossy codecs with chroma
/// subsampling, whose worst case at a sharp edge is large but whose average is not.
fn mean_deviation(image: &Decoded, alpha: bool) -> f64 {
    let mut sum = 0u64;
    for y in 0..H {
        for x in 0..W {
            let at = ((y * W + x) * 4) as usize;
            let want = expected(x, y, alpha);
            for (got, want) in image.frames[0].rgba[at..at + 4].iter().zip(want) {
                sum += u64::from(got.abs_diff(want));
            }
        }
    }
    sum as f64 / f64::from(W * H * 4)
}

#[test]
fn avif_lossless_gbr_is_exact() {
    // 4:4:4 in the identity matrix: the planes are green, blue and red.
    let image = load(&fixture("rgb444.avif")).unwrap();
    assert_eq!(image.format, Format::Avif);
    assert_eq!(deviation(&image, false), 0);
    assert!(!image.alpha);
}
#[test]
fn avif_lossy_420_limited_range_stays_close() {
    let image = load(&fixture("rgb420.avif")).unwrap();
    assert_eq!(image.format, Format::Avif);
    assert_eq!((image.width, image.height), (W, H));
    let mean = mean_deviation(&image, false);
    assert!(mean < 4.0, "mean {mean}");
    // The flat areas keep their colour closely: the white and black corner squares.
    assert!(pixel(&image, 1, 1).iter().take(3).all(|c| *c > 240));
    assert!(pixel(&image, W - 2, H - 2).iter().take(3).all(|c| *c < 15));
}
#[test]
fn avif_alpha_comes_from_the_second_picture() {
    let image = load(&fixture("rgba.avif")).unwrap();
    assert!(image.alpha);
    let mean = mean_deviation(&image, true);
    assert!(mean < 4.0, "mean {mean}");
    // Fully transparent on the left edge, nearly opaque on the right.
    assert!(pixel(&image, 0, 12)[3] < 8);
    assert!(pixel(&image, W - 1, 12)[3] > 247);
}
#[test]
fn avif_damage_is_an_error_not_a_panic() {
    let temp = Temp::new();
    let ticket = Generation::default().next();
    let good = std::fs::read(fixture("rgba.avif")).unwrap();
    for length in [4, 12, 40, 120, 200, good.len() - 20, good.len() - 1] {
        let path = temp.write("cut.avif", &good[..length]);
        assert!(decoder::load(&path, &ticket).is_err(), "{length}");
    }
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] ^= 0xa5;
        let _ = decoder::load(&temp.write("flip.avif", &bad), &ticket);
    }
}

/// A TIFF-style RAW file: Make, Model and Orientation in the first directory,
/// sensor-like filler, then the JPEG preview the camera stored.
fn raw_file(orientation: u16, preview: &[u8]) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    // Make at offset 50 (4 bytes), Model at 54, Orientation inline.
    for (tag, kind, count, value) in [
        (0x010fu16, 2u16, 5u32, 50u32),
        (0x0110, 2, 5, 56),
        (0x0112, 3, 1, u32::from(orientation)),
    ] {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&kind.to_le_bytes());
        tiff.extend_from_slice(&count.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
    }
    tiff.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(tiff.len(), 50);
    tiff.extend_from_slice(b"Kova\0\0Body\0");
    tiff.extend_from_slice(&[0x5a; 5000]);
    tiff.extend_from_slice(preview);
    tiff.extend_from_slice(&[0x11; 300]);
    tiff
}
fn jpeg_of(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 90])
    }))
    .write_to(&mut bytes, image::ImageFormat::Jpeg)
    .unwrap();
    bytes.into_inner()
}

#[test]
fn raw_shows_the_embedded_preview_with_the_camera_orientation() {
    let temp = Temp::new();
    let jpeg = jpeg_of(48, 32);
    // The extension is what tells a RAW file from a TIFF.
    let upright = load(&temp.write("shot.arw", &raw_file(1, &jpeg))).unwrap();
    assert_eq!(upright.format, Format::Raw);
    assert_eq!((upright.width, upright.height), (48, 32));
    assert_eq!(upright.photo.camera.as_deref(), Some("Kova Body"));
    // Orientation 6 means the camera was held upright: turn the preview.
    let turned = load(&temp.write("shot.nef", &raw_file(6, &jpeg))).unwrap();
    assert_eq!((turned.width, turned.height), (32, 48));
    // Without the extension it is only a TIFF, whose first image is the sensor stub.
    let as_tiff = temp.write("shot.dat", &raw_file(1, &jpeg));
    assert!(load(&as_tiff).is_err());
    // A RAW file without any JPEG is reported, not guessed at.
    let mut bare = raw_file(1, &jpeg);
    bare.truncate(5000);
    assert!(load(&temp.write("bare.cr2", &bare)).is_err());
}

#[test]
fn heic_lossy_pictures_stay_close() {
    for name in ["rgb420.heic", "rgb10.heic"] {
        let image = load(&fixture(name)).unwrap();
        assert_eq!(image.format, Format::Heic, "{name}");
        assert_eq!((image.width, image.height), (W, H), "{name}");
        let mean = mean_deviation(&image, false);
        assert!(mean < 4.0, "{name}: mean {mean}");
        assert!(
            pixel(&image, 1, 1).iter().take(3).all(|c| *c > 235),
            "{name}"
        );
        assert!(
            pixel(&image, W - 2, H - 2).iter().take(3).all(|c| *c < 20),
            "{name}"
        );
        assert!(!image.alpha, "{name}");
    }
}
#[test]
fn heic_alpha_comes_from_the_auxiliary_picture() {
    let image = load(&fixture("rgba.heic")).unwrap();
    assert!(image.alpha);
    assert!(mean_deviation(&image, true) < 4.0);
    assert!(pixel(&image, 0, 12)[3] < 8);
    assert!(pixel(&image, W - 1, 12)[3] > 247);
}
#[test]
fn heic_grid_tiles_are_put_together() {
    let image = load(&fixture("grid.heic")).unwrap();
    assert_eq!((image.width, image.height), (2 * W, 2 * H));
    // Each tile shows the picture at half the scale of the whole.
    let mut total = 0u64;
    for y in 0..2 * H {
        for x in 0..2 * W {
            let want = expected(x / 2, y / 2, false);
            let got = pixel(&image, x, y);
            total += (0..3)
                .map(|c| u64::from(got[c].abs_diff(want[c])))
                .sum::<u64>();
        }
    }
    let mean = total as f64 / f64::from(2 * W * 2 * H * 3);
    assert!(mean < 5.0, "mean {mean}");
}
#[test]
fn heic_rotation_property_turns_the_picture_back() {
    let image = load(&fixture("turned.heic")).unwrap();
    assert_eq!((image.width, image.height), (W, H));
    assert!(mean_deviation(&image, false) < 4.0);
    assert!(pixel(&image, 1, 1).iter().take(3).all(|c| *c > 235));
}
#[test]
fn heic_damage_is_an_error_not_a_panic() {
    let temp = Temp::new();
    let ticket = Generation::default().next();
    let good = std::fs::read(fixture("rgba.heic")).unwrap();
    for length in [4, 20, 60, 200, good.len() / 2, good.len() - 1] {
        let path = temp.write("cut.heic", &good[..length]);
        assert!(decoder::load(&path, &ticket).is_err(), "{length}");
    }
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] ^= 0x5a;
        let _ = decoder::load(&temp.write("flip.heic", &bad), &ticket);
    }
}

/// A DDS file for the container tests: header, optional DX10 header, pixel data.
struct Dds {
    width: u32,
    height: u32,
    flags: u32,
    four_cc: [u8; 4],
    bits: u32,
    masks: [u32; 4],
    dxgi: Option<u32>,
}

impl Dds {
    fn bytes(&self, data: &[u8]) -> Vec<u8> {
        let mut out = b"DDS ".to_vec();
        let mut header = [0u32; 31];
        header[0] = 124;
        header[1] = 0x1007; // caps, height, width, pixel format
        header[2] = self.height;
        header[3] = self.width;
        header[18] = 32; // size of the pixel format
        header[19] = self.flags;
        header[20] = u32::from_le_bytes(self.four_cc);
        header[21] = self.bits;
        header[22..26].copy_from_slice(&self.masks);
        header[26] = 0x1000;
        for word in header {
            out.extend_from_slice(&word.to_le_bytes());
        }
        if let Some(format) = self.dxgi {
            for word in [format, 3, 0, 1, 0] {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        out.extend_from_slice(data);
        out
    }
}

const RGB: u32 = 0x40;
const ARGB: u32 = 0x41;
const LUMINANCE: u32 = 0x2_0000;
const ALPHA_ONLY: u32 = 0x2;
const FOUR_CC: u32 = 0x4;

fn packed(width: u32, height: u32, flags: u32, bits: u32, masks: [u32; 4]) -> Dds {
    Dds {
        width,
        height,
        flags,
        four_cc: [0; 4],
        bits,
        masks,
        dxgi: None,
    }
}

fn dx10(width: u32, height: u32, format: u32) -> Dds {
    Dds {
        width,
        height,
        flags: FOUR_CC,
        four_cc: *b"DX10",
        bits: 0,
        masks: [0; 4],
        dxgi: Some(format),
    }
}

fn dds_load(file: &[u8]) -> Result<Decoded, Error> {
    let temp = Temp::new();
    load(&temp.write("texture.dds", file))
}

#[test]
fn dds_from_pillows_encoder_matches_the_picture() {
    // Block compressed with and without alpha, and uncompressed.
    let image = load(&fixture("rgba.dds")).unwrap();
    assert_eq!(
        (image.format, image.width, image.height),
        (Format::Dds, W, H)
    );
    assert!(mean_deviation(&image, true) < 8.0);
    let image = load(&fixture("rgb.dds")).unwrap();
    assert!(mean_deviation(&image, false) < 8.0);
    let image = load(&fixture("rgba-raw.dds")).unwrap();
    assert_eq!(deviation(&image, true), 0);
    assert!(image.alpha);
}

#[test]
fn dds_uncompressed_layouts_follow_their_masks() {
    let argb = [0xff_0000, 0xff00, 0xff, 0xff00_0000];
    // Stored as B, G, R, A.
    let file = packed(2, 1, ARGB, 32, argb).bytes(&[10, 20, 30, 40, 50, 60, 70, 80]);
    let image = dds_load(&file).unwrap();
    assert_eq!(image.frames[0].rgba, [30, 20, 10, 40, 70, 60, 50, 80]);
    assert!(image.alpha);
    // X8R8G8B8: no alpha mask, so opaque.
    let file = packed(1, 1, RGB, 32, [0xff_0000, 0xff00, 0xff, 0]).bytes(&[1, 2, 3, 0]);
    let image = dds_load(&file).unwrap();
    assert_eq!(image.frames[0].rgba, [3, 2, 1, 255]);
    assert!(!image.alpha);
    // 24 bit
    let file = packed(1, 1, RGB, 24, [0xff_0000, 0xff00, 0xff, 0]).bytes(&[0, 0, 255]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [255, 0, 0, 255]);
    // R5G6B5
    let file = packed(2, 1, RGB, 16, [0xf800, 0x07e0, 0x001f, 0]).bytes(&[0x00, 0xf8, 0xe0, 0x07]);
    assert_eq!(
        dds_load(&file).unwrap().frames[0].rgba,
        [255, 0, 0, 255, 0, 255, 0, 255]
    );
    // Luminance, luminance with alpha, and alpha alone (shown as grey).
    let file = packed(1, 1, LUMINANCE, 8, [0xff, 0, 0, 0]).bytes(&[77]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [77, 77, 77, 255]);
    let file = packed(1, 1, LUMINANCE | 1, 16, [0xff, 0, 0, 0xff00]).bytes(&[77, 200]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [77, 77, 77, 200]);
    let file = packed(1, 1, ALPHA_ONLY, 8, [0, 0, 0, 0xff]).bytes(&[99]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [99, 99, 99, 255]);
}

#[test]
fn dds_dx10_header_names_the_format() {
    // R8G8B8A8_UNORM
    let file = dx10(1, 1, 28).bytes(&[1, 2, 3, 4]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [1, 2, 3, 4]);
    // B8G8R8A8_UNORM_SRGB
    let file = dx10(1, 1, 91).bytes(&[1, 2, 3, 4]);
    assert_eq!(dds_load(&file).unwrap().frames[0].rgba, [3, 2, 1, 4]);
    // BC6H holds floating point colour and is not supported.
    let file = dx10(4, 4, 95).bytes(&[0; 16]);
    assert!(matches!(dds_load(&file), Err(Error::Unsupported)));
}

const BC7_BLOCKS: &[u8] = include_bytes!("fixtures/bc7.blocks");
const BC7_PIXELS: &[u8] = include_bytes!("fixtures/bc7.rgba");

#[test]
fn dds_bc7_blocks_are_placed_and_cropped_correctly() {
    // A row of blocks: each lands in its own four columns.
    let blocks = BC7_BLOCKS.len() / 16;
    let file = dx10(4 * blocks as u32, 4, 98).bytes(BC7_BLOCKS);
    let image = dds_load(&file).unwrap();
    assert_eq!((image.width, image.height), (4 * blocks as u32, 4));
    let rgba = &image.frames[0].rgba;
    for block in 0..blocks {
        for y in 0..4 {
            for x in 0..4 {
                let at = (y * 4 * blocks + block * 4 + x) * 4;
                let want = block * 64 + (y * 4 + x) * 4;
                assert_eq!(
                    rgba[at..at + 4],
                    BC7_PIXELS[want..want + 4],
                    "{block} {x},{y}"
                );
            }
        }
    }
    // 6 x 5 pixels need 2 x 2 blocks; the rest of them is cut off.
    let file = dx10(6, 5, 98).bytes(&BC7_BLOCKS[..64]);
    let image = dds_load(&file).unwrap();
    assert_eq!((image.width, image.height), (6, 5));
    for y in 0..5usize {
        for x in 0..6usize {
            let block = x / 4 + 2 * (y / 4);
            let want = block * 64 + (y % 4 * 4 + x % 4) * 4;
            assert_eq!(
                image.frames[0].rgba[(y * 6 + x) * 4..][..4],
                BC7_PIXELS[want..want + 4]
            );
        }
    }
}

#[test]
fn dds_damage_is_an_error_not_a_panic() {
    let whole = dx10(8, 8, 98).bytes(&BC7_BLOCKS[..64]);
    // Cut off anywhere: in the header, the DX10 header or the pixel data.
    for length in [0, 3, 60, 127, 130, 147, 148, 150, whole.len() - 1] {
        let result = std::panic::catch_unwind(|| dds_load(&whole[..length]));
        assert!(matches!(result, Ok(Err(_))), "length {length}");
    }
    // Absurd sizes are refused before anything is allocated.
    let file = packed(
        60_000,
        60_000,
        ARGB,
        32,
        [0xff_0000, 0xff00, 0xff, 0xff00_0000],
    )
    .bytes(&[]);
    assert!(dds_load(&file).is_err());
    // Not a pixel format Kova Image reads: a YUV layout.
    let file = packed(2, 2, 0x200, 16, [0; 4]).bytes(&[0; 8]);
    assert!(matches!(dds_load(&file), Err(Error::Unsupported)));
}
