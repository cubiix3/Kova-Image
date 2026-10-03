//! What a song file says about itself: title, artist, album and the cover
//! picture. It is read before playback so the window can show something more
//! than a file name. Everything here is best effort: a tag that is damaged,
//! unknown or too large is ignored, never an error, and nothing is read beyond
//! fixed limits.
//!
//! Formats: ID3v2 and ID3v1 (MP3, and AAC or FLAC with a tag in front), the
//! `ilst` atoms of MP4 audio (M4A), and FLAC metadata blocks. WAV, Ogg, Opus and
//! WMA show no tags.
use crate::{
    decoder::{self, Decoded, Source, Target},
    media::VideoKind,
    security::Ticket,
};
use std::{
    cell::Cell,
    fs::File,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

/// The largest tag, metadata block or `moov` box that is read.
const MAX_TAG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;
/// Everything read from one file for its tags, all blocks together: a file with
/// many large blocks must not keep the decode worker busy.
const MAX_TOTAL_BYTES: u64 = 48 * 1024 * 1024;
/// Reads are made in pieces of this size, and the request is checked in between.
const READ_CHUNK: usize = 1024 * 1024;
/// The largest cover picture that is decoded.
const MAX_PICTURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// Tags and decoded cover of an audio file.
pub struct AudioInfo {
    pub tags: Tags,
    /// The cover, already fitted to the window like any picture.
    pub cover: Option<Arc<Decoded>>,
}

/// Reads the tags and cover of an audio file without moving its read position.
pub fn read_info(
    file: &File,
    length: u64,
    kind: VideoKind,
    ticket: &Ticket,
    target: Target,
) -> AudioInfo {
    let source = FileBytes {
        file,
        length,
        ticket,
        budget: Cell::new(MAX_TOTAL_BYTES),
    };
    let found = catch_unwind(AssertUnwindSafe(|| read_tags(&source, kind))).unwrap_or_default();
    let cover = found
        .picture
        .and_then(|(_, bytes)| decode_cover(bytes, ticket, target));
    AudioInfo {
        tags: Tags {
            title: found.title.unwrap_or_default(),
            artist: found.artist.unwrap_or_default(),
            album: found.album.unwrap_or_default(),
        },
        cover,
    }
}

fn decode_cover(bytes: Vec<u8>, ticket: &Ticket, target: Target) -> Option<Arc<Decoded>> {
    let bytes: Arc<[u8]> = bytes.into();
    let length = bytes.len() as u64;
    let open = || -> Result<Box<dyn Source>, crate::error::Error> {
        Ok(Box::new(Cursor::new(bytes.clone())))
    };
    decoder::load_stream(&open, length, None, ticket, target)
        .ok()
        .map(Arc::new)
}

/// Read access to the bytes of a file or of a buffer.
trait Bytes {
    fn length(&self) -> u64;
    /// `len` bytes at `at`, or `None` if they are not all there.
    fn read(&self, at: u64, len: u64) -> Option<Vec<u8>>;
}

struct FileBytes<'a> {
    file: &'a File,
    length: u64,
    /// The request this is read for: reading stops when it is replaced.
    ticket: &'a Ticket,
    /// What may still be read from this file.
    budget: Cell<u64>,
}
impl Bytes for FileBytes<'_> {
    fn length(&self) -> u64 {
        self.length
    }
    fn read(&self, at: u64, len: u64) -> Option<Vec<u8>> {
        if len > MAX_TAG_BYTES.max(MAX_MOOV_BYTES)
            || len > self.budget.get()
            || at.checked_add(len)? > self.length
            || !self.ticket.is_current()
        {
            return None;
        }
        self.budget.set(self.budget.get() - len);
        let mut buffer = vec![0u8; len as usize];
        let mut done = 0;
        while done < buffer.len() {
            // A slow disk or a cloud placeholder: look at the request between pieces.
            if !self.ticket.is_current() {
                return None;
            }
            let end = buffer.len().min(done + READ_CHUNK);
            match read_at(self.file, &mut buffer[done..end], at + done as u64) {
                Ok(0) | Err(_) => return None,
                Ok(n) => done += n,
            }
        }
        Some(buffer)
    }
}
#[cfg(windows)]
fn read_at(file: &File, buffer: &mut [u8], at: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, at)
}
#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], at: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buffer, at)
}

