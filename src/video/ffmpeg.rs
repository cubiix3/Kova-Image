use super::{Desired, Frame, VideoState, output::Output};
use crate::{error::Error, media::VideoSource};
use std::{
    ffi::{CStr, c_char, c_void},
    fs::File,
    os::windows::fs::FileExt,
    ptr::NonNull,
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

#[repr(C)]
#[derive(Default)]
struct MediaInfo {
    width: i32,
    height: i32,
    audio: i32,
    duration: f64,
}
#[repr(C)]
#[derive(Default)]
struct FrameInfo {
    kind: i32,
    samples: i32,
    width: i32,
    height: i32,
    pts: f64,
    audio: *const f32,
}

unsafe extern "C" {
    fn kova_video_init();
    fn kova_video_open(
        out: *mut *mut c_void,
        opaque: *mut c_void,
        read: unsafe extern "C" fn(*mut c_void, *mut u8, i32) -> i32,
        seek: unsafe extern "C" fn(*mut c_void, i64, i32) -> i64,
        interrupt: unsafe extern "C" fn(*mut c_void) -> i32,
        info: *mut MediaInfo,
    ) -> i32;
    fn kova_video_close(video: *mut c_void);
    fn kova_video_next(video: *mut c_void, info: *mut FrameInfo) -> i32;
    fn kova_video_seek(video: *mut c_void, seconds: f64) -> i32;
    fn kova_video_present(video: *mut c_void);
    fn kova_video_render(video: *mut c_void, pixels: *mut u8, width: i32, height: i32) -> i32;
    fn kova_video_error(code: i32, buffer: *mut c_char, size: i32);
}

fn initialize() {
    static INIT: std::sync::Once = std::sync::Once::new();
    // SAFETY: this sets FFmpeg's allocation limit before any FFmpeg call, including parallel tests.
    INIT.call_once(|| unsafe { kova_video_init() });
}

fn checked(code: i32) -> Result<i32, Error> {
    if code >= 0 {
        return Ok(code);
    }
    let mut message = [0i8; 128];
    // SAFETY: fixed writable buffer; FFmpeg always terminates av_strerror output.
    unsafe {
        kova_video_error(code, message.as_mut_ptr(), message.len() as i32);
        Err(Error::Io(format!(
            "Video could not be decoded: {}",
            CStr::from_ptr(message.as_ptr()).to_string_lossy()
        )))
    }
}

struct Input {
    file: Arc<File>,
    length: u64,
    position: u64,
    shared: Arc<(Mutex<Desired>, Condvar)>,
    revision: u64,
}
impl Input {
    fn cancelled(&self) -> bool {
        self.shared
            .0
            .lock()
            .map_or(true, |s| s.quit || s.revision != self.revision)
    }
}
unsafe extern "C" fn read(opaque: *mut c_void, data: *mut u8, length: i32) -> i32 {
    // SAFETY: FFmpeg uses the stable boxed Input and supplies length writable bytes.
    let input = unsafe { &mut *opaque.cast::<Input>() };
    if input.cancelled() {
        return -1414092869;
    } // AVERROR_EXIT
    if length <= 0 || data.is_null() {
        return -22;
    }
    let bytes = unsafe { std::slice::from_raw_parts_mut(data, length as usize) };
    match input.file.seek_read(bytes, input.position) {
        Ok(0) => -541478725, // AVERROR_EOF
        Ok(n) => {
            input.position += n as u64;
            n as i32
        }
        Err(_) => -5,
    }
}
unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: i32) -> i64 {
    // SAFETY: only the demuxer calls seek on its worker-owned boxed Input.
    let input = unsafe { &mut *opaque.cast::<Input>() };
    if input.cancelled() {
        return -1414092869;
    }
    if whence & 0x10000 != 0 {
        return input.length as i64;
    } // AVSEEK_SIZE
    let base = match whence & !0x20000 {
        0 => 0,
        1 => input.position,
        2 => input.length,
        _ => return -22,
    };
    let Some(position) = base
        .checked_add_signed(offset)
        .filter(|p| *p <= input.length)
    else {
        return -22;
    };
    input.position = position;
    position as i64
}
unsafe extern "C" fn interrupt(opaque: *mut c_void) -> i32 {
    // SAFETY: FFmpeg invokes this while the boxed Input is retained by Engine.
    i32::from(unsafe { &*opaque.cast::<Input>() }.cancelled())
}

pub(super) struct Engine {
    video: NonNull<c_void>,
    _input: Box<Input>,
    audio: Option<Output>,
    state: VideoState,
    base: f64,
    clock: Instant,
    pending: Option<f64>,
    displayed: bool,
    seek_pending: bool,
    eof: bool,
    looping: bool,
    last_size: (u32, u32),
    audio_origin: f64,
}

