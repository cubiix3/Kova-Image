//! Deterministic damage tests. Valid files of every supported format are
//! truncated, bit-flipped and spliced with a seeded generator, then handed to
//! the production loaders. The outcome may be an image or an error, but never a
//! contained decoder panic, a hang or a crash. This is not a substitute for
//! coverage-guided fuzzing; it is a cheap regression net that runs in CI.
use image::{DynamicImage, ImageEncoder, ImageFormat, RgbaImage};
use kova_image::{
    decoder::{self, Target},
    error::Error,
    security::Generation,
    settings::Settings,
};
use std::{io::Cursor, path::PathBuf, time::Instant};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kova-damage-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn sample(format: ImageFormat) -> Vec<u8> {
    let image = RgbaImage::from_fn(24, 16, |x, y| {
        image::Rgba([x as u8 * 9, y as u8 * 14, 90, 255])
    });
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image).into_rgb8())
    } else {
        DynamicImage::ImageRgba8(image)
    };
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}
fn jpeg_with_exif() -> Vec<u8> {
    let raw: Vec<u8> = (0..24 * 16 * 3).map(|i| (i % 251) as u8).collect();
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
    let mut exif = b"MM\0*\0\0\0\x08\0\x01".to_vec();
    exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0]);
    encoder.set_exif_metadata(exif).unwrap();
    encoder
        .write_image(&raw, 24, 16, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}
fn gif() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 4, 4, &[255, 0, 0, 0, 0, 255]).unwrap();
        encoder.set_repeat(gif::Repeat::Finite(2)).unwrap();
        for index in [0u8, 1, 0] {
            let frame = gif::Frame {
                width: 4,
                height: 4,
                delay: 5,
                buffer: std::borrow::Cow::Owned(vec![index; 16]),
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    bytes
}
fn apng() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 4, 4);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(2, 0).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200; 4 * 4 * 4]).unwrap();
        writer.write_image_data(&[50; 4 * 4 * 4]).unwrap();
    }
    bytes
}
fn damage(original: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = original.to_vec();
    match rng.below(4) {
        0 => bytes.truncate(rng.below(bytes.len())),
        1 => {
            for _ in 0..=rng.below(8) {
                let at = rng.below(bytes.len());
                bytes[at] ^= 1 << rng.below(8);
            }
        }
        2 => {
            // Overwrite a short run with random bytes, often inside a header.
            let start = rng.below(bytes.len().min(96));
            for byte in bytes.iter_mut().skip(start).take(1 + rng.below(6)) {
                *byte = rng.next() as u8;
            }
        }
        _ => {
            let at = rng.below(bytes.len());
            let junk: Vec<u8> = (0..1 + rng.below(32)).map(|_| rng.next() as u8).collect();
            bytes.splice(at..at, junk);
        }
    }
    bytes
}
fn panicked(result: &Result<decoder::Decoded, Error>) -> bool {
    matches!(result, Err(Error::Corrupted(text)) if text == "decoder panicked")
}

#[test]
fn damaged_images_never_panic_or_hang() {
    let temp = Temp::new("images");
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        ("jpeg", sample(ImageFormat::Jpeg)),
        ("jpeg-exif", jpeg_with_exif()),
        ("png", sample(ImageFormat::Png)),
        ("webp", sample(ImageFormat::WebP)),
        ("bmp", sample(ImageFormat::Bmp)),
        ("tiff", sample(ImageFormat::Tiff)),
        ("ico", sample(ImageFormat::Ico)),
        ("gif", gif()),
        ("apng", apng()),
    ];
    let targets = [
        Target::full(),
        Target {
            max_width: 8,
            max_height: 8,
        },
    ];
    let mut panics = Vec::new();
    let mut decoded = 0;
    let start = Instant::now();
    for (name, original) in &fixtures {
        // Undamaged fixtures must load, or the mutations prove nothing.
        let path = temp.0.join(format!("{name}.bin"));
        std::fs::write(&path, original).unwrap();
        let ticket = Generation::default().next();
        assert!(decoder::load(&path, &ticket).is_ok(), "{name} fixture");
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (name.len() as u64 * 7919));
        for round in 0..150 {
            let bytes = damage(original, &mut rng);
            std::fs::write(&path, &bytes).unwrap();
            for target in targets {
                let ticket = Generation::default().next();
                let result = decoder::load_target(&path, &ticket, target, &mut |_| {});
                if panicked(&result) {
                    panics.push(format!("{name} round {round} target {target:?}"));
                }
                if result.is_ok() {
                    decoded += 1;
                }
            }
        }
    }
    eprintln!(
        "{decoded} damaged files still decoded; {:?} in total",
        start.elapsed()
    );
    assert!(
        panics.is_empty(),
        "decoder panics (contained, but bugs): {panics:#?}"
    );
    assert!(
        start.elapsed().as_secs() < 60,
        "damaged input took too long"
    );
}

