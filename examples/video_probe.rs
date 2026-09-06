//! Local, opt-in MF integration check with a generated test clip (audio muted).
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
            .ok_or("provide a generated local video")?,
    )
    .canonicalize()?;
    let source = media::open_video(&path, &Generation::default().next())?;
    let (tx, rx) = mpsc::sync_channel(4);
    let player = Player::new(move |update| {
        let _ = tx.try_send(update);
    })?;
    player.audio(0.0, true);
    player.open(1, source, true, false);
    let start = Instant::now();
    let mut frames = 0;
    let mut sought = false;
    loop {
        let update = rx.recv_timeout(Duration::from_secs(25))?;
        let state = update.state?;
        if let Some(frame) = update.frame {
            assert_eq!(
                frame.rgba.len(),
                frame.width as usize * frame.height as usize * 4
            );
            assert!(
                frame
                    .rgba
                    .chunks_exact(4)
                    .any(|p| p[0] > 10 || p[1] > 10 || p[2] > 10)
            );
            frames += 1;
        }
        if frames > 3 && !sought {
            player.pause(true);
            player.seek(state.duration * 0.6);
            sought = true;
        }
        if sought && state.paused && state.position >= state.duration * 0.5 {
            println!(
                "PASS: {}x{}, {:.2}s, {frames} frames, seek {:.2}s, D3D hardware device={}",
                state.width, state.height, state.duration, state.position, state.hardware
            );
            break;
        }
        if start.elapsed() > Duration::from_secs(25) {
            return Err("video validation timed out".into());
        }
    }
    player.pause(false);
    // End-of-file, restart and native looping are clock behavior, not guesses
    // based on frame count. Ignore mailbox updates from before each transition.
    let duration = media_duration(&rx)?;
    player.seek((duration - 0.3).max(0.));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let state = rx.recv_timeout(Duration::from_secs(3))?.state?;
        if state.ended {
            break;
        }
        if Instant::now() > deadline {
            return Err("end-of-file timeout".into());
        }
    }
    player.looping(true);
    player.seek((duration - 0.3).max(0.));
    player.pause(false);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let state = rx.recv_timeout(Duration::from_secs(3))?.state?;
        if !state.paused && !state.ended && state.position < 1. {
            break;
        }
        if Instant::now() > deadline {
            return Err("loop timeout".into());
        }
    }
    player
        .stop_confirmed()
        .recv_timeout(Duration::from_secs(8))?;
    // The stop acknowledgement must follow release of the admitted file handle.
    drop(std::fs::OpenOptions::new().write(true).open(&path)?);
    println!("PASS: end-of-file, loop restart, confirmed resource release");
    Ok(())
}
fn media_duration(
    rx: &mpsc::Receiver<kova_image::video::Update>,
) -> Result<f64, Box<dyn std::error::Error>> {
    Ok(rx.recv_timeout(Duration::from_secs(3))?.state?.duration)
}
