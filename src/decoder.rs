use crate::{
    animation::{Loops, frame_delay},
    error::Error,
    security::{self, Ticket},
};
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    created: Option<SystemTime>,
}
impl Stamp {
    pub fn from_metadata(m: &Metadata) -> Self {
        Self {
            bytes: m.len(),
            modified: m.modified().ok(),
            created: m.created().ok(),
        }
    }
    pub fn read(path: &Path) -> Result<Self, Error> {
        Ok(Self::from_metadata(&std::fs::metadata(path)?))
    }
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay: Duration,
}
/// Empty axes mean the original pixel size. Otherwise the bitmap is fitted
/// inside the box and never enlarged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub max_width: u32,
    pub max_height: u32,
}
impl Target {
    pub const fn full() -> Self {
        Self {
            max_width: 0,
            max_height: 0,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhotoInfo {
    pub taken: Option<String>,
    pub camera: Option<String>,
    pub exposure: Option<String>,
}
pub struct Decoded {
    /// Stored bitmap size. This can be smaller than the file when the view
    /// does not need every source pixel.
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub format: ImageFormat,
    pub frames: Vec<Frame>,
    pub loops: Loops,
    pub stamp: Stamp,
    pub photo: PhotoInfo,
}
impl Decoded {
    pub fn weight(&self) -> usize {
        self.frames.iter().map(|f| f.rgba.len()).sum()
    }
    pub fn serves(&self, target: Target) -> bool {
        let (w, h) = fitted_size(self.source_width, self.source_height, target);
        self.width.saturating_add(1) >= w && self.height.saturating_add(1) >= h
    }
}
pub fn fitted_size(src_w: u32, src_h: u32, target: Target) -> (u32, u32) {
    if src_w == 0 || src_h == 0 || target.max_width == 0 || target.max_height == 0 {
        return (src_w, src_h);
    }
    let scale = (f64::from(target.max_width) / f64::from(src_w))
        .min(f64::from(target.max_height) / f64::from(src_h))
        .min(1.0);
    if scale >= 0.999 {
        return (src_w, src_h);
    }
    (
        (f64::from(src_w) * scale)
            .round()
            .clamp(1.0, f64::from(src_w)) as u32,
        (f64::from(src_h) * scale)
            .round()
            .clamp(1.0, f64::from(src_h)) as u32,
    )
}

pub fn detect(bytes: &[u8]) -> Result<ImageFormat, Error> {
    let format = image::guess_format(bytes).map_err(|_| Error::Unsupported)?;
    if supported(format) {
        Ok(format)
    } else {
        Err(Error::Unsupported)
    }
}
pub fn supported(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tiff
            | ImageFormat::Ico
    )
}

struct Cancellable {
    file: File,
    ticket: Ticket,
}
impl Read for Cancellable {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if !self.ticket.is_current() {
            return Err(io::Error::other("load cancelled"));
        }
        self.file.read(b)
    }
}
impl Seek for Cancellable {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        if !self.ticket.is_current() {
            return Err(io::Error::other("load cancelled"));
        }
        self.file.seek(p)
    }
}

