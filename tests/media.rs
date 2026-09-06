#![cfg(windows)]
use kova_image::{error::Error, media, security::Generation};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kova-media-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn atom(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = ((8 + data.len()) as u32).to_be_bytes().to_vec();
    out.extend(name);
    out.extend(data);
    out
}
fn movie(entry: &[u8]) -> Vec<u8> {
    let mut reference = vec![0, 0, 0, 0, 0, 0, 0, 1];
    reference.extend(entry);
    let mut out = atom(b"ftyp", b"isom\0\0\0\0isom");
    let mut tree = atom(b"dref", &reference);
    for name in [b"dinf", b"minf", b"mdia", b"trak", b"moov"] {
        tree = atom(name, &tree);
    }
    out.extend(tree);
    out
}
#[test]
fn rejects_external_references_and_bad_box_lengths() {
    let dir = Temp::new();
    let ticket = Generation::default().next();
    let local = movie(&atom(b"url ", &[0, 0, 0, 1]));
    assert!(media::open_video(&dir.write("local.mp4", &local), &ticket).is_ok());
    for (i, entry) in [
        atom(b"url ", b"\0\0\0\0https://example.invalid/a\0"),
        atom(b"urn ", &[0, 0, 0, 1]),
    ]
    .iter()
    .enumerate()
    {
        assert!(
            media::open_video(
                &dir.write(&format!("external{i}.mp4"), &movie(entry)),
                &ticket
            )
            .is_err()
        );
    }
    let mut invalid = local.clone();
    invalid.extend([
        0, 0, 0, 1, b'm', b'd', b'a', b't', 255, 255, 255, 255, 255, 255, 255, 255,
    ]);
    assert!(media::open_video(&dir.write("overflow.mp4", &invalid), &ticket).is_err());
    let mut nested = atom(b"moov", &[]);
    for _ in 0..20 {
        nested = atom(b"moov", &nested);
    }
    let mut hostile = atom(b"ftyp", b"isom");
    hostile.extend(nested);
    assert!(media::open_video(&dir.write("nested.mp4", &hostile), &ticket).is_err());
    for name in [b"rmra", b"cmov"] {
        let mut reference = atom(b"ftyp", b"qt  ");
        reference.extend(atom(b"moov", &atom(name, &[])));
        assert!(media::open_video(&dir.write("reference.mov", &reference), &ticket).is_err());
    }
}
#[test]
fn local_unicode_long_paths_disappearance_and_read_lock() {
    let dir = Temp::new();
    let ticket = Generation::default().next();
    let bytes = movie(&atom(b"url ", &[0, 0, 0, 1]));
    let mut folder = dir.0.clone();
    for _ in 0..7 {
        folder.push("long path with spaces and Unicode 猫");
    }
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Bild 日本語.mp4");
    std::fs::write(&path, &bytes).unwrap();
    let source = media::open_video(&path, &ticket).unwrap();
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(std::fs::remove_file(&path).is_err());
    drop(source);
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        media::open_video(&path, &ticket),
        Err(Error::NotFound)
    ));
    assert!(media::open_video(&dir.write("broken.mp4", b"broken video"), &ticket).is_err());
    for path in [
        r"\\server\share\clip.mp4",
        r"\\?\UNC\server\share\clip.mp4",
        r"https://example.invalid/clip.mp4",
    ] {
        assert!(
            kova_image::windows_integration::require_local_file(std::path::Path::new(path))
                .is_err()
        );
    }
}
#[test]
fn mixed_navigation_and_cancelled_video_admission() {
    let dir = Temp::new();
    for name in [
        "item10.mp4",
        "item2.png",
        "item1.gif",
        "item3.MOV",
        "ignore.txt",
    ] {
        dir.write(name, b"x");
    }
    let generation = Generation::default();
    let old = generation.next();
    let ticket = generation.next();
    let path = dir.0.join("item2.png");
    let files = kova_image::folder_navigation::scan(&path, &ticket, true).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        ["item1.gif", "item2.png", "item3.MOV", "item10.mp4"]
    );
    assert!(matches!(
        media::open_video(&path, &old),
        Err(Error::Cancelled)
    ));
}
