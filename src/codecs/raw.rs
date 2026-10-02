//! Camera RAW files (ARW, CR2, CR3, DNG, NEF, ORF, RAF, RW2 and others). Kova Image
//! shows the picture the camera itself stored in the file: the largest JPEG
//! preview, which every common RAW format carries, not a development of the
//! sensor data. It is found by looking for JPEG streams and checking each one's
//! structure, which works the same for TIFF-based files, Canon CR3 and Fujifilm
//! RAF without a parser for each maker's layout.
use super::Reader;
use crate::{
    decoder::{self, Decoded, Pending, PhotoInfo, Stamp, Target},
    error::Error,
    format::Format,
    security::Ticket,
};
use image::metadata::Orientation;
use std::io::{BufRead, Cursor, Read, Seek, SeekFrom};

/// Largest preview that is read into memory.
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
/// How many JPEG streams are examined before the best so far is used.
const MAX_CANDIDATES: usize = 512;
/// Bytes at the start of the file searched for the camera's Exif data.
const EXIF_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct Preview {
    offset: u64,
    length: u64,
    width: u32,
    height: u32,
}

pub(super) fn decode(
    mut reader: Reader,
    length: u64,
    stamp: &Stamp,
    ticket: &Ticket,
    target: Target,
) -> Result<Pending, Error> {
    let (container_photo, container_orientation) = container_exif(&mut reader, length)?;
    let preview = find_preview(&mut reader, length, ticket)?
        .ok_or_else(|| Error::Corrupted("RAW: the file holds no JPEG preview".into()))?;
    ticket.check()?;
    reader.seek(SeekFrom::Start(preview.offset))?;
    let mut jpeg = Vec::with_capacity(preview.length as usize);
    (&mut reader).take(preview.length).read_to_end(&mut jpeg)?;
    if jpeg.len() as u64 != preview.length {
        return Err(Error::Corrupted(
            "RAW: the file ends inside its preview".into(),
        ));
    }
    ticket.check()?;
    let mut pending = decoder::image_pending(
        &|| Ok(Cursor::new(&jpeg[..])),
        Format::Jpeg,
        ticket,
        target,
        stamp,
        container_orientation.unwrap_or(Orientation::NoTransforms),
        &mut |_: Decoded| {},
    )?;
    pending.format = Format::Raw;
    // The camera's own Exif block is more complete than the preview's.
    let photo = &mut pending.photo;
    let merged = PhotoInfo {
        taken: container_photo.taken.or(photo.taken.take()),
        camera: container_photo.camera.or(photo.camera.take()),
        lens: container_photo.lens.or(photo.lens.take()),
        exposure: container_photo.exposure.or(photo.exposure.take()),
    };
    *photo = merged;
    Ok(pending)
}

/// Date, camera and orientation from the Exif of a TIFF-based RAW file. Makers
/// that use their own byte-order marks (Olympus, Panasonic) keep the ordinary
/// structure behind them, so the mark is replaced before reading.
fn container_exif(
    reader: &mut Reader,
    length: u64,
) -> Result<(PhotoInfo, Option<Orientation>), Error> {
    reader.seek(SeekFrom::Start(0))?;
    let mut head = Vec::with_capacity(EXIF_BYTES);
    (&mut *reader)
        .take(length.min(EXIF_BYTES as u64))
        .read_to_end(&mut head)?;
    match head.get(..2) {
        Some(b"II") => head[2..4].copy_from_slice(&[0x2a, 0]),
        Some(b"MM") => head[2..4].copy_from_slice(&[0, 0x2a]),
        _ => return Ok((PhotoInfo::default(), None)),
    }
    let orientation = exif::Reader::new()
        .read_raw(head.clone())
        .ok()
        .and_then(|exif| {
            let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
            let value = u8::try_from(field.value.get_uint(0)?).ok()?;
            Orientation::from_exif(value)
        });
    Ok((decoder::photo_from_exif(Some(&head)), orientation))
}

