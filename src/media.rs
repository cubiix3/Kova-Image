//! Shared media admission for video and audio. Container names are not
//! codec-support promises. The names say "video" for historical reasons; an
//! audio file goes through the same admission and the same player.
use crate::{audio::AudioInfo, decoder::Stamp, error::Error, security::Ticket};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

pub use crate::format::IMAGE_EXTENSIONS;
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv"];
/// Audio files the player opens. What Windows has no codec for is reported as an
/// error when the file is played, as for video.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "wav", "flac", "ogg", "oga", "opus", "wma",
];
pub const MAX_VIDEO_BYTES: u64 = 32 * 1024 * 1024 * 1024;
/// Bytes read from the start of a file to tell what it is.
const HEAD_BYTES: usize = 64;

/// The kind of container a file has, found from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoKind {
    Mp4,
    QuickTime,
    Matroska,
    Mp3,
    M4a,
    Aac,
    Wav,
    Flac,
    Ogg,
    Opus,
    Wma,
}
impl VideoKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::QuickTime => "MOV",
            Self::Matroska => "WebM / MKV",
            Self::Mp3 => "MP3",
            Self::M4a => "M4A",
            Self::Aac => "AAC",
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Ogg => "Ogg Vorbis",
            Self::Opus => "Opus",
            Self::Wma => "WMA",
        }
    }
    /// The file name the player is told, so that Media Foundation picks the right
    /// handler for a byte stream that has no name.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Mp4 => "kova.mp4",
            Self::QuickTime => "kova.mov",
            Self::Matroska => "kova.mkv",
            Self::Mp3 => "kova.mp3",
            Self::M4a => "kova.m4a",
            Self::Aac => "kova.aac",
            Self::Wav => "kova.wav",
            Self::Flac => "kova.flac",
            Self::Ogg => "kova.ogg",
            Self::Opus => "kova.opus",
            Self::Wma => "kova.wma",
        }
    }
    /// Sound only: no picture is decoded, the player runs without a graphics device.
    pub fn is_audio(self) -> bool {
        !matches!(self, Self::Mp4 | Self::QuickTime | Self::Matroska)
    }
    /// ISO base media files carry boxes that can point at other files.
    fn is_base_media(self) -> bool {
        matches!(self, Self::Mp4 | Self::QuickTime | Self::M4a)
    }
}