#[derive(Default)]
struct Found {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    /// The picture type (3 is the front cover) and the encoded picture.
    picture: Option<(u8, Vec<u8>)>,
}
impl Found {
    fn text(slot: &mut Option<String>, value: String) {
        if slot.is_none() && !value.is_empty() {
            *slot = Some(value);
        }
    }
    fn offer_picture(&mut self, kind: u8, data: &[u8]) {
        if data.is_empty() || data.len() > MAX_PICTURE_BYTES {
            return;
        }
        // The first picture counts, unless a front cover turns up later.
        if self
            .picture
            .as_ref()
            .is_none_or(|(have, _)| *have != 3 && kind == 3)
        {
            self.picture = Some((kind, data.to_vec()));
        }
    }
}

fn read_tags(source: &dyn Bytes, kind: VideoKind) -> Found {
    let mut found = Found::default();
    match kind {
        VideoKind::Mp3 | VideoKind::Aac | VideoKind::Flac => {
            let after = id3v2(source, &mut found);
            if kind == VideoKind::Flac {
                flac(source, after.unwrap_or(0), &mut found);
            } else if found.title.is_none() && found.artist.is_none() {
                id3v1(source, &mut found);
            }
        }
        VideoKind::M4a => mp4(source, &mut found),
        _ => {}
    }
    found
}

fn syncsafe(bytes: &[u8]) -> Option<u32> {
    bytes
        .iter()
        .try_fold(0u32, |n, &b| (b & 0x80 == 0).then(|| n << 7 | u32::from(b)))
}
fn be24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2])
}
fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Makes a text safe to show: control characters become spaces, the length is
/// bounded.
fn tidy(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_TEXT_CHARS)
        .collect();
    cleaned.trim().to_string()
}

/// Text in the "ISO-8859-1" encoding of ID3 (and of ID3v1). Many taggers put
/// UTF-8 there; valid UTF-8 with non-ASCII characters is far too unlikely in real
/// Latin-1 text to be anything else, so it is read as UTF-8. Text in a legacy code
/// page (Korean, Japanese) cannot be told from Latin-1 and shows as Latin-1.
fn latin1_or_utf8(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.is_ascii() => tidy(text),
        _ => tidy(&bytes.iter().map(|&b| char::from(b)).collect::<String>()),
    }
}

/// An ID3 text in one of its four encodings, up to its first terminator.
fn id3_string(encoding: u8, bytes: &[u8]) -> String {
    match encoding {
        1 | 2 => {
            let (big, body) = match bytes {
                [0xff, 0xfe, rest @ ..] => (false, rest),
                [0xfe, 0xff, rest @ ..] => (true, rest),
                _ => (encoding == 2, bytes),
            };
            let units: Vec<u16> = body
                .chunks_exact(2)
                .map(|pair| {
                    if big {
                        u16::from_be_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_le_bytes([pair[0], pair[1]])
                    }
                })
                .take_while(|&unit| unit != 0)
                .collect();
            tidy(&String::from_utf16_lossy(&units))
        }
        3 => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            tidy(&String::from_utf8_lossy(&bytes[..end]))
        }
        _ => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            latin1_or_utf8(&bytes[..end])
        }
    }
}
/// The length of an encoded string including its terminator, inside `bytes`.
fn id3_terminated(encoding: u8, bytes: &[u8]) -> Option<usize> {
    if matches!(encoding, 1 | 2) {
        bytes
            .chunks_exact(2)
            .position(|pair| pair == [0, 0])
            .map(|at| at * 2 + 2)
    } else {
        bytes.iter().position(|&b| b == 0).map(|at| at + 1)
    }
}

/// An `APIC` (or ID3v2.2 `PIC`) frame: the picture type and the picture.
fn id3_picture(data: &[u8], old: bool) -> Option<(u8, &[u8])> {
    let encoding = *data.first()?;
    let mut at = 1;
    if old {
        at += 3; // a three letter format, not a MIME type
    } else {
        at += data.get(at..)?.iter().position(|&b| b == 0)? + 1;
    }
    let kind = *data.get(at)?;
    at += 1;
    at += id3_terminated(encoding, data.get(at..)?)?;
    Some((kind, data.get(at..)?))
}

