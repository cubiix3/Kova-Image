//! Local, opt-in playback check with a generated test clip (audio muted).
use kova_image::{media, security::Generation, video::Player};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--pacing")) {
        return pacing_probe(std::env::args_os().nth(2).map(PathBuf::from));
    }
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
    player.open(1, source, true, false, (1920, 1080));
    let start = Instant::now();
    let mut frames = 0;
    let mut sought = false;
    let mut seek_frame = false;
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
            seek_frame |= sought && state.paused && state.position >= state.duration * 0.5;
        }
        if frames > 3 && !sought {
            player.pause(true);
            player.seek(state.duration * 0.6);
            sought = true;
        }
        if seek_frame && state.paused && state.position >= state.duration * 0.5 {
            println!(
                "PASS: {}x{}, {:.2}s, {frames} frames, seek {:.2}s, hardware decoding={}",
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
    player.pause(true);
    player.seek(duration);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let update = rx.recv_timeout(Duration::from_secs(3))?;
        let state = update.state?;
        if state.paused && state.position >= duration - 0.01 && update.frame.is_some() {
            break;
        }
        if Instant::now() > deadline {
            return Err("exact-end seek did not display a frame".into());
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
/// Opt-in real-time regression: a 10 ms renderer/upload callback still leaves
/// enough of each 60 Hz interval for this tiny silent clip. A fixed extra
/// 16 ms sleep used to discard about a third of its frames.
fn pacing_probe(path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let path = path
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codec-pacing.mp4")
        })
        .canonicalize()?;
    let source = media::open_video(&path, &Generation::default().next())?;
    let (tx, rx) = mpsc::sync_channel(4);
    // Fixed-size measurement state: no frame/timing history is retained.
    let timings = Arc::new(Mutex::new((0u32, Duration::ZERO, Duration::ZERO)));
    let observed = timings.clone();
    let player = Player::new(move |update| {
        if update.frame.is_some() {
            let start = Instant::now();
            std::thread::sleep(Duration::from_millis(10));
            let elapsed = start.elapsed();
            if let Ok(mut observed) = observed.lock() {
                observed.0 += 1;
                observed.1 += elapsed;
                observed.2 = observed.2.max(elapsed);
            }
        }
        let _ = tx.try_send(update);
    })?;
    let start = Instant::now();
    player.open(1, source, true, false, (1920, 1080));
    let mut frames = 0;
    let duration = loop {
        let update = rx.recv_timeout(Duration::from_secs(5))?;
        let state = update.state?;
        frames += usize::from(update.frame.is_some());
        if state.ended {
            break state.duration;
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err("frame pacing validation timed out".into());
        }
    };
    player
        .stop_confirmed()
        .recv_timeout(Duration::from_secs(5))?;
    // Allow occasional scheduling misses without accepting the previous
    // roughly 38 fps ceiling. This is opt-in because host load affects timing.
    let expected = (duration * 60.0).round() as usize;
    let minimum = (duration * 50.0).ceil() as usize;
    let timings = timings.lock().map_err(|_| "Pacing measurement poisoned")?;
    println!(
        "PACING: delivered {frames}/{expected} frames in {duration:.2}s ({:.2}s wall time); simulated upload mean={:.3}ms max={:.3}ms",
        start.elapsed().as_secs_f64(),
        timings.1.as_secs_f64() * 1000.0 / f64::from(timings.0.max(1)),
        timings.2.as_secs_f64() * 1000.0,
    );
    if frames < minimum {
        return Err(format!("60 fps pacing lost too many frames: {frames}/{expected}").into());
    }
    println!("PASS: 60 fps worker pacing with simulated upload");
    Ok(())
}
fn media_duration(
    rx: &mpsc::Receiver<kova_image::video::Update>,
) -> Result<f64, Box<dyn std::error::Error>> {
    Ok(rx.recv_timeout(Duration::from_secs(3))?.state?.duration)
}