/// Decoder panics unwind on this worker, never on the UI thread. Allocator aborts
/// and native faults are not caught: this is resource limiting, not a sandbox.
pub fn load(path: &Path, ticket: &Ticket) -> Result<Decoded, Error> {
    load_target(path, ticket, Target::full(), &mut |_| {})
}
pub fn load_target(
    path: &Path,
    ticket: &Ticket,
    target: Target,
    preview: &mut dyn FnMut(Decoded),
) -> Result<Decoded, Error> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode(path, ticket, target, preview)
    }))
    .unwrap_or_else(|_| Err(Error::Corrupted("decoder panicked".into())));
    ticket.check()?;
    result
}
fn decode(
    path: &Path,
    ticket: &Ticket,
    target: Target,
    preview: &mut dyn FnMut(Decoded),
) -> Result<Decoded, Error> {
    ticket.check()?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // While decoding, deny writers/deleters and open the reparse point itself.
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::Io("Reparse-point images are not opened".into()));
        }
    }
    if !meta.is_file() {
        return Err(Error::Unsupported);
    }
    if meta.len() > security::MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let stamp = Stamp::from_metadata(&meta);
    let monitor = file.try_clone()?;
    let source = BufReader::with_capacity(
        64 * 1024,
        Cancellable {
            file,
            ticket: ticket.clone(),
        },
    );
    let mut reader = ImageReader::new(source).with_guessed_format()?;
    let format = reader
        .format()
        .filter(|f| supported(*f))
        .ok_or(Error::Unsupported)?;
    reader.limits(security::limits());
    // into_dimensions consumes the reader, so probe using a decoder and rewind
    // the same locked file handle before construction of the actual decoder.
    let mut probe = reader.into_decoder()?;
    let (mut width, mut height) = probe.dimensions();
    security::rgba_bytes(width, height)?;
    if probe.total_bytes() > security::DECODE_BUDGET {
        return Err(Error::MemoryBudget);
    }
    let orientation = probe
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = probe.icc_profile().ok().flatten();
    let photo = photo_from_exif(probe.exif_metadata().ok().flatten().as_deref());
    drop(probe);
    let mut source = BufReader::new(Cancellable {
        file: monitor.try_clone()?,
        ticket: ticket.clone(),
    });
    source.seek(SeekFrom::Start(0))?;
    let mut loops = Loops(Some(1));
    let mut frames = match format {
        ImageFormat::Gif => {
            // GIF stores *additional* repetitions; image's generic loop API does
            // not distinguish an absent extension. Parse only structural blocks.
            loops = gif_loops(&mut source, ticket)?;
            source.seek(SeekFrom::Start(0))?;
            let mut decoder = image::codecs::gif::GifDecoder::new(source)?;
            decoder.set_limits(security::limits())?;
            collect(
                decoder.into_frames(),
                width,
                height,
                target,
                ticket,
                &mut |frame, stored_w, stored_h| {
                    preview(partial(
                        frame,
                        (stored_w, stored_h),
                        (width, height),
                        format,
                        loops,
                        &stamp,
                        &photo,
                    ))
                },
                &icc,
            )?
        }
        ImageFormat::Png => {
            let decoder = image::codecs::png::PngDecoder::with_limits(source, security::limits())?;
            if decoder.is_apng()? {
                let decoder = decoder.apng()?;
                loops = loop_count(decoder.loop_count());
                collect(
                    decoder.into_frames(),
                    width,
                    height,
                    target,
                    ticket,
                    &mut |frame, stored_w, stored_h| {
                        preview(partial(
                            frame,
                            (stored_w, stored_h),
                            (width, height),
                            format,
                            loops,
                            &stamp,
                            &photo,
                        ))
                    },
                    &icc,
                )?
            } else {
                vec![still(decoder)?]
            }
        }
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(source)?;
            decoder.set_limits(security::limits())?;
            if decoder.has_animation() {
                loops = loop_count(decoder.loop_count());
                collect(
                    decoder.into_frames(),
                    width,
                    height,
                    target,
                    ticket,
                    &mut |frame, stored_w, stored_h| {
                        preview(partial(
                            frame,
                            (stored_w, stored_h),
                            (width, height),
                            format,
                            loops,
                            &stamp,
                            &photo,
                        ))
                    },
                    &icc,
                )?
            } else {
                vec![still(decoder)?]
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(source, format);
            reader.limits(security::limits());
            vec![still(reader.into_decoder()?)?]
        }
    };
    ticket.check()?;
    if Stamp::from_metadata(&monitor.metadata()?) != stamp || Stamp::read(path)? != stamp {
        return Err(Error::Changed);
    }
    if frames.len() == 1 && orientation != image::metadata::Orientation::NoTransforms {
        let rgba = std::mem::take(&mut frames[0].rgba);
        let mut image = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(width, height, rgba).ok_or(Error::Dimensions)?,
        );
        image.apply_orientation(orientation);
        width = image.width();
        height = image.height();
        frames[0].rgba = image.into_rgba8().into_raw();
    }
    let source_width = width;
    let source_height = height;
    // Animation frames are fitted and converted as they arrive. A single
    // frame, including a one-frame animation, is fitted here.
    if frames.len() < 2 {
        let rgba = std::mem::take(&mut frames[0].rgba);
        let (rgba, w, h) = scale_rgba(rgba, width, height, target)?;
        frames[0].rgba = rgba;
        width = w;
        height = h;
        to_srgb(&mut frames[0].rgba, icc.as_deref());
    } else {
        let (w, h) = fitted_size(source_width, source_height, target);
        width = w;
        height = h;
    }
    Ok(Decoded {
        width,
        height,
        source_width,
        source_height,
        format,
        frames,
        loops,
        stamp,
        photo,
    })
}
fn still(decoder: impl ImageDecoder) -> Result<Frame, Error> {
    let image = image::DynamicImage::from_decoder(decoder)?.into_rgba8();
    Ok(Frame {
        rgba: image.into_raw(),
        delay: Duration::from_secs(1),
    })
}
fn loop_count(count: image::metadata::LoopCount) -> Loops {
    match count {
        image::metadata::LoopCount::Infinite => Loops(None),
        image::metadata::LoopCount::Finite(n) => Loops(Some(n.get())),
    }
}
fn collect(
    mut frames: image::Frames<'_>,
    width: u32,
    height: u32,
    target: Target,
    ticket: &Ticket,
    preview: &mut dyn FnMut(Frame, u32, u32),
    icc: &Option<Vec<u8>>,
) -> Result<Vec<Frame>, Error> {
    let (stored_w, stored_h) = fitted_size(width, height, target);
    // Count the source canvas, matching the stored-budget admission used before
    // display scaling. Downscaling only reduces what is retained afterwards.
    let weight = security::rgba_bytes(width, height)?;
    let mut result: Vec<Frame> = Vec::new();
    let mut announced = false;
    loop {
        ticket.check()?;
        if result.len() == 1 && !announced {
            announced = true;
            let (mut rgba, w, h) =
                scale_rgba(std::mem::take(&mut result[0].rgba), width, height, target)?;
            if (w, h) != (stored_w, stored_h) {
                return Err(Error::Dimensions);
            }
            to_srgb(&mut rgba, icc.as_deref());
            result[0].rgba = rgba;
            preview(result[0].clone(), stored_w, stored_h);
        }
        if result.len() >= security::MAX_FRAMES
            || result.len().saturating_add(1).saturating_mul(weight) > security::FRAME_BUDGET
        {
            return Err(Error::MemoryBudget);
        }
        let Some(frame) = frames.next() else {
            break;
        };
        let frame = frame?;
        if frame.buffer().dimensions() != (width, height) {
            return Err(Error::Corrupted("invalid frame canvas".into()));
        }
        let (n, d) = frame.delay().numer_denom_ms();
        let mut rgba = frame.into_buffer().into_raw();
        if announced {
            let (scaled, w, h) = scale_rgba(rgba, width, height, target)?;
            if (w, h) != (stored_w, stored_h) {
                return Err(Error::Dimensions);
            }
            rgba = scaled;
            to_srgb(&mut rgba, icc.as_deref());
        }
        result.push(Frame {
            rgba,
            delay: frame_delay(n, d),
        });
    }
    if result.is_empty() {
        return Err(Error::Corrupted("no image frames".into()));
    }
    Ok(result)
}
fn partial(
    frame: Frame,
    stored: (u32, u32),
    source: (u32, u32),
    format: ImageFormat,
    loops: Loops,
    stamp: &Stamp,
    photo: &PhotoInfo,
) -> Decoded {
    Decoded {
        width: stored.0,
        height: stored.1,
        source_width: source.0,
        source_height: source.1,
        format,
        frames: vec![frame],
        loops,
        stamp: stamp.clone(),
        photo: photo.clone(),
    }
}
fn scale_rgba(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    target: Target,
) -> Result<(Vec<u8>, u32, u32), Error> {
    let (w, h) = fitted_size(width, height, target);
    if (w, h) == (width, height) {
        return Ok((rgba, width, height));
    }
    let image = image::RgbaImage::from_raw(width, height, rgba).ok_or(Error::Dimensions)?;
    // Triangle is the image crate's bilinear-class downscale. The codec has
    // already produced the full canvas; only the retained bitmap shrinks.
    let small = image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle);
    let (w, h) = small.dimensions();
    Ok((small.into_raw(), w, h))
}
fn to_srgb(rgba: &mut [u8], icc: Option<&[u8]>) {
    let Some(icc) = icc.filter(|p| p.len() >= 128 && rgba.len().is_multiple_of(4)) else {
        return;
    };
    // image-rs exposes the profile and does not convert pixels. moxcms is the
    // same library image already uses for CICP, applied here to 8-bit RGBA.
    // https://docs.rs/image/0.25.10/image/trait.ImageDecoder.html#method.icc_profile
    let Ok(source) = moxcms::ColorProfile::new_from_slice(icc) else {
        return;
    };
    let destination = moxcms::ColorProfile::new_srgb();
    let Ok(transform) = source.create_in_place_transform_8bit(
        moxcms::Layout::Rgba,
        &destination,
        moxcms::TransformOptions::default(),
    ) else {
        return;
    };
    let _ = transform.transform(rgba);
}
fn photo_from_exif(bytes: Option<&[u8]>) -> PhotoInfo {
    let Some(bytes) = bytes.filter(|b| b.len() >= 16) else {
        return PhotoInfo::default();
    };
    let mut data = bytes.to_vec();
    if data.starts_with(b"Exif\0\0") {
        data.drain(..6);
    }
    let Ok(exif) = exif::Reader::new().read_raw(data) else {
        return PhotoInfo::default();
    };
    let text = |tag| {
        exif.get_field(tag, exif::In::PRIMARY)
            .map(|field| field.display_value().to_string())
            .map(|value| clean_meta(&value))
            .filter(|value| !value.is_empty())
    };
    let taken = text(exif::Tag::DateTimeOriginal)
        .or_else(|| text(exif::Tag::DateTime))
        .map(|value| {
            let mut bytes = value.into_bytes();
            if bytes.len() >= 10 && bytes.get(4) == Some(&b':') && bytes.get(7) == Some(&b':') {
                bytes[4] = b'-';
                bytes[7] = b'-';
            }
            String::from_utf8(bytes).unwrap_or_default()
        })
        .filter(|value| !value.is_empty());
    let camera = match (text(exif::Tag::Make), text(exif::Tag::Model)) {
        (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (Some(make), None) => Some(make),
        (None, Some(model)) => Some(model),
        (None, None) => None,
    };
    let exposure = [
        text(exif::Tag::ExposureTime).map(|value| format!("{value} s")),
        text(exif::Tag::FNumber).map(|value| format!("f/{value}")),
        text(exif::Tag::PhotographicSensitivity).map(|value| format!("ISO {value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    PhotoInfo {
        taken,
        camera,
        exposure: if exposure.is_empty() {
            None
        } else {
            Some(exposure)
        },
    }
}
fn clean_meta(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if out.len() >= 80 {
            break;
        }
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    out.trim().trim_matches('"').trim().to_string()
}

fn gif_loops<R: Read + Seek>(r: &mut R, ticket: &Ticket) -> Result<Loops, Error> {
    let mut header = [0; 13];
    r.read_exact(&mut header)?;
    if header[10] & 0x80 != 0 {
        r.seek(SeekFrom::Current(3 * (1i64 << ((header[10] & 7) + 1))))?;
    }
    let mut loops = Loops(Some(1));
    loop {
        ticket.check()?;
        let mut b = [0];
        r.read_exact(&mut b)?;
        match b[0] {
            0x3b => return Ok(loops),
            0x2c => {
                let mut descriptor = [0; 9];
                r.read_exact(&mut descriptor)?;
                if descriptor[8] & 0x80 != 0 {
                    r.seek(SeekFrom::Current(3 * (1i64 << ((descriptor[8] & 7) + 1))))?;
                }
                r.read_exact(&mut b)?; // LZW minimum code size
                skip_blocks(r, ticket)?;
            }
            0x21 => {
                r.read_exact(&mut b)?;
                if b[0] == 0xff {
                    r.read_exact(&mut b)?;
                    let mut app = [0; 255];
                    let n = b[0] as usize;
                    r.read_exact(&mut app[..n])?;
                    let netscape = &app[..n] == b"NETSCAPE2.0" || &app[..n] == b"ANIMEXTS1.0";
                    loop {
                        r.read_exact(&mut b)?;
                        let n = b[0] as usize;
                        if n == 0 {
                            break;
                        }
                        r.read_exact(&mut app[..n])?;
                        if netscape && n == 3 && app[0] == 1 {
                            let repeats = u16::from_le_bytes([app[1], app[2]]);
                            loops = Loops(if repeats == 0 {
                                None
                            } else {
                                Some(u32::from(repeats) + 1)
                            });
                        }
                        ticket.check()?;
                    }
                } else {
                    skip_blocks(r, ticket)?;
                }
            }
            _ => return Err(Error::Corrupted("invalid GIF block".into())),
        }
    }
}
fn skip_blocks<R: Read + Seek>(r: &mut R, ticket: &Ticket) -> Result<(), Error> {
    loop {
        ticket.check()?;
        let mut n = [0];
        r.read_exact(&mut n)?;
        if n[0] == 0 {
            return Ok(());
        }
        r.seek(SeekFrom::Current(i64::from(n[0])))?;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fitted_size_never_enlarges_and_preserves_full_requests() {
        assert_eq!(fitted_size(4000, 3000, Target::full()), (4000, 3000));
        assert_eq!(
            fitted_size(
                4000,
                3000,
                Target {
                    max_width: 800,
                    max_height: 600
                }
            ),
            (800, 600)
        );
        assert_eq!(
            fitted_size(
                400,
                300,
                Target {
                    max_width: 800,
                    max_height: 600
                }
            ),
            (400, 300)
        );
        let image = Decoded {
            width: 800,
            height: 600,
            source_width: 4000,
            source_height: 3000,
            format: ImageFormat::Png,
            frames: Vec::new(),
            loops: Loops(Some(1)),
            stamp: Stamp {
                bytes: 1,
                modified: None,
                created: None,
            },
            photo: PhotoInfo::default(),
        };
        assert!(image.serves(Target {
            max_width: 800,
            max_height: 600
        }));
        assert!(!image.serves(Target::full()));
    }
    #[test]
    fn exif_model_is_read_and_garbage_is_ignored() {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0110u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&4u32.to_le_bytes());
        tiff.extend_from_slice(b"Cam\0");
        tiff.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(photo_from_exif(Some(&tiff)).camera.as_deref(), Some("Cam"));
        assert_eq!(photo_from_exif(Some(b"not exif")), PhotoInfo::default());
    }
}