fn unsynchronise(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        out.push(bytes[i]);
        if bytes[i] == 0xff && bytes.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

/// Reads an ID3v2 tag at the start of the file and returns where the audio
/// begins.
fn id3v2(source: &dyn Bytes, found: &mut Found) -> Option<u64> {
    let head = source.read(0, 10)?;
    if &head[..3] != b"ID3" || !(2..=4).contains(&head[3]) {
        return None;
    }
    let (major, flags) = (head[3], head[5]);
    let size = u64::from(syncsafe(&head[6..10])?);
    let end = 10
        + size
        + if major == 4 && flags & 0x10 != 0 {
            10
        } else {
            0
        };
    if size > MAX_TAG_BYTES {
        return Some(end);
    }
    let Some(mut body) = source.read(10, size) else {
        return Some(end);
    };
    if flags & 0x80 != 0 && major <= 3 {
        body = unsynchronise(&body);
    }
    let mut at = 0usize;
    if flags & 0x40 != 0 {
        // The extended header: its size includes itself in v2.4 but not in v2.3.
        let skip = body.get(..4).and_then(|b| {
            if major == 4 {
                syncsafe(b).map(|n| n as usize)
            } else {
                Some(be32(b) as usize + 4)
            }
        });
        at = skip.unwrap_or(body.len());
    }
    let (id_len, header_len) = if major == 2 { (3, 6) } else { (4, 10) };
    while at + header_len <= body.len() {
        let id = &body[at..at + id_len];
        if id[0] == 0 {
            break; // padding
        }
        let size = match major {
            2 => be24(&body[at + 3..at + 6]) as usize,
            3 => be32(&body[at + 4..at + 8]) as usize,
            _ => match syncsafe(&body[at + 4..at + 8]) {
                Some(n) => n as usize,
                None => break,
            },
        };
        let frame_flags = if major == 2 {
            0
        } else {
            u16::from_be_bytes([body[at + 8], body[at + 9]])
        };
        let start = at + header_len;
        let Some(stop) = start.checked_add(size).filter(|&e| e <= body.len()) else {
            break;
        };
        at = stop;
        // Compressed and encrypted frames cannot be read.
        let unreadable = match major {
            3 => frame_flags & 0x00c0 != 0,
            4 => frame_flags & 0x000c != 0,
            _ => false,
        };
        if unreadable {
            continue;
        }
        let mut data = body[start..stop].to_vec();
        if major == 4 {
            if frame_flags & 0x0040 != 0 && !data.is_empty() {
                data.remove(0); // grouping identity byte
            }
            if frame_flags & 0x0001 != 0 && data.len() >= 4 {
                data.drain(..4); // data length indicator
            }
            if frame_flags & 0x0002 != 0 {
                data = unsynchronise(&data);
            }
        }
        match id {
            b"TT2" | b"TIT2" => {
                if let Some((&e, text)) = data.split_first() {
                    Found::text(&mut found.title, id3_string(e, text));
                }
            }
            b"TP1" | b"TPE1" => {
                if let Some((&e, text)) = data.split_first() {
                    Found::text(&mut found.artist, id3_string(e, text));
                }
            }
            b"TAL" | b"TALB" => {
                if let Some((&e, text)) = data.split_first() {
                    Found::text(&mut found.album, id3_string(e, text));
                }
            }
            b"PIC" | b"APIC" => {
                if let Some((kind, picture)) = id3_picture(&data, major == 2) {
                    found.offer_picture(kind, picture);
                }
            }
            _ => {}
        }
    }
    Some(end)
}

/// The fixed 128 byte tag at the end of old MP3 files.
fn id3v1(source: &dyn Bytes, found: &mut Found) {
    let length = source.length();
    let Some(at) = length.checked_sub(128) else {
        return;
    };
    let Some(tag) = source.read(at, 128) else {
        return;
    };
    if &tag[..3] != b"TAG" {
        return;
    }
    let field = |range: std::ops::Range<usize>| {
        let bytes = &tag[range];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        latin1_or_utf8(&bytes[..end])
    };
    Found::text(&mut found.title, field(3..33));
    Found::text(&mut found.artist, field(33..63));
    Found::text(&mut found.album, field(63..93));
}

/// `KEY=value` comments of a Vorbis comment block (FLAC).
fn vorbis_comments(data: &[u8], found: &mut Found) {
    let Some(vendor) = data.get(..4).map(|b| le32(b) as usize) else {
        return;
    };
    let Some(mut at) = vendor.checked_add(4) else {
        return;
    };
    let Some(count) = data.get(at..at + 4).map(le32) else {
        return;
    };
    at += 4;
    for _ in 0..count.min(10_000) {
        let Some(len) = data.get(at..at + 4).map(|b| le32(b) as usize) else {
            return;
        };
        at += 4;
        let Some(entry) = at.checked_add(len).and_then(|end| data.get(at..end)) else {
            return;
        };
        at += len;
        let Some(split) = entry.iter().position(|&b| b == b'=') else {
            continue;
        };
        let key = &entry[..split];
        let value = tidy(&String::from_utf8_lossy(&entry[split + 1..]));
        if key.eq_ignore_ascii_case(b"title") {
            Found::text(&mut found.title, value);
        } else if key.eq_ignore_ascii_case(b"artist") {
            Found::text(&mut found.artist, value);
        } else if key.eq_ignore_ascii_case(b"album") {
            Found::text(&mut found.album, value);
        }
    }
}

/// A FLAC `PICTURE` block.
fn flac_picture(data: &[u8], found: &mut Found) {
    let field = |at: usize| data.get(at..at + 4).map(be32);
    let Some(kind) = field(0) else { return };
    let Some(mime) = field(4).map(|n| n as usize) else {
        return;
    };
    let Some(at) = 8usize.checked_add(mime) else {
        return;
    };
    let Some(description) = field(at).map(|n| n as usize) else {
        return;
    };
    // Width, height, depth and colour count follow the description.
    let Some(at) = at.checked_add(4 + description + 16) else {
        return;
    };
    let Some(length) = field(at).map(|n| n as usize) else {
        return;
    };
    if let Some(picture) = (at + 4)
        .checked_add(length)
        .and_then(|end| data.get(at + 4..end))
    {
        found.offer_picture(u8::try_from(kind).unwrap_or(0), picture);
    }
}

/// Reads the metadata blocks of a FLAC file starting at `start`.
fn flac(source: &dyn Bytes, start: u64, found: &mut Found) {
    if source.read(start, 4).as_deref() != Some(b"fLaC".as_slice()) {
        return;
    }
    let mut at = start + 4;
    for _ in 0..64 {
        let Some(head) = source.read(at, 4) else {
            return;
        };
        let (last, kind, len) = (
            head[0] & 0x80 != 0,
            head[0] & 0x7f,
            u64::from(be24(&head[1..])),
        );
        at += 4;
        if len <= MAX_TAG_BYTES {
            match kind {
                4 => {
                    if let Some(block) = source.read(at, len) {
                        vorbis_comments(&block, found);
                    }
                }
                6 => {
                    if let Some(block) = source.read(at, len) {
                        flac_picture(&block, found);
                    }
                }
                _ => {}
            }
        }
        at += len;
        if last {
            return;
        }
    }
}

/// The boxes inside `data`: name and payload. Stops at the first box that does
/// not fit.
fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let head = data.get(at..at + 8)?;
        let size32 = u64::from(be32(head));
        let name = [head[4], head[5], head[6], head[7]];
        let (size, header) = match size32 {
            0 => ((data.len() - at) as u64, 8),
            1 => (
                u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?),
                16,
            ),
            n => (n, 8),
        };
        if size < header as u64 || size > (data.len() - at) as u64 {
            return None;
        }
        let payload = &data[at + header..at + size as usize];
        at += size as usize;
        Some((name, payload))
    })
}
fn child<'a>(data: &'a [u8], name: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data)
        .find(|(n, _)| n == name)
        .map(|(_, payload)| payload)
}

