//! Lazy, bounded media worker. Videos use bundled decoders; audio uses MF.
mod ffmpeg;
mod native;
mod output;
mod stream;
use crate::{error::Error, media::VideoSource};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

/// Condvar timeouts otherwise round to the coarse Windows timer tick, turning
/// a remaining 6 ms frame budget into a roughly 20 ms wait on some machines.
/// Request precision only while visible video is playing, balanced by Drop.
#[derive(Default)]
struct PlaybackTiming {
    active: bool,
    requested: bool,
}
impl PlaybackTiming {
    fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            // SAFETY: the successful 1 ms request is paired with the same
            // period on deactivation or worker shutdown, including errors.
            self.requested = unsafe { timeBeginPeriod(1) == 0 };
        } else if self.requested {
            self.requested = false;
            // SAFETY: balances exactly one successful timeBeginPeriod(1).
            unsafe {
                let _ = timeEndPeriod(1);
            }
        }
    }
}
impl Drop for PlaybackTiming {
    fn drop(&mut self) {
        self.set_active(false);
    }
}
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    buffer: FrameBuffer,
}
const MAX_PRESENTATION_BYTES: usize = 3840 * 2160 * 4;
/// One spare presentation buffer per engine. The Slint upload consumes a
/// Frame on the worker, then its allocation can be filled by the next render.
/// Retaining a Frame elsewhere never permits the pool to accumulate buffers.
#[derive(Clone, Default)]
struct FrameBuffer(Arc<Mutex<Vec<u8>>>);
impl FrameBuffer {
    fn acquire(&self, bytes: usize) -> Result<Vec<u8>, Error> {
        if bytes > MAX_PRESENTATION_BYTES {
            return Err(Error::MemoryBudget);
        }
        let mut pixels = self
            .0
            .lock()
            .map(|mut spare| std::mem::take(&mut *spare))
            .unwrap_or_default();
        pixels
            .try_reserve_exact(bytes.saturating_sub(pixels.len()))
            .map_err(|_| Error::MemoryBudget)?;
        // Scaling overwrites every pixel. Reusing the length avoids zeroing
        // the entire 4K buffer each frame; newly grown storage is initialized.
        pixels.resize(bytes, 0);
        Ok(pixels)
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        if self.rgba.capacity() <= MAX_PRESENTATION_BYTES
            && let Ok(mut spare) = self.buffer.0.lock()
            && spare.capacity() < self.rgba.capacity()
        {
            *spare = std::mem::take(&mut self.rgba);
        }
    }
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
    /// A video can keep playing silently when no audio endpoint is available.
    pub audio_unavailable: bool,
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
    view_w: u32,
    view_h: u32,
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
                view_w: 1920,
                view_h: 1080,
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
    pub fn open(
        &self,
        id: u64,
        source: VideoSource,
        autoplay: bool,
        looping: bool,
        view: (u32, u32),
    ) {
        self.change(|s| {
            s.id = id;
            s.source = Some(source);
            s.revision = s.revision.wrapping_add(1);
            s.paused = !autoplay;
            s.looping = looping;
            s.view_w = view.0.max(1);
            s.view_h = view.1.max(1);
            s.seek = None;
        });
    }
    /// Resize the presentation without restarting playback.
    pub fn viewport(&self, width: u32, height: u32) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut s) = lock.lock()
            && (s.view_w != width.max(1) || s.view_h != height.max(1))
        {
            s.view_w = width.max(1);
            s.view_h = height.max(1);
            wake.notify_one();
        }
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
    let mut reported_state: Option<VideoState> = None;
    // A command was just applied: tell the UI what became of it at once, even
    // when the state looks the same as the last one reported (a sound of a few
    // milliseconds ends again before the next check).
    let mut report_now = false;
    let mut opened = Instant::now();
    let mut last_report = Instant::now();
    let mut last_poll = Instant::now();
    let mut timing = PlaybackTiming::default();
    loop {
        let desired = {
            let (lock, wake) = &*shared;
            let Ok(mut s) = lock.lock() else {
                return;
            };
            timing.set_active(
                engine.is_some()
                    && s.revision == revision
                    && needs_precise_timing(
                        s.paused,
                        s.hidden,
                        ended,
                        engine.as_ref().is_some_and(|engine| engine.audio_only()),
                        s.quit,
                    ),
            );
            if s.revision == revision && s.controls == controls && !s.quit {
                if engine.is_some() {
                    // Sound has no frames to fetch: the time display needs far fewer checks.
                    let audio = engine.as_ref().is_some_and(|e| e.audio_only());
                    let interval = if s.paused || s.hidden || ended || audio {
                        Duration::from_millis(100)
                    } else {
                        Duration::from_micros(16_667)
                    };
                    // Decoding, scaling and delivering a frame belong to this
                    // interval. Sleeping for a full interval after that work
                    // needlessly drops 60 fps sources to 30–45 fps.
                    let wait = poll_delay(interval, last_poll.elapsed());
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
        last_poll = Instant::now();
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
                #[cfg(debug_assertions)]
                if delay_test_open(&shared, revision) {
                    continue;
                }
                let result = native::Runtime::new(source.kind.is_audio()).and_then(|r| {
                    runtime = Some(r);
                    native::Engine::open(
                        source,
                        desired.volume,
                        desired.muted,
                        shared.clone(),
                        desired.revision,
                    )
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
                    return Err(Error::Io("Opening the media file timed out".into()));
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
                report_now = true;
            }
            let frame = if !desired.hidden {
                e.frame(&state, desired.view_w, desired.view_h)?
            } else {
                None
            };
            ended = state.ended;
            // Pausing or ending is reported at once; a moving position only now and then.
            let flags_changed = reported_state
                .as_ref()
                .is_some_and(|r| r.paused != state.paused || r.ended != state.ended);
            if !ready
                || frame.is_some()
                || report_now
                || flags_changed
                || (reported_state.as_ref() != Some(&state)
                    && last_report.elapsed() >= Duration::from_millis(250))
            {
                report_now = false;
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
/// Deterministic visual QA for keeping the previous picture during a slow
/// native open. The wait is absent from release builds and Stop cancels it.
#[cfg(debug_assertions)]
fn delay_test_open(shared: &Arc<(Mutex<Desired>, Condvar)>, revision: u64) -> bool {
    let Some(millis) = std::env::var("KOVA_TEST_VIDEO_OPEN_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| (1..=5_000).contains(millis))
    else {
        return false;
    };
    let (lock, wake) = &**shared;
    let Ok(desired) = lock.lock() else {
        return true;
    };
    wake.wait_timeout_while(desired, Duration::from_millis(millis), |desired| {
        desired.revision == revision && !desired.quit
    })
    .map_or(true, |(desired, _)| {
        desired.revision != revision || desired.quit
    })
}
fn poll_delay(interval: Duration, work: Duration) -> Duration {
    interval.saturating_sub(work)
}
fn needs_precise_timing(paused: bool, hidden: bool, ended: bool, audio: bool, quit: bool) -> bool {
    !paused && !hidden && !ended && !audio && !quit
}
pub fn validate_dimensions(w: u32, h: u32) -> Result<(), Error> {
    if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 16_777_216 {
        return Err(Error::Io(
            "Video resolution exceeds the 16 megapixel limit".into(),
        ));
    }
    Ok(())
}
/// Fit the source inside the window without enlarging it. 3840×2160 is the
/// presentation memory ceiling, including on a larger monitor.
pub fn presentation_size(w: u32, h: u32, view_w: u32, view_h: u32) -> (u32, u32) {
    let max_w = view_w.clamp(1, 3840);
    let max_h = view_h.clamp(1, 2160);
    let scale = (f64::from(max_w) / f64::from(w.max(1)))
        .min(f64::from(max_h) / f64::from(h.max(1)))
        .min(1.0);
    (
        (f64::from(w) * scale).round().clamp(1.0, f64::from(max_w)) as u32,
        (f64::from(h) * scale).round().clamp(1.0, f64::from(max_h)) as u32,
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
    fn precise_timing_is_released_for_inactive_playback() {
        assert!(needs_precise_timing(false, false, false, false, false));
        for flag in 0..5 {
            let mut inactive = [false; 5];
            inactive[flag] = true;
            assert!(!needs_precise_timing(
                inactive[0],
                inactive[1],
                inactive[2],
                inactive[3],
                inactive[4],
            ));
        }
        let mut timing = PlaybackTiming::default();
        timing.set_active(true);
        timing.set_active(true);
        timing.set_active(false);
        assert!(!timing.active && !timing.requested);
    }

    #[test]
    fn presentation_buffer_reuses_one_allocation_across_resizes() {
        let buffer = FrameBuffer::default();
        let mut pixels = buffer.acquire(16).unwrap();
        pixels.fill(7);
        let pointer = pixels.as_ptr();
        drop(Frame {
            width: 2,
            height: 2,
            rgba: pixels,
            buffer: buffer.clone(),
        });
        let pixels = buffer.acquire(4).unwrap();
        assert_eq!(pixels.as_ptr(), pointer);
        assert_eq!(pixels, [7; 4]);
        let held = Frame {
            width: 1,
            height: 1,
            rgba: pixels,
            buffer: buffer.clone(),
        };
        // A consumer can retain a frame while another is rendered. Returning
        // them in either order keeps a single spare, with the largest capacity.
        drop(Frame {
            width: 4,
            height: 2,
            rgba: buffer.acquire(32).unwrap(),
            buffer: buffer.clone(),
        });
        drop(held);
        assert_eq!(buffer.0.lock().unwrap().capacity(), 32);
        let pixels = buffer.acquire(32).unwrap();
        assert_eq!(pixels.len(), 32);
        assert_eq!(buffer.0.lock().unwrap().capacity(), 0);
        assert!(buffer.acquire(MAX_PRESENTATION_BYTES + 1).is_err());
    }

    #[test]
    fn frame_processing_uses_the_polling_budget() {
        let interval = Duration::from_micros(16_667);
        let work = Duration::from_millis(10);
        assert_eq!(work + poll_delay(interval, work), interval);
        assert_eq!(
            poll_delay(interval, Duration::from_millis(30)),
            Duration::ZERO
        );
        assert_eq!(
            poll_delay(Duration::from_millis(100), work),
            Duration::from_millis(90)
        );
    }
    #[test]
    fn bounded_presentation_and_time() {
        assert_eq!(presentation_size(3840, 2160, 1920, 1080), (1920, 1080));
        assert_eq!(presentation_size(3840, 2160, 2560, 1440), (2560, 1440));
        assert_eq!(presentation_size(7680, 4320, 5000, 3000), (3840, 2160));
        assert_eq!(presentation_size(400, 240, 2560, 1440), (400, 240));
        assert!(validate_dimensions(u32::MAX, 9).is_err());
        assert_eq!(time_label(f64::NAN), "0:00");
        assert_eq!(time_label(3661.0), "1:01:01");
    }
}
