use image::{DynamicImage, ImageFormat, RgbaImage};
use kova_image::{
    animation::Loops,
    cache::Cache,
    decoder::{self, Stamp},
    error::Error,
    folder_navigation,
    security::{self, Generation},
};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kova-image-test-{}-{}",
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
fn encoded(format: ImageFormat) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(16, 12, |x, y| {
        image::Rgba([x as u8 * 10, y as u8 * 10, 80, 255])
    }));
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(image.into_rgb8())
    } else {
        image
    };
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, format).unwrap();
    output.into_inner()
}
#[test]
fn actual_decodes_and_magic_bytes() {
    let temp = Temp::new();
    for format in [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::WebP,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::Ico,
    ] {
        let bytes = encoded(format);
        assert_eq!(decoder::detect(&bytes), Ok(format));
        let path = temp.write("wrong-extension.dat", &bytes);
        let image = decoder::load(&path, &Generation::default().next()).unwrap();
        assert_eq!((image.width, image.height), (16, 12));
        assert_eq!(image.weight(), 16 * 12 * 4);
        assert_eq!(image.frames.len(), 1);
    }
    assert_eq!(
        decoder::detect(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
        Err(Error::Unsupported)
    );
}
#[test]
fn missing_corrupted_and_disappearing() {
    let temp = Temp::new();
    let ticket = Generation::default().next();
    assert!(matches!(
        decoder::load(&temp.0.join("missing.png"), &ticket),
        Err(Error::NotFound)
    ));
    let path = temp.write("bad.png", b"\x89PNG\r\n\x1a\ncorrupt");
    assert!(decoder::load(&path, &ticket).is_err());
    let path = temp.write("gone.png", &encoded(ImageFormat::Png));
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        decoder::load(&path, &ticket),
        Err(Error::NotFound)
    ));
    let path = temp.write("fake.jpg", b"This is not an image");
    assert!(matches!(
        decoder::load(&path, &ticket),
        Err(Error::Unsupported)
    ));
}
#[test]
fn unicode_and_long_paths() {
    let temp = Temp::new();
    let mut dir = temp.0.clone();
    for _ in 0..8 {
        dir = dir.join("long-unicode-猫-ä-01234567890123456789");
    }
    // Extended Win32 prefix also lets the test runner operate without our app manifest.
    #[cfg(windows)]
    let dir = PathBuf::from(format!("\\\\?\\{}", dir.display()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("画像-😀.PNG");
    std::fs::write(&path, encoded(ImageFormat::Png)).unwrap();
    assert!(path.as_os_str().len() > 260);
    assert!(decoder::load(&path, &Generation::default().next()).is_ok());
}
#[test]
fn empty_directory_and_unknown_files() {
    let temp = Temp::new();
    let ticket = Generation::default().next();
    let files = folder_navigation::scan(&temp.0.join("missing.png"), &ticket, true).unwrap();
    assert_eq!(files.len(), 1);
    temp.write("a.txt", b"x");
    temp.write("image10.PNG", &encoded(ImageFormat::Png));
    temp.write("image2.png", &encoded(ImageFormat::Png));
    let files = folder_navigation::scan(&temp.0.join("image2.png"), &ticket, true).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].file_name().unwrap(), "image2.png");
}
#[test]
fn cache_eviction_budget_and_changed_file() {
    let temp = Temp::new();
    let path = temp.write("a.png", &encoded(ImageFormat::Png));
    let ticket = Generation::default().next();
    let image = Arc::new(decoder::load(&path, &ticket).unwrap());
    let weight = image.weight();
    let mut cache = Cache::new(weight * 2);
    cache.insert("a".into(), image.clone());
    cache.insert("b".into(), image.clone());
    assert!(cache.get(std::path::Path::new("a"), &image.stamp).is_some());
    cache.insert("c".into(), image.clone());
    assert!(cache.get(std::path::Path::new("b"), &image.stamp).is_none());
    assert_eq!(cache.used(), weight * 2);
    let mut tiny = Cache::new(weight - 1);
    tiny.insert(path.clone(), image.clone());
    assert_eq!(tiny.used(), 0);
    cache.insert(path.clone(), image.clone());
    std::fs::write(&path, b"changed").unwrap();
    let stamp = Stamp::read(&path).unwrap();
    assert!(cache.get(&path, &stamp).is_none());
    assert!(cache.used() <= weight * 2);
}
#[test]
fn huge_headers_and_oversized_files() {
    let temp = Temp::new();
    let path = temp.0.join("huge.png");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(security::MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    assert!(matches!(
        decoder::load(&path, &Generation::default().next()),
        Err(Error::TooLarge)
    ));
    // Valid BMP header carrying dimensions that would overflow 32-bit RGBA math.
    let mut bmp = vec![0u8; 54];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&100000i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&100000i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
    let path = temp.write("bomb.bmp", &bmp);
    assert!(decoder::load(&path, &Generation::default().next()).is_err());
}
fn gif_bytes(repeat: Option<gif::Repeat>) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 2, 1, &[255, 0, 0, 0, 0, 255]).unwrap();
        if let Some(repeat) = repeat {
            encoder.set_repeat(repeat).unwrap();
        }
        let first = gif::Frame {
            width: 2,
            height: 1,
            delay: 3,
            buffer: std::borrow::Cow::Borrowed(&[0, 0]),
            dispose: gif::DisposalMethod::Keep,
            ..gif::Frame::default()
        };
        encoder.write_frame(&first).unwrap();
        let second = gif::Frame {
            width: 1,
            height: 1,
            left: 1,
            delay: 11,
            buffer: std::borrow::Cow::Borrowed(&[1]),
            dispose: gif::DisposalMethod::Previous,
            ..gif::Frame::default()
        };
        encoder.write_frame(&second).unwrap();
        let third = gif::Frame {
            width: 1,
            height: 1,
            left: 0,
            delay: 4,
            buffer: std::borrow::Cow::Borrowed(&[1]),
            dispose: gif::DisposalMethod::Keep,
            ..gif::Frame::default()
        };
        encoder.write_frame(&third).unwrap();
    }
    bytes
}
#[test]
fn gif_disposal_durations_and_loop_metadata() {
    let temp = Temp::new();
    for (repeat, expected) in [
        (None, Loops(Some(1))),
        (Some(gif::Repeat::Finite(2)), Loops(Some(3))),
        (Some(gif::Repeat::Infinite), Loops(None)),
    ] {
        let path = temp.write("animation.gif", &gif_bytes(repeat));
        let image = decoder::load(&path, &Generation::default().next()).unwrap();
        assert_eq!(image.loops, expected);
        assert_eq!(image.frames.len(), 3);
        assert_eq!(image.frames[0].delay.as_millis(), 30);
        assert_eq!(image.frames[1].delay.as_millis(), 110);
        assert_eq!(image.frames[1].rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(image.frames[2].rgba, [0, 0, 255, 255, 255, 0, 0, 255]);
    }
}
#[test]
fn apng_frames_and_loops() {
    let temp = Temp::new();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(2, 3).unwrap();
        encoder.set_frame_delay(1, 10).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 255, 0, 0, 255])
            .unwrap();
        writer.set_frame_delay(1, 5).unwrap();
        writer
            .write_image_data(&[0, 0, 255, 255, 0, 0, 255, 255])
            .unwrap();
    }
    let path = temp.write("animation.png", &bytes);
    let image = decoder::load(&path, &Generation::default().next()).unwrap();
    assert_eq!(image.frames.len(), 2);
    assert_eq!(image.loops, Loops(Some(3)));
    assert_eq!(image.frames[1].delay.as_millis(), 200);
    assert_eq!(&image.frames[1].rgba[..4], &[0, 0, 255, 255]);
}
fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::from(*tag);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
    out
}
#[test]
fn animated_webp_frames_and_loops() {
    let temp = Temp::new();
    let mut riff = Vec::from(*b"WEBP");
    riff.extend(chunk(b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 0, 0, 0]));
    riff.extend(chunk(b"ANIM", &[0, 0, 0, 0, 2, 0]));
    for (color, delay) in [([255, 0, 0, 255], 30u8), ([0, 0, 255, 255], 90u8)] {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 1, image::Rgba(color)));
        let mut webp = Cursor::new(Vec::new());
        image.write_to(&mut webp, ImageFormat::WebP).unwrap();
        let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, delay, 0, 0, 2];
        frame.extend_from_slice(&webp.into_inner()[12..]);
        riff.extend(chunk(b"ANMF", &frame));
    }
    let mut bytes = Vec::from(*b"RIFF");
    bytes.extend_from_slice(&(riff.len() as u32).to_le_bytes());
    bytes.extend(riff);
    let path = temp.write("animation.webp", &bytes);
    let image = decoder::load(&path, &Generation::default().next()).unwrap();
    assert_eq!(image.frames.len(), 2);
    assert_eq!(image.loops, Loops(Some(2)));
    assert_eq!(image.frames[1].delay.as_millis(), 90);
    assert_eq!(&image.frames[1].rgba[..4], &[0, 0, 255, 255]);
}
#[test]
fn cancelled_decode_and_folder_scan() {
    let temp = Temp::new();
    let path = temp.write("image.png", &encoded(ImageFormat::Png));
    let generation = Generation::default();
    let stale = generation.next();
    let _latest = generation.next();
    assert!(matches!(
        decoder::load(&path, &stale),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        folder_navigation::scan(&path, &stale, true),
        Err(Error::Cancelled)
    );
}

#[test]
fn latest_worker_request_and_recovery() {
    use kova_image::image_loader::{Event, Loader};
    let temp = Temp::new();
    let good = temp.write("good.png", &encoded(ImageFormat::Png));
    let (tx, rx) = std::sync::mpsc::channel();
    let loader = Loader::new(move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    let missing = loader.request(temp.0.join("missing.png"), vec![], false, true);
    let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(matches!(event,Event::Image {id,result:Err(Error::NotFound),..} if id==missing));
    for _ in 0..100 {
        loader.request(good.clone(), vec![], false, true);
    }
    let latest = loader.request(good.clone(), vec![], true, true);
    loop {
        let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        if let Event::Image {
            id, path, result, ..
        } = event
            && id == latest {
                assert_eq!(path, good);
                assert!(result.is_ok());
                break;
            }
    }
}

#[test]
fn animation_frame_limit_and_truncated_frame() {
    let temp = Temp::new();
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 1, 1, &[0, 0, 0, 255, 255, 255]).unwrap();
        let frame = gif::Frame {
            width: 1,
            height: 1,
            buffer: std::borrow::Cow::Borrowed(&[0]),
            ..gif::Frame::default()
        };
        for _ in 0..=security::MAX_FRAMES {
            encoder.write_frame(&frame).unwrap();
        }
    }
    let path = temp.write("too-many.gif", &bytes);
    assert!(matches!(
        decoder::load(&path, &Generation::default().next()),
        Err(Error::MemoryBudget)
    ));
    let bytes = gif_bytes(None);
    let path = temp.write("truncated.gif", &bytes[..bytes.len() / 2]);
    assert!(decoder::load(&path, &Generation::default().next()).is_err());
}