#[cfg(windows)]
#[test]
fn damaged_movies_are_rejected_or_admitted_without_panics() {
    fn atom(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = ((8 + data.len()) as u32).to_be_bytes().to_vec();
        out.extend(name);
        out.extend(data);
        out
    }
    let mut reference = vec![0, 0, 0, 0, 0, 0, 0, 1];
    reference.extend(atom(b"url ", &[0, 0, 0, 1]));
    let mut movie = atom(b"ftyp", b"isom\0\0\0\0isom");
    let mut tree = atom(b"dref", &reference);
    for name in [b"dinf", b"minf", b"mdia", b"trak", b"moov"] {
        tree = atom(name, &tree);
    }
    movie.extend(tree);
    movie.extend(atom(b"mdat", &[0; 64]));
    let temp = Temp::new("movies");
    let path = temp.0.join("clip.mp4");
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for _ in 0..1500 {
        std::fs::write(&path, damage(&movie, &mut rng)).unwrap();
        let ticket = Generation::default().next();
        // Any outcome is fine as long as the box walker returns.
        let _ = kova_image::media::open_video(&path, &ticket);
    }
}

#[test]
fn settings_text_never_panics_and_keeps_defaults_for_garbage() {
    let mut rng = Rng(42);
    for _ in 0..2000 {
        let bytes: Vec<u8> = (0..rng.below(300)).map(|_| rng.next() as u8).collect();
        let text = String::from_utf8_lossy(&bytes);
        let _ = Settings::parse(&text);
    }
    let defaults = Settings::default();
    let parsed = Settings::parse("=\n===\nfit\n\u{0}=true\nwrap=\nsort=\n\n");
    assert_eq!(parsed.wrap, defaults.wrap);
    assert_eq!(parsed.sort_by, defaults.sort_by);
}

#[test]
fn natural_comparison_is_a_consistent_total_order() {
    use kova_image::folder_navigation::natural_cmp;
    let atoms = [
        "",
        "a",
        "B",
        "0",
        "1",
        "01",
        "2",
        "10",
        "007",
        "_",
        " ",
        "\u{e4}",
        "z",
        "99999999999999999999",
    ];
    let mut names = Vec::new();
    for a in atoms {
        for b in atoms {
            names.push(format!("{a}{b}"));
            names.push(format!("{a}{b}.png"));
        }
    }
    for x in &names {
        assert_eq!(natural_cmp(x, x), std::cmp::Ordering::Equal);
        for y in &names {
            assert_eq!(
                natural_cmp(x, y),
                natural_cmp(y, x).reverse(),
                "{x:?} {y:?}"
            );
        }
    }
    // Transitivity on a sample of triples.
    let mut rng = Rng(7);
    for _ in 0..20_000 {
        let (x, y, z) = (
            &names[rng.below(names.len())],
            &names[rng.below(names.len())],
            &names[rng.below(names.len())],
        );
        if natural_cmp(x, y).is_le() && natural_cmp(y, z).is_le() {
            assert!(natural_cmp(x, z).is_le(), "{x:?} {y:?} {z:?}");
        }
    }
}