fn extension_is(extension: Option<&str>, list: &[&str]) -> bool {
    extension.is_some_and(|e| list.iter().any(|x| e.eq_ignore_ascii_case(x)))
}
/// The Windows Media (ASF) header object.
const ASF_HEADER: [u8; 16] = [
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];
/// An ID3v2 tag: version 2.2 to 2.4 and a size of four 7-bit bytes.
fn id3v2_header(h: &[u8]) -> bool {
    h.len() >= 10
        && h.starts_with(b"ID3")
        && (2..=4).contains(&h[3])
        && h[4] != 0xff
        && h[6..10].iter().all(|b| b & 0x80 == 0)
}
/// The start of an MPEG audio frame (layer I to III).
fn mpeg_audio_frame(h: &[u8]) -> bool {
    let [a, b, c, ..] = h else { return false };
    *a == 0xff
        && b & 0xe0 == 0xe0
        && (b >> 3) & 3 != 1 // reserved version
        && (b >> 1) & 3 != 0 // reserved layer
        && c >> 4 != 0xf // bad bitrate
        && (c >> 2) & 3 != 3 // reserved sample rate
}
/// The start of an ADTS (raw AAC) frame.
fn adts_frame(h: &[u8]) -> bool {
    let [a, b, c, ..] = h else { return false };
    *a == 0xff && b & 0xf6 == 0xf0 && (c >> 2) & 0xf < 13
}
/// Finds an audio container. The signatures of FLAC, Ogg, WAV and ID3 are
/// strong; a bare MPEG or ADTS frame is only believed with an audio extension.
pub fn audio_magic(header: &[u8], extension: Option<&str>) -> Option<VideoKind> {
    if id3v2_header(header) {
        // A tag in front of whatever follows; the extension tells what that is.
        return Some(if extension_is(extension, &["flac"]) {
            VideoKind::Flac
        } else if extension_is(extension, &["aac"]) {
            VideoKind::Aac
        } else {
            VideoKind::Mp3
        });
    }
    if header.starts_with(b"fLaC") {
        return Some(VideoKind::Flac);
    }
    if header.starts_with(b"OggS") && header.get(4) == Some(&0) {
        return Some(if header.get(28..36) == Some(b"OpusHead".as_slice()) {
            VideoKind::Opus
        } else {
            VideoKind::Ogg
        });
    }
    if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        return Some(VideoKind::Wav);
    }
    // ASF also holds Windows Media video, which is not played.
    if header.starts_with(&ASF_HEADER) {
        return extension_is(extension, &["wma"]).then_some(VideoKind::Wma);
    }
    if extension_is(extension, AUDIO_EXTENSIONS) {
        if adts_frame(header) {
            return Some(VideoKind::Aac);
        }
        if mpeg_audio_frame(header) {
            return Some(VideoKind::Mp3);
        }
    }
    None
}
/// Video or audio, from the first bytes and the extension.
pub fn media_magic(header: &[u8], extension: Option<&str>) -> Option<VideoKind> {
    if let Some(kind) = audio_magic(header, extension) {
        return Some(kind);
    }
    let kind = video_magic(header)?;
    // A song or an audiobook in an MP4 container: by brand, or by extension.
    let audio_brand = matches!(header.get(8..12), Some(b"M4A " | b"M4B " | b"M4P "));
    if kind == VideoKind::Mp4 && (audio_brand || extension_is(extension, &["m4a", "m4b"])) {
        return Some(VideoKind::M4a);
    }
    Some(kind)
}
pub fn video_magic(header: &[u8]) -> Option<VideoKind> {
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some(VideoKind::Matroska);
    }
    let atom = header.get(4..8)?;
    if atom == b"ftyp" {
        // AVIF/HEIF are images and must not accidentally enter a video engine.
        let brand = header.get(8..12)?;
        if [b"avif", b"avis", b"heic", b"heix", b"mif1", b"msf1"].contains(&brand.try_into().ok()?)
        {
            return None;
        }
        return Some(if brand == b"qt  " {
            VideoKind::QuickTime
        } else {
            VideoKind::Mp4
        });
    }
    [b"moov", b"mdat", b"wide"]
        .contains(&atom.try_into().ok()?)
        .then_some(VideoKind::QuickTime)
}
fn path_extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|s| s.to_str())
}
pub fn video_extension(path: &Path) -> bool {
    extension_is(path_extension(path), VIDEO_EXTENSIONS)
}
pub fn audio_extension(path: &Path) -> bool {
    extension_is(path_extension(path), AUDIO_EXTENSIONS)
}
/// Whether the extension names a file the media player opens.
pub fn media_extension(path: &Path) -> bool {
    video_extension(path) || audio_extension(path)
}
pub fn probe(path: &Path) -> Result<Option<VideoKind>, Error> {
    let mut file = File::open(path)?;
    let mut header = [0; HEAD_BYTES];
    let count = file.read(&mut header)?;
    Ok(media_magic(&header[..count], path_extension(path)))
}
#[derive(Clone)]
pub struct VideoSource {
    pub file: Arc<File>,
    pub stamp: Stamp,
    pub kind: VideoKind,
    /// Title, artist and cover of an audio file, read by the loader.
    pub audio: Option<Arc<AudioInfo>>,
}
pub fn open_video(path: &Path, ticket: &Ticket) -> Result<VideoSource, Error> {
    ticket.check()?;
    #[cfg(windows)]
    crate::windows_integration::require_local_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000); // read sharing only; open reparse point itself
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    let (mut file, metadata) = {
        use std::os::windows::fs::MetadataExt;
        if crate::windows_integration::blocks_reparse(&file, metadata.file_attributes()) {
            return Err(Error::Io("Video reparse points are not supported".into()));
        }
        // A cloud placeholder is read through an ordinary handle.
        let file =
            crate::windows_integration::reopen_placeholder(file, path, metadata.file_attributes())?;
        let metadata = file.metadata()?;
        (file, metadata)
    };
    #[cfg(not(windows))]
    let mut file = file;
    if !metadata.is_file() || metadata.len() > MAX_VIDEO_BYTES {
        return Err(Error::Io(
            "Video must be a regular local file no larger than 32 GiB".into(),
        ));
    }
    let mut head = [0; HEAD_BYTES];
    let count = file.read(&mut head)?;
    let kind = media_magic(&head[..count], path_extension(path)).ok_or(Error::Unsupported)?;
    if kind.is_base_media() {
        validate_bmff(&mut file, metadata.len(), ticket)?;
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(VideoSource {
        file: Arc::new(file),
        stamp: Stamp::from_metadata(&metadata),
        kind,
        audio: None,
    })
}

