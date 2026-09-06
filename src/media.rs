//! Shared media admission. Container names are not codec-support promises.
use crate::{decoder::Stamp, error::Error, security::Ticket};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "apng", "gif", "webp", "bmp", "tif", "tiff", "ico",
];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv"];
pub const MAX_VIDEO_BYTES: u64 = 32 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoKind {
    Mp4,
    QuickTime,
    Matroska,
}
impl VideoKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::QuickTime => "MOV",
            Self::Matroska => "WebM / MKV",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Mp4 => "kova.mp4",
            Self::QuickTime => "kova.mov",
            Self::Matroska => "kova.mkv",
        }
    }
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
pub fn video_extension(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        VIDEO_EXTENSIONS
            .iter()
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
}
pub fn probe(path: &Path) -> Result<Option<VideoKind>, Error> {
    let mut file = File::open(path)?;
    let mut header = [0; 32];
    let count = file.read(&mut header)?;
    Ok(video_magic(&header[..count]))
}
#[derive(Clone)]
pub struct VideoSource {
    pub file: Arc<File>,
    pub stamp: Stamp,
    pub kind: VideoKind,
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
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::Io("Video reparse points are not supported".into()));
        }
    }
    if !metadata.is_file() || metadata.len() > MAX_VIDEO_BYTES {
        return Err(Error::Io(
            "Video must be a regular local file no larger than 32 GiB".into(),
        ));
    }
    let mut head = [0; 32];
    let count = file.read(&mut head)?;
    let kind = video_magic(&head[..count]).ok_or(Error::Unsupported)?;
    if kind != VideoKind::Matroska {
        validate_bmff(&mut file, metadata.len(), ticket)?;
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(VideoSource {
        file: Arc::new(file),
        stamp: Stamp::from_metadata(&metadata),
        kind,
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
}
