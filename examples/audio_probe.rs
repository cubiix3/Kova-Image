//! Local, opt-in Media Foundation check with a generated audio file (muted).
//! `cargo run --example audio_probe -- tests/fixtures/tone.mp3`
//!
//! Plays the file, pauses, seeks, waits for the end, loops and checks that the
//! player never produces a picture and releases the file when it stops.
use kova_image::{media, security::Generation, video::Player};
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("provide a generated local audio file")?,
    )
    .canonicalize()?;
    let source = media::open_video(&path, &Generation::default().next())?;
    if !source.kind.is_audio() {
        return Err(format!("{} was not admitted as audio", source.kind.name()).into());
    }
    let kind = source.kind;
    let (tx, rx) = mpsc::sync_channel(8);
    let player = Player::new(move |update| {
        let _ = tx.try_send(update);
    })?;
    player.audio(0.0, true);
    player.open(1, source, true, false, (1920, 1080));
    let next = |what: &str| -> Result<kova_image::video::VideoState, Box<dyn std::error::Error>> {
        let update = rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| format!("no update while waiting for {what}"))?;
        if update.frame.is_some() {
            return Err("audio produced a picture".into());
        }
        Ok(update.state?)
    };

    // Playback starts and the clock runs.
    let first = next("the first state")?;
    assert_eq!(
        (first.width, first.height),
        (0, 0),
        "audio has no picture size"
    );
    assert!(first.duration > 0.0, "{}", first.duration);
    // A click sound of a tenth of a second is over before the clock can be sampled.
    let short = first.duration < 0.5;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = next("the position to advance")?;
        if state.position > 0.15 || state.ended || short {
            break;
        }
        if Instant::now() > deadline {
            return Err("position never advanced".into());
        }
    }
    // Pause and seek.
    if !short {
        player.pause(true);
        player.seek(first.duration * 0.6);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = next("the seek")?;
            if state.paused && state.position >= first.duration * 0.5 {
                break;
            }
            if Instant::now() > deadline {
                return Err("seek timeout".into());
            }
        }
    }
    // Play to the end (a long song is first moved to just before it).
    player.seek((first.duration - 0.8).max(0.));
    player.pause(false);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if next("the end")?.ended {
            break;
        }
        if Instant::now() > deadline {
            return Err("end-of-file timeout".into());
        }
    }
    // Loop: playing again from the start.
    if !short {
        player.looping(true);
        player.seek((first.duration - 0.3).max(0.));
        player.pause(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = next("the loop")?;
            if !state.paused && !state.ended && state.position < first.duration - 0.2 {
                break;
            }
            if Instant::now() > deadline {
                return Err("loop timeout".into());
            }
        }
    }
    player
        .stop_confirmed()
        .recv_timeout(Duration::from_secs(8))?;
    // The stop acknowledgement must follow release of the admitted file handle.
    drop(std::fs::OpenOptions::new().write(true).open(&path)?);
    println!(
        "PASS: {} {:.2}s, played, seeked, ended, looped, released the file",
        kind.name(),
        first.duration
    );
    Ok(())
}