/// The JPEG stream with the most pixels, from those that a viewer can decode.
fn find_preview(
    reader: &mut Reader,
    length: u64,
    ticket: &Ticket,
) -> Result<Option<Preview>, Error> {
    const CHUNK: usize = 1024 * 1024;
    const SIGNATURE: [u8; 3] = [0xff, 0xd8, 0xff];
    let mut best: Option<Preview> = None;
    let mut examined = 0;
    let mut offset = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    while length.saturating_sub(offset) >= SIGNATURE.len() as u64 && examined < MAX_CANDIDATES {
        ticket.check()?;
        reader.seek(SeekFrom::Start(offset))?;
        let count = read_up_to(reader, &mut buffer)?;
        let window = &buffer[..count];
        let mut from = 0;
        let mut jump = None;
        while let Some(found) = window[from..]
            .windows(SIGNATURE.len())
            .position(|w| w == SIGNATURE)
        {
            let at = offset + (from + found) as u64;
            examined += 1;
            if let Some(preview) = examine(reader, at, length, ticket)? {
                if best.is_none_or(|b| {
                    (
                        u64::from(preview.width) * u64::from(preview.height),
                        preview.length,
                    ) > (u64::from(b.width) * u64::from(b.height), b.length)
                }) {
                    best = Some(preview);
                }
                // Anything inside this stream (an Exif thumbnail) is not another picture.
                jump = Some(at + preview.length);
                break;
            }
            if examined >= MAX_CANDIDATES {
                break;
            }
            from += found + 1;
        }
        offset = match jump {
            Some(next) => next,
            // Overlap so a signature cut by the end of the window is still found.
            None if count < SIGNATURE.len() => break,
            None => offset + (count - (SIGNATURE.len() - 1)) as u64,
        };
    }
    Ok(best)
}

fn read_up_to(reader: &mut Reader, buffer: &mut [u8]) -> Result<usize, Error> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// Follows the segments of a JPEG that starts at `at`. Returns the stream when it
/// is a DCT-coded picture of ordinary kind that ends inside the file, and nothing
/// for other data that merely starts like one (or for the lossless JPEG that
/// holds the sensor data of some formats, which no viewer decodes).
fn examine(
    reader: &mut Reader,
    at: u64,
    length: u64,
    ticket: &Ticket,
) -> Result<Option<Preview>, Error> {
    let limit = length.min(at.saturating_add(MAX_PREVIEW_BYTES));
    reader.seek(SeekFrom::Start(at + 2))?;
    let mut position = at + 2;
    let mut size = None;
    loop {
        ticket.check()?;
        if position >= limit {
            return Ok(None);
        }
        let Some(marker) = next_marker(reader, &mut position, limit)? else {
            return Ok(None);
        };
        match marker {
            // Standalone markers have no length.
            0x01 | 0xd0..=0xd7 | 0xd8 => {}
            0xd9 => {
                // The end of the stream.
                return Ok(size.map(|(width, height)| Preview {
                    offset: at,
                    length: position - at,
                    width,
                    height,
                }));
            }
            _ => {
                let mut header = [0u8; 2];
                if reader.read_exact(&mut header).is_err() {
                    return Ok(None);
                }
                position += 2;
                let segment = u64::from(u16::from_be_bytes(header));
                if segment < 2 || position + segment - 2 > limit {
                    return Ok(None);
                }
                let body = segment - 2;
                match marker {
                    // Baseline, extended sequential and progressive DCT.
                    0xc0..=0xc2 => {
                        let mut sof = [0u8; 6];
                        if body < 6 || reader.read_exact(&mut sof).is_err() {
                            return Ok(None);
                        }
                        let height = u32::from(u16::from_be_bytes([sof[1], sof[2]]));
                        let width = u32::from(u16::from_be_bytes([sof[3], sof[4]]));
                        if sof[0] != 8 || !(sof[5] == 1 || sof[5] == 3) || width == 0 || height == 0
                        {
                            return Ok(None);
                        }
                        size = Some((width, height));
                        reader.seek_relative(body as i64 - 6)?;
                    }
                    // Lossless, hierarchical and arithmetic coding.
                    0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return Ok(None),
                    0xda => {
                        // Start of scan: entropy-coded data follows the header
                        // and runs to the next marker.
                        if size.is_none() {
                            return Ok(None);
                        }
                        reader.seek_relative(body as i64)?;
                        position += body;
                        match skip_scan(reader, &mut position, limit)? {
                            true => continue,
                            false => return Ok(None),
                        }
                    }
                    _ => reader.seek_relative(body as i64)?,
                }
                position += body;
            }
        }
    }
}