// QuickTime/MP4 external data references and reference movies must never reach
// the OS resolver. Walk only container metadata, skipping mdat payload by seek.
fn validate_bmff(file: &mut File, length: u64, ticket: &Ticket) -> Result<(), Error> {
    let mut budget = 100_000usize;
    boxes(file, 0, length, 0, &mut budget, ticket)
}
fn boxes(
    file: &mut File,
    mut at: u64,
    end: u64,
    depth: u32,
    budget: &mut usize,
    ticket: &Ticket,
) -> Result<(), Error> {
    if depth > 16 {
        return Err(Error::Corrupted("Container nesting exceeds limit".into()));
    }
    while at < end {
        ticket.check()?;
        if *budget == 0 || end - at < 8 {
            return Err(Error::Corrupted("Invalid video box structure".into()));
        }
        *budget -= 1;
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap());
        let tag = &header[4..];
        let (size, header_len) = if size == 1 {
            let mut ext = [0; 8];
            file.read_exact(&mut ext)?;
            (u64::from_be_bytes(ext), 16)
        } else if size == 0 {
            (end - at, 8)
        } else {
            (u64::from(size), 8)
        };
        if size < header_len || size > end - at {
            return Err(Error::Corrupted("Invalid video box length".into()));
        }
        let body = at + header_len;
        let next = at + size;
        if tag == b"rmra" || tag == b"rmda" || tag == b"rdrf" || tag == b"cmov" {
            return Err(Error::Io(
                "Reference movies are not supported; open a self-contained local video".into(),
            ));
        }
        if tag == b"dref" {
            if next - body < 8 {
                return Err(Error::Corrupted("Invalid data references".into()));
            }
            let mut full = [0; 8];
            file.read_exact(&mut full)?;
            let entries = u32::from_be_bytes(full[4..].try_into().unwrap());
            let mut pos = body + 8;
            for _ in 0..entries {
                ticket.check()?;
                if *budget == 0 || next - pos < 12 {
                    return Err(Error::Corrupted("Invalid data reference entry".into()));
                }
                *budget -= 1;
                file.seek(SeekFrom::Start(pos))?;
                let mut entry = [0; 12];
                file.read_exact(&mut entry)?;
                let n = u64::from(u32::from_be_bytes(entry[..4].try_into().unwrap()));
                if n < 12
                    || n > next - pos
                    || &entry[4..8] != b"url "
                    || entry[8..12] != [0, 0, 0, 1]
                {
                    return Err(Error::Io("External video resources are not allowed".into()));
                }
                pos += n;
            }
            if pos != next {
                return Err(Error::Corrupted("Invalid data reference length".into()));
            }
        } else if [
            b"moov", b"trak", b"mdia", b"minf", b"dinf", b"mvex", b"moof", b"traf",
        ]
        .contains(&tag.try_into().unwrap())
        {
            boxes(file, body, next, depth + 1, budget, ticket)?;
        }
        at = next;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_magic_and_image_exclusions() {
        assert_eq!(video_magic(b"\0\0\0\x18ftypisom"), Some(VideoKind::Mp4));
        assert_eq!(
            video_magic(b"\0\0\0\x18ftypqt  "),
            Some(VideoKind::QuickTime)
        );
        assert_eq!(
            video_magic(&[0x1a, 0x45, 0xdf, 0xa3]),
            Some(VideoKind::Matroska)
        );
        assert_eq!(video_magic(b"\0\0\0\x18ftypavif"), None);
        assert_eq!(video_magic(b"GIF89a"), None);
        assert!(video_extension(Path::new("A.MP4")));
        assert!(!video_extension(Path::new("a.mp4.exe")));
    }

    fn ogg_header(first_packet: &[u8]) -> Vec<u8> {
        let mut h = b"OggS\0\x02".to_vec();
        h.resize(28, 0);
        h.extend_from_slice(first_packet);
        h
    }
    #[test]
    fn audio_containers_are_found_by_content() {
        let kind = |header: &[u8], ext: Option<&str>| media_magic(header, ext);
        assert_eq!(kind(b"fLaC\0\0\0\x22", None), Some(VideoKind::Flac));
        assert_eq!(kind(b"RIFF\x24\0\0\0WAVEfmt ", None), Some(VideoKind::Wav));
        assert_eq!(kind(&ogg_header(b"\x01vorbis"), None), Some(VideoKind::Ogg));
        assert_eq!(kind(&ogg_header(b"OpusHead"), None), Some(VideoKind::Opus));
        // An ID3v2 tag in front of MP3 (or of FLAC and AAC, which the extension tells).
        let id3 = b"ID3\x03\0\0\0\0\x01\x7f....";
        assert_eq!(kind(id3, None), Some(VideoKind::Mp3));
        assert_eq!(kind(id3, Some("flac")), Some(VideoKind::Flac));
        assert_eq!(kind(id3, Some("aac")), Some(VideoKind::Aac));
        // A tag header needs a plausible version and 7-bit sizes.
        assert_eq!(kind(b"ID3\x09\0\0\0\0\0\0", None), None);
        assert_eq!(kind(b"ID3\x03\0\0\xff\0\0\0", None), None);
        // Windows Media: ASF, but only as audio with the audio extension.
        let asf = [
            0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0, 0xaa, 0, 0x62, 0xce,
            0x6c,
        ];
        assert_eq!(kind(&asf, Some("wma")), Some(VideoKind::Wma));
        assert_eq!(kind(&asf, Some("wmv")), None);
        // A bare MPEG or ADTS frame is only believed with an audio extension.
        assert_eq!(
            kind(&[0xff, 0xfb, 0x90, 0x00], Some("mp3")),
            Some(VideoKind::Mp3)
        );
        assert_eq!(kind(&[0xff, 0xfb, 0x90, 0x00], Some("dat")), None);
        assert_eq!(kind(&[0xff, 0xfb, 0x90, 0x00], None), None);
        assert_eq!(
            kind(&[0xff, 0xf1, 0x50, 0x80], Some("aac")),
            Some(VideoKind::Aac)
        );
        // Reserved bits are not a frame.
        assert_eq!(kind(&[0xff, 0xe0, 0x90, 0x00], Some("mp3")), None);
        assert_eq!(kind(&[0xff, 0xfb, 0xf0, 0x00], Some("mp3")), None);
        // MP4 containers: a song by brand or extension, a film otherwise.
        assert_eq!(
            kind(b"\0\0\0\x18ftypM4A \0\0\0\0", None),
            Some(VideoKind::M4a)
        );
        assert_eq!(
            kind(b"\0\0\0\x18ftypisom\0\0\0\0", Some("m4a")),
            Some(VideoKind::M4a)
        );
        assert_eq!(
            kind(b"\0\0\0\x18ftypisom\0\0\0\0", Some("mp4")),
            Some(VideoKind::Mp4)
        );
        // Images and plain text are not audio.
        assert_eq!(kind(b"RIFF\x24\0\0\0WEBPVP8 ", Some("wav")), None);
        assert_eq!(kind(b"hello world, this is text", Some("mp3")), None);
        assert!(VideoKind::Flac.is_audio() && !VideoKind::Mp4.is_audio());
        assert!(audio_extension(Path::new("a.MP3")) && !audio_extension(Path::new("a.mp4")));
        assert!(media_extension(Path::new("a.mp4")) && media_extension(Path::new("a.opus")));
    }
}