impl Engine {
    pub fn open(
        source: &VideoSource,
        volume: f64,
        muted: bool,
        shared: Arc<(Mutex<Desired>, Condvar)>,
        revision: u64,
    ) -> Result<Self, Error> {
        initialize();
        let mut input = Box::new(Input {
            file: source.file.clone(),
            length: source.stamp.bytes,
            position: 0,
            shared,
            revision,
        });
        let mut pointer = std::ptr::null_mut();
        let mut info = MediaInfo::default();
        // SAFETY: callbacks retain no pointer past close; box is stable through Engine's lifetime.
        unsafe {
            checked(kova_video_open(
                &mut pointer,
                (&mut *input as *mut Input).cast(),
                read,
                seek,
                interrupt,
                &mut info,
            ))?;
        }
        let video = NonNull::new(pointer).ok_or_else(|| Error::Io("No video decoder".into()))?;
        let mut engine = Self {
            video,
            _input: input,
            audio: None,
            state: VideoState {
                width: info.width as u32,
                height: info.height as u32,
                duration: info.duration,
                position: 0.0,
                paused: true,
                ended: false,
                hardware: false,
            },
            base: 0.0,
            clock: Instant::now(),
            pending: None,
            displayed: false,
            seek_pending: false,
            eof: false,
            looping: false,
            last_size: (0, 0),
            audio_origin: 0.0,
        };
        super::validate_dimensions(engine.state.width, engine.state.height)?;
        if info.audio != 0 {
            engine.audio = Some(Output::new(volume, muted)?);
        }
        Ok(engine)
    }

    fn position(&self) -> Result<f64, Error> {
        let position = if self.state.paused {
            self.base
        } else if let Some(audio) = &self.audio {
            self.audio_origin + audio.playhead()?
        } else {
            self.base + self.clock.elapsed().as_secs_f64()
        };
        Ok(position.min(self.state.duration))
    }

    pub fn state(&mut self) -> Result<Option<VideoState>, Error> {
        let position = self.position()?;
        if position >= self.state.duration && !self.state.paused {
            if self.looping {
                self.seek_to(0.0)?;
            } else {
                self.base = self.state.duration;
                self.state.ended = true;
                self.state.paused = true;
                if let Some(audio) = &mut self.audio {
                    audio.controls(true, 0.0, true)?;
                }
            }
        }
        self.state.position = self.position()?;
        if let Some(audio) = &mut self.audio {
            audio.pump(self.state.paused)?;
        }
        Ok(Some(self.state.clone()))
    }

    fn seek_to(&mut self, seconds: f64) -> Result<(), Error> {
        // SAFETY: owned decoder; finite seek position was validated by Player.
        unsafe {
            checked(kova_video_seek(self.video.as_ptr(), seconds))?;
        }
        if let Some(audio) = &mut self.audio {
            audio.reset(self.state.paused)?;
        }
        self.base = seconds;
        self.clock = Instant::now();
        self.pending = None;
        self.displayed = false;
        self.seek_pending = true;
        self.eof = false;
        self.state.ended = false;
        self.last_size = (0, 0);
        self.audio_origin = seconds;
        Ok(())
    }

    pub fn configure(
        &mut self,
        paused: bool,
        volume: f64,
        muted: bool,
        looping: bool,
        seek: Option<f64>,
    ) -> Result<(), Error> {
        if let Some(seconds) = seek {
            self.seek_to(seconds)?;
        }
        if self.state.paused != paused {
            self.base = self.position()?;
            self.clock = Instant::now();
            self.state.paused = paused;
        }
        self.looping = looping;
        if let Some(audio) = &mut self.audio {
            audio.controls(paused, volume, muted)?;
        }
        Ok(())
    }