/// The next marker byte: `FF xx` where `xx` is neither a fill byte nor a stuffed zero.
fn next_marker(reader: &mut Reader, position: &mut u64, limit: u64) -> Result<Option<u8>, Error> {
    let mut read = |position: &mut u64| -> Option<u8> {
        let mut byte = [0u8; 1];
        if *position >= limit || reader.read_exact(&mut byte).is_err() {
            return None;
        }
        *position += 1;
        Some(byte[0])
    };
    if read(position) != Some(0xff) {
        return Ok(None);
    }
    // Any number of 0xFF fill bytes may precede the marker code.
    loop {
        match read(position) {
            Some(0xff) => continue,
            Some(0x00) | None => return Ok(None),
            Some(marker) => return Ok(Some(marker)),
        }
    }
}

/// Moves over entropy-coded data to the next real marker, leaving the reader just
/// before it. False when the data ends first.
fn skip_scan(reader: &mut Reader, position: &mut u64, limit: u64) -> Result<bool, Error> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() || *position >= limit {
            return Ok(false);
        }
        let available = buffer.len().min((limit - *position) as usize);
        let Some(at) = buffer[..available].iter().position(|&b| b == 0xff) else {
            reader.consume(available);
            *position += available as u64;
            continue;
        };
        reader.consume(at);
        *position += at as u64;
        // Look at the byte after the 0xFF: a stuffed zero or a restart marker
        // belongs to the data; anything else ends it.
        let mut pair = [0u8; 2];
        if reader.read_exact(&mut pair).is_err() {
            return Ok(false);
        }
        match pair[1] {
            0x00 | 0xd0..=0xd7 | 0xff => {
                // Step back one byte so a following 0xFF is looked at too.
                reader.seek_relative(-1)?;
                *position += 1;
            }
            _ => {
                reader.seek_relative(-2)?;
                return Ok(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decoder::Cancellable, security};
    use std::io::BufReader;

    fn reader_over(bytes: &[u8]) -> (Reader, u64, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "kova-raw-{}-{}.bin",
            std::process::id(),
            bytes.len()
        ));
        std::fs::write(&path, bytes).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let ticket = security::Generation::default().next();
        (
            BufReader::new(Cancellable {
                source: Box::new(file),
                ticket,
            }),
            bytes.len() as u64,
            path,
        )
    }
    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            u32::from(width),
            u32::from(height),
            image::Rgb([200, 100, 50]),
        ))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
        .unwrap();
        bytes
    }

    #[test]
    fn the_largest_ordinary_jpeg_is_chosen_among_other_data() {
        let small = jpeg(16, 12);
        let large = jpeg(64, 48);
        let mut file = b"II*\0\x08\0\0\0 sensor data ".to_vec();
        // A lossless-JPEG header (SOF3) with a huge size must be ignored.
        file.extend_from_slice(&[
            0xff, 0xd8, 0xff, 0xc3, 0, 11, 8, 0x20, 0, 0x20, 0, 1, 1, 0x11, 0,
        ]);
        file.extend_from_slice(&[0xff, 0xd8, 0xff, 0x00, 1, 2, 3]);
        file.extend_from_slice(&small);
        file.extend_from_slice(&[0x55; 1000]);
        let at = file.len() as u64;
        file.extend_from_slice(&large);
        file.extend_from_slice(&[0xff, 0xd8, 0xff]);
        let (mut reader, length, path) = reader_over(&file);
        let ticket = security::Generation::default().next();
        let found = find_preview(&mut reader, length, &ticket).unwrap().unwrap();
        assert_eq!((found.width, found.height), (64, 48));
        assert_eq!(found.offset, at);
        assert_eq!(found.length, large.len() as u64);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn no_preview_is_reported_as_none() {
        let (mut reader, length, path) = reader_over(&[0xff, 0xd8, 0xff, 0xe0, 0, 3, 0]);
        let ticket = security::Generation::default().next();
        assert!(
            find_preview(&mut reader, length, &ticket)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_file(path);
    }
}
