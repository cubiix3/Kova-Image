//! Local, opt-in check of many audio files: admission, tags and cover, no playback.
//! `cargo run --release --example audio_scan -- list.txt` where the list holds one
//! path per line. Prints a summary and the files that were refused or were slow.
use kova_image::{audio, decoder::Target, media, security::Generation};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let list = std::fs::read_to_string(std::env::args_os().nth(1).ok_or("provide a list file")?)?;
    let target = Target {
        max_width: 1920,
        max_height: 1080,
    };
    let (mut files, mut refused, mut tagged, mut covers) = (0, 0, 0, 0);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut slowest = (0.0f64, PathBuf::new());
    // Lists written by Windows tools often start with a byte order mark.
    let list = list.trim_start_matches('\u{feff}');
    for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let path = PathBuf::from(line);
        files += 1;
        let ticket = Generation::default().next();
        let start = Instant::now();
        let source = match media::open_video(&path, &ticket) {
            Ok(source) if source.kind.is_audio() => source,
            Ok(source) => {
                refused += 1;
                println!("NOT AUDIO ({}): {line}", source.kind.name());
                continue;
            }
            Err(error) => {
                refused += 1;
                println!("REFUSED ({error}): {line}");
                continue;
            }
        };
        *kinds.entry(source.kind.name()).or_default() += 1;
        let info = audio::read_info(
            &source.file,
            source.stamp.bytes,
            source.kind,
            &ticket,
            target,
        );
        if !info.tags.title.is_empty() || !info.tags.artist.is_empty() {
            tagged += 1;
            println!(
                "TAGS {:?} / {:?} / {:?}: {line}",
                info.tags.title, info.tags.artist, info.tags.album
            );
        }
        if let Some(cover) = &info.cover {
            covers += 1;
            assert!(cover.width > 0 && cover.height > 0);
            println!("COVER {}x{}: {line}", cover.width, cover.height);
        }
        let seconds = start.elapsed().as_secs_f64();
        if seconds > slowest.0 {
            slowest = (seconds, path);
        }
    }
    println!(
        "{files} files, {refused} refused, {tagged} with title or artist, {covers} with a cover"
    );
    println!("kinds: {kinds:?}");
    println!(
        "slowest: {:.0} ms ({})",
        slowest.0 * 1000.0,
        slowest.1.display()
    );
    Ok(())
}
