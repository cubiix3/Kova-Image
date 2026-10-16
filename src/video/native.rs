//! All native media resources stay on the playback worker.
use super::{Desired, Frame, VideoState, ffmpeg};
use crate::{
    error::Error,
    media::{VideoKind, VideoSource},
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{Media::MediaFoundation::*, System::Com::*},
    core::{BSTR, Interface, implement},
};

fn fail(e: windows::core::Error) -> Error {
    Error::Io(format!(
        "Windows audio playback failed: {e}. The codec may not be installed or the file may be damaged."
    ))
}

#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<AtomicU32>);
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.0.store(1, Ordering::Release);
        }
        Ok(())
    }
}

pub(super) struct Runtime(bool);
impl Runtime {
    pub fn new(audio: bool) -> Result<Self, Error> {
        // SAFETY: balanced on the dedicated MTA worker by Drop.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(fail)?;
            if audio && let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(fail(e));
            }
        }
        Ok(Self(audio))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: every Engine is dropped before its Runtime.
        unsafe {
            if self.0 {
                let _ = MFShutdown();
            }
            CoUninitialize();
        }
    }
}

pub(super) enum Engine {
    Video(ffmpeg::Engine),
    Audio(AudioEngine),
}
impl Engine {
    pub fn open(
        source: &VideoSource,
        volume: f64,
        muted: bool,
        shared: Arc<(Mutex<Desired>, Condvar)>,
        revision: u64,
    ) -> Result<Self, Error> {
        if source.kind.is_audio() {
            AudioEngine::open(source, volume, muted).map(Self::Audio)
        } else {
            ffmpeg::Engine::open(source, volume, muted, shared, revision).map(Self::Video)
        }
    }
    pub fn audio_only(&self) -> bool {
        matches!(self, Self::Audio(_))
    }
    pub fn state(&mut self) -> Result<Option<VideoState>, Error> {
        match self {
            Self::Video(e) => e.state(),
            Self::Audio(e) => e.state(),
        }
    }
    pub fn configure(
        &mut self,
        paused: bool,
        volume: f64,
        muted: bool,
        looping: bool,
        seek: Option<f64>,
    ) -> Result<(), Error> {
        match self {
            Self::Video(e) => e.configure(paused, volume, muted, looping, seek),
            Self::Audio(e) => e.configure(paused, volume, muted, looping, seek),
        }
    }
    pub fn frame(
        &mut self,
        state: &VideoState,
        width: u32,
        height: u32,
    ) -> Result<Option<Frame>, Error> {
        match self {
            Self::Video(e) => e.frame(state, width, height),
            Self::Audio(_) => Ok(None),
        }
    }
}

pub(super) struct AudioEngine {
    engine: IMFMediaEngine,
    error: Arc<AtomicU32>,
    _stream: IMFByteStream,
    kind: VideoKind,
}
impl AudioEngine {
    fn open(source: &VideoSource, volume: f64, muted: bool) -> Result<Self, Error> {
        // SAFETY: MTA/MF runtime is live; COM references own returned resources.
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 1).map_err(fail)?;
            let attrs = attrs.ok_or_else(|| Error::Io("No media attributes".into()))?;
            let error = Arc::new(AtomicU32::new(0));
            let notify: IMFMediaEngineNotify = Notify(error.clone()).into();
            attrs
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)
                .map_err(fail)?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(fail)?;
            let flags =
                (MF_MEDIA_ENGINE_DISABLE_LOCAL_PLUGINS.0 | MF_MEDIA_ENGINE_AUDIOONLY.0) as u32;
            let engine = factory.CreateInstance(flags, &attrs).map_err(fail)?;
            let stream = MFCreateMFByteStreamOnStream(&super::stream::FileStream::make(
                source.file.clone(),
                source.stamp.bytes,
            ))
            .map_err(fail)?;
            let instance = Self {
                engine,
                error,
                _stream: stream,
                kind: source.kind,
            };
            instance.engine.SetVolume(volume).map_err(fail)?;
            instance.engine.SetMuted(muted).map_err(fail)?;
            instance.engine.SetAutoPlay(false).map_err(fail)?;
            let ex: IMFMediaEngineEx = instance.engine.cast().map_err(fail)?;
            ex.SetSourceFromByteStream(&instance._stream, &BSTR::from(source.kind.hint()))
                .map_err(fail)?;
            Ok(instance)
        }
    }
    fn state(&self) -> Result<Option<VideoState>, Error> {
        // SAFETY: synchronous queries on the worker-owned live engine.
        unsafe {
            if self.error.load(Ordering::Acquire) != 0 {
                let detail = self
                    .engine
                    .GetError()
                    .map(|e| {
                        e.GetExtendedErrorCode()
                            .err()
                            .map(|e| format!("0x{:08X}", e.code().0 as u32))
                            .unwrap_or_else(|| e.GetErrorCode().to_string())
                    })
                    .unwrap_or_default();
                if detail == "0xC00D36C4" && matches!(self.kind, VideoKind::Ogg | VideoKind::Opus) {
                    return Err(Error::Io("Windows can't play Ogg Vorbis or Opus files without an extra decoder. Install the free \u{201c}Web Media Extensions\u{201d} from the Microsoft Store, then open the file again.".into()));
                }
                return Err(Error::Io(format!(
                    "Audio could not be decoded ({detail}). This codec may be unsupported, or the file is damaged."
                )));
            }
            if self.engine.GetReadyState() < 2 {
                return Ok(None);
            }
            if !self.engine.HasAudio().as_bool() {
                return Err(Error::Io("This file has no supported audio track".into()));
            }
            let duration = self.engine.GetDuration();
            if !duration.is_finite() || duration <= 0.0 || duration > 604800.0 {
                return Err(Error::Io("Invalid or unsupported media duration".into()));
            }
            Ok(Some(VideoState {
                width: 0,
                height: 0,
                duration,
                position: self.engine.GetCurrentTime().clamp(0.0, duration),
                paused: self.engine.IsPaused().as_bool(),
                ended: self.engine.IsEnded().as_bool(),
                hardware: false,
            }))
        }
    }
    fn configure(
        &self,
        paused: bool,
        volume: f64,
        muted: bool,
        looping: bool,
        seek: Option<f64>,
    ) -> Result<(), Error> {
        // SAFETY: finite control values were validated by Player.
        unsafe {
            self.engine.SetVolume(volume).map_err(fail)?;
            self.engine.SetMuted(muted).map_err(fail)?;
            self.engine.SetLoop(looping).map_err(fail)?;
            if let Some(pos) = seek {
                self.engine.SetCurrentTime(pos).map_err(fail)?;
            }
            if paused {
                self.engine.Pause().map_err(fail)?;
            } else {
                self.engine.Play().map_err(fail)?;
            }
        }
        Ok(())
    }
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        // SAFETY: Shutdown precedes stream release and MFShutdown.
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
