//! Lazy, bounded local-video worker. MF owns audio/video synchronization.
mod native;
mod stream;
use crate::{error::Error, media::VideoSource};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VideoState {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub position: f64,
    pub paused: bool,
    pub ended: bool,
    pub hardware: bool,
}
pub struct Update {
    pub id: u64,
    pub state: Result<VideoState, Error>,
    pub frame: Option<Frame>,
}
#[derive(Clone)]
struct Desired {
    id: u64,
    source: Option<VideoSource>,
    revision: u64,
    controls: u64,
    paused: bool,
    muted: bool,
    volume: f64,
    looping: bool,
    hidden: bool,
    seek: Option<f64>,
    quit: bool,
    stopped: Option<std::sync::mpsc::SyncSender<()>>,
}
pub struct Player {
    shared: Arc<(Mutex<Desired>, Condvar)>,
}
impl Player {
    pub fn new(deliver: impl Fn(Update) + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new((
            Mutex::new(Desired {
                id: 0,
                source: None,
                revision: 0,
                controls: 0,
                paused: true,
                muted: false,
                volume: 0.7,
                looping: false,
                hidden: false,
                seek: None,
                quit: false,
                stopped: None,
            }),
            Condvar::new(),
        ));
        let state = shared.clone();
        std::thread::Builder::new()
            .name("video-player".into())
            .spawn(move || worker(state, deliver))?;
        Ok(Self { shared })
    }
    fn change(&self, f: impl FnOnce(&mut Desired)) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut s) = lock.lock() {
            f(&mut s);
            s.controls = s.controls.wrapping_add(1);
            wake.notify_one();
        }
    }
    pub fn open(&self, id: u64, source: VideoSource, autoplay: bool, looping: bool) {
        self.change(|s| {
            s.id = id;
            s.source = Some(source);
            s.revision = s.revision.wrapping_add(1);
            s.paused = !autoplay;
            s.looping = looping;
            s.seek = None;
        });
    }
    pub fn stop(&self) {
        self.change(|s| {
            s.source = None;
            s.revision = s.revision.wrapping_add(1);
            s.seek = None;
        });
    }
    pub fn stop_confirmed(&self) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.change(|s| {
            s.source = None;
            s.revision = s.revision.wrapping_add(1);
            s.seek = None;
            s.stopped = Some(tx);
        });
        rx
    }
    pub fn pause(&self, paused: bool) {
        self.change(|s| s.paused = paused);
    }
    pub fn seek(&self, seconds: f64) {
        if seconds.is_finite() {
            self.change(|s| s.seek = Some(seconds.max(0.0)));
        }
    }
    pub fn audio(&self, volume: f64, muted: bool) {
        if volume.is_finite() {
            self.change(|s| {
                s.volume = volume.clamp(0.0, 1.0);
                s.muted = muted;
            });
        }
    }
    pub fn hidden(&self, hidden: bool) {
        self.change(|s| s.hidden = hidden);
    }
    pub fn looping(&self, looping: bool) {
        self.change(|s| s.looping = looping);
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.change(|s| s.quit = true);
    }
}
fn worker(shared: Arc<(Mutex<Desired>, Condvar)>, deliver: impl Fn(Update)) {
    // Initialized only after the first video request, destroyed after Stop.
    let mut runtime = None;
    let mut engine: Option<native::Engine> = None;
    let (mut revision, mut controls) = (0, 0);
    let mut ready = false;
    let mut ended = false;
    let mut reported_state = None;
    let mut opened = Instant::now();
    let mut last_report = Instant::now();
    loop {
        let desired = {
            let (lock, wake) = &*shared;
            let Ok(mut s) = lock.lock() else {
                return;
            };
            if s.revision == revision && s.controls == controls && !s.quit {
                if engine.is_some() {
                    let wait = if s.paused || s.hidden || ended {
                        Duration::from_millis(100)
                    } else {
                        Duration::from_millis(16)
                    };
                    s = match wake.wait_timeout(s, wait) {
                        Ok((s, _)) => s,
                        Err(_) => return,
                    };
                } else {
                    s = match wake.wait(s) {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                }
            }
            let desired = s.clone();
            s.seek = None;
            s.stopped = None;
            desired
        };
        if desired.quit {
            break;
        }
        if desired.revision != revision {
            engine = None;
            runtime.take();
            revision = desired.revision;
            ready = false;
            ended = false;
            reported_state = None;
            opened = Instant::now();
            if let Some(source) = &desired.source {
                let result = native::Runtime::new().and_then(|r| {
                    runtime = Some(r);
                    native::Engine::open(source, desired.volume, desired.muted)
                });
                match result {
                    Ok(e) => engine = Some(e),
                    Err(e) => {
                        deliver(Update {
                            id: desired.id,
                            state: Err(e),
                            frame: None,
                        });
                        runtime.take();
                    }
                }
            }
        }
        if let Some(ack) = &desired.stopped {
            let _ = ack.try_send(());
        }
        let Some(e) = &mut engine else {
            controls = desired.controls;
            continue;
        };
        let result = (|| -> Result<(), Error> {
            let Some(state) = e.state()? else {
                controls = desired.controls;
                if opened.elapsed() > Duration::from_secs(20) {
                    return Err(Error::Io("Video opening timed out".into()));
                }
                return Ok(());
            };
            if !ready || controls != desired.controls {
                let seek = desired.seek.map(|p| p.clamp(0.0, state.duration));
                e.configure(
                    desired.paused || desired.hidden,
                    desired.volume,
                    desired.muted,
                    desired.looping,
                    seek,
                )?;
                controls = desired.controls;
            }
            let frame = if !desired.hidden {
                e.frame(&state)?
            } else {
                None
            };
            ended = state.ended;
            if !ready
                || frame.is_some()
                || (reported_state.as_ref() != Some(&state)
                    && last_report.elapsed() >= Duration::from_millis(250))
            {
                let state = e.state()?.unwrap_or(state);
                reported_state = Some(state.clone());
                deliver(Update {
                    id: desired.id,
                    state: Ok(state),
                    frame,
                });
                last_report = Instant::now();
            }
            ready = true;
            Ok(())
        })();
        if let Err(error) = result {
            engine = None;
            runtime.take();
            deliver(Update {
                id: desired.id,
                state: Err(error),
                frame: None,
            });
            controls = desired.controls;
        }
    }
    drop(engine);
    drop(runtime);
}
pub fn validate_dimensions(w: u32, h: u32) -> Result<(), Error> {
    if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 16_777_216 {
        return Err(Error::Io(
            "Video resolution exceeds the 16 megapixel limit".into(),
        ));
    }
    Ok(())
}
pub fn presentation_size(w: u32, h: u32) -> (u32, u32) {
    let scale = (1920.0 / w.max(1) as f64)
        .min(1080.0 / h.max(1) as f64)
        .min(1.0);
    (
        (w as f64 * scale).round().max(1.0) as u32,
        (h as f64 * scale).round().max(1.0) as u32,
    )
}
pub fn time_label(seconds: f64) -> String {
    let s = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_presentation_and_time() {
        assert_eq!(presentation_size(3840, 2160), (1920, 1080));
        assert_eq!(presentation_size(400, 240), (400, 240));
        assert!(validate_dimensions(u32::MAX, 9).is_err());
        assert_eq!(time_label(f64::NAN), "0:00");
        assert_eq!(time_label(3661.0), "1:01:01");
    }
}