/// `moov/udta/meta/ilst` of an MP4 file: the iTunes style tags.
fn mp4(source: &dyn Bytes, found: &mut Found) {
    let length = source.length();
    let mut at = 0u64;
    for _ in 0..10_000 {
        let Some(head) = source.read(at, 16.min(length.saturating_sub(at))) else {
            return;
        };
        if head.len() < 8 {
            return;
        }
        let size32 = u64::from(be32(&head));
        let (size, header) = match size32 {
            0 => (length - at, 8),
            1 if head.len() >= 16 => (u64::from_be_bytes(head[8..16].try_into().unwrap()), 16),
            n => (n, 8),
        };
        if size < header || size > length - at {
            return;
        }
        if &head[4..8] == b"moov" {
            if size - header <= MAX_MOOV_BYTES
                && let Some(moov) = source.read(at + header, size - header)
            {
                ilst(&moov, found);
            }
            return;
        }
        at += size;
    }
}
fn ilst(moov: &[u8], found: &mut Found) {
    let Some(udta) = child(moov, b"udta") else {
        return;
    };
    let Some(meta) = child(udta, b"meta") else {
        return;
    };
    // iTunes writes a version and flags word before the children; QuickTime does not.
    let meta = if meta.get(4..8) == Some(b"hdlr".as_slice()) {
        meta
    } else {
        meta.get(4..).unwrap_or_default()
    };
    let Some(list) = child(meta, b"ilst") else {
        return;
    };
    for (name, item) in boxes(list) {
        let Some(data) = child(item, b"data") else {
            continue;
        };
        // A type word and a locale word, then the value.
        let (Some(kind), Some(value)) =
            (data.get(..4).map(|b| be32(b) & 0x00ff_ffff), data.get(8..))
        else {
            continue;
        };
        match &name {
            b"\xa9nam" => Found::text(&mut found.title, tidy(&String::from_utf8_lossy(value))),
            b"\xa9ART" => Found::text(&mut found.artist, tidy(&String::from_utf8_lossy(value))),
            b"aART" if found.artist.is_none() => {
                Found::text(&mut found.artist, tidy(&String::from_utf8_lossy(value)));
            }
            b"\xa9alb" => Found::text(&mut found.album, tidy(&String::from_utf8_lossy(value))),
            b"covr" if matches!(kind, 13 | 14 | 27) => found.offer_picture(3, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Memory(Vec<u8>);
    impl Bytes for Memory {
        fn length(&self) -> u64 {
            self.0.len() as u64
        }
        fn read(&self, at: u64, len: u64) -> Option<Vec<u8>> {
            let end = at.checked_add(len)?;
            self.0
                .get(at as usize..usize::try_from(end).ok()?)
                .map(<[u8]>::to_vec)
        }
    }
    fn tags_of(bytes: Vec<u8>, kind: VideoKind) -> Found {
        read_tags(&Memory(bytes), kind)
    }
    fn syncsafe_bytes(n: usize) -> [u8; 4] {
        [
            (n >> 21) as u8 & 0x7f,
            (n >> 14) as u8 & 0x7f,
            (n >> 7) as u8 & 0x7f,
            n as u8 & 0x7f,
        ]
    }
    /// An ID3v2 tag (`major` 2, 3 or 4) with the given frames.
    fn id3(major: u8, frames: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, data) in frames {
            body.extend_from_slice(id.as_bytes());
            match major {
                2 => body.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]),
                3 => body.extend_from_slice(&(data.len() as u32).to_be_bytes()),
                _ => body.extend_from_slice(&syncsafe_bytes(data.len())),
            }
            if major > 2 {
                body.extend_from_slice(&[0, 0]);
            }
            body.extend_from_slice(data);
        }
        body.extend_from_slice(&[0; 20]); // padding
        let mut tag = b"ID3".to_vec();
        tag.extend_from_slice(&[major, 0, 0]);
        tag.extend_from_slice(&syncsafe_bytes(body.len()));
        tag.extend_from_slice(&body);
        tag
    }
    fn text(encoding: u8, bytes: &[u8]) -> Vec<u8> {
        let mut v = vec![encoding];
        v.extend_from_slice(bytes);
        v
    }
    fn utf16(text: &str) -> Vec<u8> {
        let mut v = vec![0xff, 0xfe];
        v.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        v
    }

    #[test]
    fn id3v23_text_frames_in_every_encoding() {
        let tag = id3(
            3,
            &[
                ("TIT2", text(0, b"Caf\xe9 song")),
                ("TPE1", text(1, &utf16("Zoë ♪"))),
                ("TALB", text(3, "Álbum".as_bytes())),
            ],
        );
        let found = tags_of(tag, VideoKind::Mp3);
        assert_eq!(found.title.as_deref(), Some("Café song"));
        assert_eq!(found.artist.as_deref(), Some("Zoë ♪"));
        assert_eq!(found.album.as_deref(), Some("Álbum"));
    }

    #[test]
    fn latin1_text_that_is_really_utf8_is_read_as_utf8() {
        // UTF-8 under the Latin-1 encoding byte, as many taggers write it.
        let found = tags_of(
            id3(3, &[("TIT2", text(0, "Zoë – Ünïcode".as_bytes()))]),
            VideoKind::Mp3,
        );
        assert_eq!(found.title.as_deref(), Some("Zoë – Ünïcode"));
        // Real Latin-1 is not valid UTF-8 and stays Latin-1; a Korean code page
        // cannot be recognised and is shown as Latin-1.
        assert_eq!(latin1_or_utf8(b"Caf\xe9"), "Café");
        assert_eq!(latin1_or_utf8(b"\xb8\xde\xc6\xbe 2"), "¸ÞÆ¾ 2");
        assert_eq!(latin1_or_utf8(b"plain"), "plain");
    }

    #[test]
    fn id3v24_and_v22_frames_and_the_front_cover() {
        let mut apic = vec![0];
        apic.extend_from_slice(b"image/jpeg\0");
        apic.push(0); // other
        apic.extend_from_slice(b"back\0");
        apic.extend_from_slice(b"BACK");
        let mut front = vec![0];
        front.extend_from_slice(b"image/png\0");
        front.push(3); // front cover
        front.extend_from_slice(b"\0");
        front.extend_from_slice(b"FRONT");
        let found = tags_of(
            id3(
                4,
                &[
                    ("TIT2", text(3, b"Four")),
                    ("APIC", apic.clone()),
                    ("APIC", front),
                ],
            ),
            VideoKind::Mp3,
        );
        assert_eq!(found.title.as_deref(), Some("Four"));
        assert_eq!(found.picture, Some((3, b"FRONT".to_vec())));
        // Version 2.2 uses three letter ids and a three letter picture format.
        let mut pic = vec![0];
        pic.extend_from_slice(b"JPG");
        pic.push(3);
        pic.extend_from_slice(b"\0");
        pic.extend_from_slice(b"OLD");
        let found = tags_of(
            id3(
                2,
                &[
                    ("TT2", text(0, b"Two")),
                    ("TP1", text(0, b"Band")),
                    ("PIC", pic),
                ],
            ),
            VideoKind::Mp3,
        );
        assert_eq!(found.title.as_deref(), Some("Two"));
        assert_eq!(found.artist.as_deref(), Some("Band"));
        assert_eq!(found.picture, Some((3, b"OLD".to_vec())));
        // Without a front cover the first picture counts.
        let found = tags_of(id3(3, &[("APIC", apic)]), VideoKind::Mp3);
        assert_eq!(found.picture, Some((0, b"BACK".to_vec())));
    }

    #[test]
    fn id3v1_is_the_fallback_for_old_files() {
        let mut file = vec![0xff, 0xfb, 0x90, 0x00];
        file.extend_from_slice(&[0; 300]);
        let mut tag = b"TAG".to_vec();
        for (text, len) in [("Old title", 30), ("Old artist", 30), ("Old album", 30)] {
            let mut field = text.as_bytes().to_vec();
            field.resize(len, 0);
            tag.extend_from_slice(&field);
        }
        tag.resize(128, 0);
        file.extend_from_slice(&tag);
        let found = tags_of(file, VideoKind::Mp3);
        assert_eq!(found.title.as_deref(), Some("Old title"));
        assert_eq!(found.artist.as_deref(), Some("Old artist"));
        assert_eq!(found.album.as_deref(), Some("Old album"));
    }

    #[test]
    fn mp4_ilst_gives_tags_and_cover() {
        fn mp4_box(name: &[u8], payload: &[u8]) -> Vec<u8> {
            let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
            b.extend_from_slice(name);
            b.extend_from_slice(payload);
            b
        }
        fn data(kind: u32, value: &[u8]) -> Vec<u8> {
            let mut p = kind.to_be_bytes().to_vec();
            p.extend_from_slice(&[0; 4]);
            p.extend_from_slice(value);
            mp4_box(b"data", &p)
        }
        let items = [
            mp4_box(b"\xa9nam", &data(1, b"Song")),
            mp4_box(b"\xa9ART", &data(1, "Zoë".as_bytes())),
            mp4_box(b"\xa9alb", &data(1, b"Disc")),
            mp4_box(b"covr", &data(14, b"PNGDATA")),
        ]
        .concat();
        let mut meta = vec![0; 4]; // version and flags
        meta.extend(mp4_box(b"hdlr", &[0; 24]));
        meta.extend(mp4_box(b"ilst", &items));
        let moov = mp4_box(b"moov", &mp4_box(b"udta", &mp4_box(b"meta", &meta)));
        let mut file = mp4_box(b"ftyp", b"M4A \0\0\0\0M4A isom");
        file.extend(&moov);
        file.extend(mp4_box(b"mdat", &[0; 64]));
        let found = tags_of(file, VideoKind::M4a);
        assert_eq!(found.title.as_deref(), Some("Song"));
        assert_eq!(found.artist.as_deref(), Some("Zoë"));
        assert_eq!(found.album.as_deref(), Some("Disc"));
        assert_eq!(found.picture, Some((3, b"PNGDATA".to_vec())));
    }

    #[test]
    fn flac_vorbis_comments_and_picture() {
        let mut comment = Vec::new();
        comment.extend_from_slice(&6u32.to_le_bytes());
        comment.extend_from_slice(b"vendor");
        let entries: [&[u8]; 3] = [b"TITLE=Flac song", b"artist=Band", b"ALBUM=Lossless"];
        comment.extend_from_slice(&3u32.to_le_bytes());
        for e in entries {
            comment.extend_from_slice(&(e.len() as u32).to_le_bytes());
            comment.extend_from_slice(e);
        }
        let mut picture = Vec::new();
        picture.extend_from_slice(&3u32.to_be_bytes());
        picture.extend_from_slice(&9u32.to_be_bytes());
        picture.extend_from_slice(b"image/png");
        picture.extend_from_slice(&0u32.to_be_bytes());
        picture.extend_from_slice(&[0; 16]);
        picture.extend_from_slice(&4u32.to_be_bytes());
        picture.extend_from_slice(b"DATA");
        let block = |kind: u8, last: bool, data: &[u8]| {
            let mut b = vec![kind | if last { 0x80 } else { 0 }];
            b.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
            b.extend_from_slice(data);
            b
        };
        let mut file = b"fLaC".to_vec();
        file.extend(block(0, false, &[0; 34])); // STREAMINFO
        file.extend(block(4, false, &comment));
        file.extend(block(6, true, &picture));
        let found = tags_of(file.clone(), VideoKind::Flac);
        assert_eq!(found.title.as_deref(), Some("Flac song"));
        assert_eq!(found.artist.as_deref(), Some("Band"));
        assert_eq!(found.album.as_deref(), Some("Lossless"));
        assert_eq!(found.picture, Some((3, b"DATA".to_vec())));
        // An ID3 tag in front of the stream is skipped.
        let mut tagged = id3(3, &[("TIT2", text(3, b"Outer"))]);
        tagged.extend(file);
        let found = tags_of(tagged, VideoKind::Flac);
        assert_eq!(found.title.as_deref(), Some("Outer"));
        assert_eq!(found.artist.as_deref(), Some("Band"));
    }

    #[test]
    fn damaged_or_hostile_tags_never_panic_or_read_wildly() {
        let valid = id3(
            3,
            &[
                ("TIT2", text(1, &utf16("Title"))),
                ("APIC", {
                    let mut a = vec![1];
                    a.extend_from_slice(b"image/png\0");
                    a.push(3);
                    a.extend_from_slice(&[0, 0, 0, 0]);
                    a.extend_from_slice(b"PIXELS");
                    a
                }),
            ],
        );
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for kind in [
            VideoKind::Mp3,
            VideoKind::M4a,
            VideoKind::Flac,
            VideoKind::Aac,
        ] {
            for round in 0..400 {
                let mut bytes = valid.clone();
                for _ in 0..1 + next() % 6 {
                    let at = next() as usize % bytes.len();
                    bytes[at] = next() as u8;
                }
                if round % 4 == 0 {
                    bytes.truncate(next() as usize % bytes.len());
                }
                let _ = tags_of(bytes, kind);
            }
        }
        // A size that claims far more than the file holds.
        let mut huge = b"ID3\x03\0\0\x7f\x7f\x7f\x7f".to_vec();
        huge.extend_from_slice(&[1; 64]);
        let _ = tags_of(huge, VideoKind::Mp3);
        // Control characters and long texts are tidied.
        assert_eq!(tidy("a\u{0}b\nc"), "a b c");
        assert_eq!(tidy(&"x".repeat(1000)).chars().count(), MAX_TEXT_CHARS);
    }

    fn temporary_file(name: &str, bytes: &[u8]) -> (std::path::PathBuf, File) {
        let path =
            std::env::temp_dir().join(format!("kova-audio-unit-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let file = File::open(&path).unwrap();
        (path, file)
    }

    #[test]
    fn one_file_may_only_make_the_worker_read_so_much() {
        let (path, file) = temporary_file("budget.bin", &vec![7u8; 4096]);
        let ticket = crate::security::Generation::default().next();
        let source = FileBytes {
            file: &file,
            length: 4096,
            ticket: &ticket,
            budget: Cell::new(3000),
        };
        assert!(source.read(0, 1000).is_some());
        assert!(source.read(1000, 1000).is_some());
        // 1000 of the budget are left: a bigger read is refused, a smaller one is not.
        assert!(source.read(2000, 1500).is_none());
        assert!(source.read(2000, 1000).is_some());
        assert!(source.read(3000, 1).is_none());
        // Reading past the end is refused as before.
        let fresh = FileBytes {
            budget: Cell::new(10_000),
            ..source
        };
        assert!(fresh.read(4000, 200).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reading_stops_when_the_request_is_replaced() {
        let (path, file) = temporary_file("cancel.bin", &vec![1u8; 2048]);
        let generation = crate::security::Generation::default();
        let old = generation.next();
        let source = FileBytes {
            file: &file,
            length: 2048,
            ticket: &old,
            budget: Cell::new(MAX_TOTAL_BYTES),
        };
        assert!(source.read(0, 16).is_some());
        let _newer = generation.next();
        assert!(source.read(0, 16).is_none());
        // And a whole tag is not read for a request that nobody waits for any more.
        let mut tag = b"ID3\x03\0\0\0\0\x04\0".to_vec();
        tag.extend_from_slice(&[0; 512]);
        let (tag_path, tag_file) = temporary_file("cancel-tag.mp3", &tag);
        let stale = FileBytes {
            file: &tag_file,
            length: tag.len() as u64,
            ticket: &old,
            budget: Cell::new(MAX_TOTAL_BYTES),
        };
        assert!(read_tags(&stale, VideoKind::Mp3).title.is_none());
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(tag_path);
    }

    #[test]
    fn unsynchronisation_is_undone() {
        assert_eq!(
            unsynchronise(&[0xff, 0x00, 0x01, 0xff, 0x00, 0x00]),
            [0xff, 0x01, 0xff, 0x00]
        );
        assert_eq!(syncsafe(&[0, 0, 2, 1]), Some(257));
        assert_eq!(syncsafe(&[0, 0x80, 0, 0]), None);
    }
}
