#![cfg(windows)]
//! Audio admission, tags and covers, with the small generated files in
//! `tests/fixtures` (`scripts/audio-fixtures.py`). Playback itself needs sound
//! codecs and is checked by `examples/audio_probe.rs`.
use kova_image::{
    audio,
    decoder::Target,
    error::Error,
    folder_navigation,
    media::{self, VideoKind},
    security::Generation,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kova-audio-{}-{}",
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
const TARGET: Target = Target {
    max_width: 1920,
    max_height: 1080,
};
const FILES: [(&str, VideoKind); 10] = [
    ("tone.mp3", VideoKind::Mp3),
    ("song.mp3", VideoKind::Mp3),
    ("song.m4a", VideoKind::M4a),
    ("tone.aac", VideoKind::Aac),
    ("tone.wav", VideoKind::Wav),
    ("click.wav", VideoKind::Wav),
    ("song.flac", VideoKind::Flac),
    ("tone.ogg", VideoKind::Ogg),
    ("tone.opus", VideoKind::Opus),
    ("tone.wma", VideoKind::Wma),
];

#[test]
fn every_audio_format_is_admitted_as_audio_and_named() {
    let ticket = Generation::default().next();
    for (name, kind) in FILES {
        let source = media::open_video(&fixture(name), &ticket).unwrap();
        assert_eq!(source.kind, kind, "{name}");
        assert!(source.kind.is_audio(), "{name}");
        assert!(
            source.audio.is_none(),
            "the loader reads tags, not admission"
        );
        assert!(!source.kind.name().is_empty() && source.kind.hint().starts_with("kova."));
        assert!(media::audio_extension(Path::new(name)));
    }
}

#[test]
fn title_artist_album_and_cover_are_read_from_id3_atoms_and_flac_blocks() {
    let ticket = Generation::default().next();
    for name in ["song.mp3", "song.m4a", "song.flac"] {
        let source = media::open_video(&fixture(name), &ticket).unwrap();
        let info = audio::read_info(
            &source.file,
            source.stamp.bytes,
            source.kind,
            &ticket,
            TARGET,
        );
        assert_eq!(info.tags.title, "Kova Song", "{name}");
        assert_eq!(info.tags.artist, "Kova Band", "{name}");
        assert_eq!(info.tags.album, "Kova Album", "{name}");
        let cover = info.cover.unwrap_or_else(|| panic!("{name} has no cover"));
        assert_eq!((cover.width, cover.height), (32, 24), "{name}");
        // The picture is the gradient of the generator: red grows to the right.
        let rgba = &cover.frames[0].rgba;
        assert!(rgba[(31) * 4] > rgba[0] + 150, "{name}");
        assert_eq!(rgba[3], 255);
    }
}

#[test]
fn files_without_tags_have_none() {
    let ticket = Generation::default().next();
    for name in [
        "tone.mp3",
        "tone.aac",
        "tone.wav",
        "click.wav",
        "tone.ogg",
        "tone.opus",
        "tone.wma",
    ] {
        let source = media::open_video(&fixture(name), &ticket).unwrap();
        let info = audio::read_info(
            &source.file,
            source.stamp.bytes,
            source.kind,
            &ticket,
            TARGET,
        );
        assert_eq!(info.tags, audio::Tags::default(), "{name}");
        assert!(info.cover.is_none(), "{name}");
    }
}

#[test]
fn the_content_decides_not_the_name() {
    let dir = Temp::new();
    let ticket = Generation::default().next();
    let bytes = |name: &str| std::fs::read(fixture(name)).unwrap();
    // A FLAC called .mp3, a tagged MP3 without extension, and a song in an .mp4.
    let source = media::open_video(&dir.write("really-flac.mp3", &bytes("song.flac")), &ticket);
    assert_eq!(source.unwrap().kind, VideoKind::Flac);
    let source = media::open_video(&dir.write("song", &bytes("song.mp3")), &ticket);
    assert_eq!(source.unwrap().kind, VideoKind::Mp3);
    let source = media::open_video(&dir.write("song.mp4", &bytes("song.m4a")), &ticket);
    assert_eq!(source.unwrap().kind, VideoKind::M4a);
    // MP3 frames with no tag are believed only with an audio extension.
    let frames = bytes("tone.mp3");
    assert!(media::open_video(&dir.write("tone.mp3", &frames), &ticket).is_ok());
    assert!(matches!(
        media::open_video(&dir.write("tone.dat", &frames), &ticket),
        Err(Error::Unsupported)
    ));
    // Text, a picture or nothing at all under an audio name is not audio.
    for (name, content) in [
        ("text.mp3", b"just some words".to_vec()),
        ("empty.flac", Vec::new()),
        ("picture.wav", bytes("rgb420.avif")),
    ] {
        assert!(
            media::open_video(&dir.write(name, &content), &ticket).is_err(),
            "{name}"
        );
    }
}

#[test]
fn an_m4a_that_points_at_other_files_is_refused() {
    fn atom(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = ((8 + data.len()) as u32).to_be_bytes().to_vec();
        out.extend(name);
        out.extend(data);
        out
    }
    let dir = Temp::new();
    let ticket = Generation::default().next();
    // A data reference entry that is not "this file".
    let mut reference = vec![0, 0, 0, 0, 0, 0, 0, 1];
    reference.extend(atom(b"url ", &[0, 0, 0, 0, b'x', 0]));
    let mut tree = atom(b"dref", &reference);
    for name in [b"dinf", b"minf", b"mdia", b"trak", b"moov"] {
        tree = atom(name, &tree);
    }
    let mut file = atom(b"ftyp", b"M4A \0\0\0\0M4A isom");
    file.extend(tree);
    assert!(media::open_video(&dir.write("outside.m4a", &file), &ticket).is_err());
}

#[test]
fn damaged_audio_is_an_error_or_a_plain_file_never_a_panic() {
    let dir = Temp::new();
    let ticket = Generation::default().next();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for (name, _) in FILES {
        let original = std::fs::read(fixture(name)).unwrap();
        for round in 0..40 {
            let mut bytes = original.clone();
            for _ in 0..1 + next() % 8 {
                let at = next() as usize % bytes.len();
                bytes[at] = next() as u8;
            }
            if round % 5 == 0 {
                bytes.truncate(next() as usize % bytes.len());
            }
            let path = dir.write(name, &bytes);
            let result = std::panic::catch_unwind(|| {
                if let Ok(source) = media::open_video(&path, &ticket) {
                    let _ = audio::read_info(
                        &source.file,
                        source.stamp.bytes,
                        source.kind,
                        &ticket,
                        TARGET,
                    );
                }
            });
            assert!(result.is_ok(), "{name} round {round}");
        }
    }
}

#[test]
fn audio_files_are_listed_with_the_other_media_of_a_folder() {
    for name in [
        "a.mp3", "b.M4A", "c.flac", "d.OGG", "e.opus", "f.wma", "g.wav", "h.aac",
    ] {
        assert!(
            folder_navigation::supported_extension(Path::new(name)),
            "{name}"
        );
        assert!(media::media_extension(Path::new(name)), "{name}");
    }
    assert!(!folder_navigation::supported_extension(Path::new(
        "notes.txt"
    )));
    // The file dialog and Open with use the same lists as the folder scan.
    assert_eq!(media::AUDIO_EXTENSIONS.len(), 10);
}