    pub fn frame(
        &mut self,
        _state: &VideoState,
        view_w: u32,
        view_h: u32,
    ) -> Result<Option<Frame>, Error> {
        let position = self.position()?;
        let mut changed = false;
        let start = Instant::now();
        if !self.state.paused || !self.displayed || self.seek_pending {
            while !self.eof {
                if let Some(pts) = self.pending {
                    if self.seek_pending && pts + 0.001 < self.base {
                        // Retain the last pre-target frame for seeks to exact EOF.
                        unsafe {
                            kova_video_present(self.video.as_ptr());
                        }
                        self.displayed = true;
                        self.pending = None;
                    } else if pts <= position + 0.001 || self.seek_pending || !self.displayed {
                        // SAFETY: one pending decoded frame is promoted to the retained display frame.
                        unsafe {
                            kova_video_present(self.video.as_ptr());
                        }
                        self.pending = None;
                        self.displayed = true;
                        self.seek_pending = false;
                        changed = true;
                        if self.state.paused {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                let mut info = FrameInfo::default();
                // SAFETY: decoder and output structure are live; audio is consumed before the next call.
                let result = unsafe { checked(kova_video_next(self.video.as_ptr(), &mut info))? };
                if result == 0 {
                    self.eof = true;
                    changed |= self.seek_pending && self.displayed;
                    self.seek_pending = false;
                    break;
                }
                if info.kind == 1 {
                    super::validate_dimensions(info.width as u32, info.height as u32)?;
                    self.state.width = info.width as u32;
                    self.state.height = info.height as u32;
                    self.pending = Some(info.pts);
                } else if info.kind == 2 {
                    if info.samples < 0 || info.samples > 96_000 || info.audio.is_null() {
                        return Err(Error::Corrupted("Invalid decoded audio frame".into()));
                    }
                    let count = info.samples as usize;
                    let skip =
                        ((self.audio_origin - info.pts).max(0.0) * 48_000.0).round() as usize;
                    let skip = skip.min(count);
                    if let Some(audio) = &mut self.audio {
                        // SAFETY: C bounds this packed stereo output to 96,000 frames.
                        let samples = unsafe { std::slice::from_raw_parts(info.audio, count * 2) };
                        if skip < count {
                            audio.push(
                                info.pts + skip as f64 / 48_000.0 - self.audio_origin,
                                &samples[skip * 2..],
                            )?;
                        }
                        audio.pump(self.state.paused)?;
                    }
                }
                if start.elapsed().as_millis() >= 15 {
                    break;
                }
            }
        }
        let size = super::presentation_size(self.state.width, self.state.height, view_w, view_h);
        if !self.displayed || (!changed && size == self.last_size) {
            return Ok(None);
        }
        let mut rgba = Vec::new();
        let bytes = size.0 as usize * size.1 as usize * 4;
        rgba.try_reserve_exact(bytes)
            .map_err(|_| Error::MemoryBudget)?;
        rgba.resize(bytes, 0);
        // SAFETY: presentation_size caps the writable packed RGBA buffer at 3840x2160.
        unsafe {
            checked(kova_video_render(
                self.video.as_ptr(),
                rgba.as_mut_ptr(),
                size.0 as i32,
                size.1 as i32,
            ))?;
        }
        self.last_size = size;
        Ok(Some(Frame {
            width: size.0,
            height: size.1,
            rgba,
        }))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.audio.take();
        // SAFETY: close frees decoder/IO before Rust drops the retained callback box and file.
        unsafe {
            kova_video_close(self.video.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Clip {
        video: *mut c_void,
        input: Option<Box<Input>>,
        info: MediaInfo,
        path: std::path::PathBuf,
        _player: super::super::Player,
    }
    impl Clip {
        fn open(bytes: &[u8], extension: &str) -> Self {
            initialize();
            let path = std::env::temp_dir().join(format!(
                "kova-codec-{}-{}.{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                extension
            ));
            std::fs::write(&path, bytes).unwrap();
            let source =
                crate::media::open_video(&path, &crate::security::Generation::default().next())
                    .unwrap();
            let player = super::super::Player::new(|_| {}).unwrap();
            let mut clip = Self {
                video: std::ptr::null_mut(),
                info: MediaInfo::default(),
                path,
                input: Some(Box::new(Input {
                    file: source.file,
                    length: source.stamp.bytes,
                    position: 0,
                    shared: player.shared.clone(),
                    revision: 0,
                })),
                _player: player,
            };
            let input = &mut **clip.input.as_mut().unwrap();
            // SAFETY: same retained-handle boundary as the player, without requiring a sound device in CI.
            unsafe {
                checked(kova_video_open(
                    &mut clip.video,
                    (input as *mut Input).cast(),
                    read,
                    seek,
                    interrupt,
                    &mut clip.info,
                ))
                .unwrap();
            }
            clip
        }
        fn next(&mut self) -> FrameInfo {
            let mut info = FrameInfo::default();
            // SAFETY: Clip retains its decoder, callbacks and writable output structure.
            unsafe {
                checked(kova_video_next(self.video, &mut info)).unwrap();
            }
            info
        }
    }
    impl Drop for Clip {
        fn drop(&mut self) {
            // SAFETY: free decoder before callback/file storage, including on assertion failure.
            unsafe {
                kova_video_close(self.video);
            }
            self.input.take();
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn bundled_decoders_play_hevc_ac3_av1_opus_and_vp9_vorbis() {
        for (bytes, extension) in [
            (
                include_bytes!("../../tests/fixtures/codec-hevc.mkv").as_slice(),
                "mkv",
            ),
            (
                include_bytes!("../../tests/fixtures/codec-av1.mkv").as_slice(),
                "mkv",
            ),
            (
                include_bytes!("../../tests/fixtures/codec-vp9.webm").as_slice(),
                "webm",
            ),
        ] {
            let mut clip = Clip::open(bytes, extension);
            assert_eq!(
                (clip.info.width, clip.info.height, clip.info.audio),
                (64, 48, 1)
            );
            let (mut videos, mut samples, mut audible) = (0, 0, false);
            loop {
                let frame = clip.next();
                match frame.kind {
                    0 => break,
                    1 => {
                        let mut rgba = vec![0u8; 64 * 48 * 4];
                        // SAFETY: one decoded frame; output matches the validated dimensions.
                        unsafe {
                            kova_video_present(clip.video);
                            checked(kova_video_render(clip.video, rgba.as_mut_ptr(), 64, 48))
                                .unwrap();
                        }
                        assert!(
                            rgba.chunks_exact(4)
                                .any(|p| p[0] > 32 || p[1] > 32 || p[2] > 32)
                        );
                        assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
                        videos += 1;
                    }
                    2 => {
                        // SAFETY: C returns at most 96,000 packed stereo samples, live until next().
                        let audio = unsafe {
                            std::slice::from_raw_parts(frame.audio, frame.samples as usize * 2)
                        };
                        assert!(audio.iter().all(|sample| sample.is_finite()));
                        audible |= audio.iter().any(|sample| sample.abs() > 0.01);
                        samples += frame.samples;
                    }
                    _ => panic!("Unexpected decoded media"),
                }
                assert!(
                    videos < 32 && samples < 100_000,
                    "Decoder did not reach EOF"
                );
            }
            assert!(videos >= 10 && samples >= 48_000);
            assert!(audible, "Decoded tone is silent");
            // SAFETY: seek on the same owned decoder and retained file handle.
            unsafe {
                checked(kova_video_seek(clip.video, 0.8)).unwrap();
            }
            let mut reached = false;
            for _ in 0..128 {
                let frame = clip.next();
                if frame.kind == 1 && frame.pts >= 0.7 {
                    reached = true;
                    break;
                }
                if frame.kind == 0 {
                    break;
                }
            }
            assert!(reached, "Seeking did not produce a frame near the target");
            assert!(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&clip.path)
                    .is_err()
            );
        }
    }

    #[test]
    fn resampled_audio_timestamps_are_continuous_through_eof() {
        let mut clip = Clip::open(
            include_bytes!("../../tests/fixtures/codec-resample.mp4"),
            "mp4",
        );
        let mut end: Option<f64> = None;
        let mut samples = 0;
        loop {
            let frame = clip.next();
            match frame.kind {
                0 => break,
                2 => {
                    if let Some(previous) = end {
                        assert!(
                            (frame.pts - previous).abs() < 1.5 / 48_000.0,
                            "Resampled audio gap: {} to {}",
                            previous,
                            frame.pts
                        );
                    }
                    end = Some(frame.pts + f64::from(frame.samples) / 48_000.0);
                    samples += frame.samples;
                }
                _ => {}
            }
            assert!(samples < 100_000);
        }
        // Reference FFmpeg PCM export at 48 kHz includes the decoder's final partial frame.
        assert_eq!(
            samples, 57_958,
            "Resampler did not drain the complete track"
        );
    }

    #[test]
    fn video_rotation_aspect_and_color_match_reference_frames() {
        for (movie, expected) in [
            (
                include_bytes!("../../tests/fixtures/codec-color.mp4").as_slice(),
                include_bytes!("../../tests/fixtures/codec-color.png").as_slice(),
            ),
            (
                include_bytes!("../../tests/fixtures/codec-rotated.mp4").as_slice(),
                include_bytes!("../../tests/fixtures/codec-rotated.png").as_slice(),
            ),
        ] {
            let mut clip = Clip::open(movie, "mp4");
            let frame = clip.next();
            assert_eq!(frame.kind, 1);
            let reference = image::load_from_memory(expected).unwrap().to_rgba8();
            assert_eq!(
                (frame.width as u32, frame.height as u32),
                reference.dimensions()
            );
            let mut rgba = vec![0; reference.as_raw().len()];
            // SAFETY: writable RGBA buffer is sized from the validated frame geometry.
            unsafe {
                kova_video_present(clip.video);
                checked(kova_video_render(
                    clip.video,
                    rgba.as_mut_ptr(),
                    frame.width,
                    frame.height,
                ))
                .unwrap();
            }
            let error: u64 = rgba
                .iter()
                .zip(reference.as_raw())
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            assert!(
                error as f64 / (rgba.len() as f64) < 2.0,
                "Reference frame mismatch: {error}"
            );
        }
    }
}
