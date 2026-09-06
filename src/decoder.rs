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
pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay: Duration,
}
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub frames: Vec<Frame>,
    pub loops: Loops,
    pub stamp: Stamp,
}
impl Decoded {
    pub fn weight(&self) -> usize {
        self.frames.iter().map(|f| f.rgba.len()).sum()
    }
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(path, ticket)))
        .unwrap_or_else(|_| Err(Error::Corrupted("decoder panicked".into())));
    ticket.check()?;
    result
}
fn decode(path: &Path, ticket: &Ticket) -> Result<Decoded, Error> {
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
    let (width, height) = probe.dimensions();
    security::rgba_bytes(width, height)?;
    if probe.total_bytes() > security::DECODE_BUDGET {
        return Err(Error::MemoryBudget);
    }
    let orientation = probe
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    drop(probe);
    let mut source = BufReader::new(Cancellable {
        file: monitor.try_clone()?,
        ticket: ticket.clone(),
    });
    source.seek(SeekFrom::Start(0))?;
    let mut loops = Loops(Some(1));
    let frames = match format {
        ImageFormat::Gif => {
            // GIF stores *additional* repetitions; image's generic loop API does
            // not distinguish an absent extension. Parse only structural blocks.
            loops = gif_loops(&mut source, ticket)?;
            source.seek(SeekFrom::Start(0))?;
            let mut decoder = image::codecs::gif::GifDecoder::new(source)?;
            decoder.set_limits(security::limits())?;
            collect(decoder.into_frames(), width, height, ticket)?
        }
        ImageFormat::Png => {
            let decoder = image::codecs::png::PngDecoder::with_limits(source, security::limits())?;
            if decoder.is_apng()? {
                let decoder = decoder.apng()?;
                loops = loop_count(decoder.loop_count());
                collect(decoder.into_frames(), width, height, ticket)?
            } else {
                vec![still(decoder)?]
            }
        }
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(source)?;
            decoder.set_limits(security::limits())?;
            if decoder.has_animation() {
                loops = loop_count(decoder.loop_count());
                collect(decoder.into_frames(), width, height, ticket)?
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
    let mut result = Decoded {
        width,
        height,
        format,
        frames,
        loops,
        stamp,
    };
    if result.frames.len() == 1 && orientation != image::metadata::Orientation::NoTransforms {
        let rgba = std::mem::take(&mut result.frames[0].rgba);
        let mut image = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(width, height, rgba).ok_or(Error::Dimensions)?,
        );
        image.apply_orientation(orientation);
        result.width = image.width();
        result.height = image.height();
        result.frames[0].rgba = image.into_rgba8().into_raw();
    }
    Ok(result)
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
    ticket: &Ticket,
) -> Result<Vec<Frame>, Error> {
    let weight = security::rgba_bytes(width, height)?;
    let mut result = Vec::new();
    loop {
        ticket.check()?;
        // Leave one canvas of room for the iterator's next allocation.
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
        result.push(Frame {
            rgba: frame.into_buffer().into_raw(),
            delay: frame_delay(n, d),
        });
    }
    if result.is_empty() {
        return Err(Error::Corrupted("no image frames".into()));
    }
    Ok(result)
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
