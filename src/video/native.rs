//! Media Foundation and D3D calls are confined to the playback worker.
use super::{Frame, VideoState};
use crate::{error::Error, media::VideoSource};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            Dxgi::{Common::*, IDXGISurface},
        },
        Media::MediaFoundation::*,
        System::Com::*,
    },
    core::{BSTR, Interface, implement},
};

fn fail(e: windows::core::Error) -> Error {
    Error::Io(format!(
        "Windows video playback failed: {e}. The codec may not be installed or the file may be damaged."
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
pub(super) struct Engine {
    engine: IMFMediaEngine,
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    target: Option<ID3D11Texture2D>,
    staging: Option<ID3D11Texture2D>,
    size: (u32, u32),
    error: Arc<AtomicU32>,
    // Retain stream and manager until Shutdown finishes.
    _stream: IMFByteStream,
    _manager: IMFDXGIDeviceManager,
    pub hardware: bool,
}
pub(super) struct Runtime;
impl Runtime {
    pub fn new() -> Result<Self, Error> {
        // SAFETY: balanced on this single dedicated MTA worker by Drop below.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(fail)?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(fail(e));
            }
        }
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: all Engine objects are dropped before Runtime.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
impl Engine {
    pub fn open(source: &VideoSource, volume: f64, muted: bool) -> Result<Self, Error> {
        // SAFETY: worker has an MTA/MF runtime; COM references own all returned
        // resources. Device is multithread-protected for MF's decoder threads.
        unsafe {
            let mut device = None;
            let mut context = None;
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
            let hardware = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .is_ok();
            if !hardware {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(fail)?;
            }
            let device = device.ok_or_else(|| Error::Io("No video graphics device".into()))?;
            let context = context.ok_or_else(|| Error::Io("No video graphics context".into()))?;
            let multi: ID3D11Multithread = context.cast().map_err(fail)?;
            let _ = multi.SetMultithreadProtected(true);
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(fail)?;
            let manager = manager.ok_or_else(|| Error::Io("No DXGI device manager".into()))?;
            manager.ResetDevice(&device, token).map_err(fail)?;
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4).map_err(fail)?;
            let attrs = attrs.ok_or_else(|| Error::Io("No media attributes".into()))?;
            let error = Arc::new(AtomicU32::new(0));
            let notify: IMFMediaEngineNotify = Notify(error.clone()).into();
            attrs
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)
                .map_err(fail)?;
            attrs
                .SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)
                .map_err(fail)?;
            attrs
                .SetUINT32(
                    &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
                )
                .map_err(fail)?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(fail)?;
            let engine = factory
                .CreateInstance(MF_MEDIA_ENGINE_DISABLE_LOCAL_PLUGINS.0 as u32, &attrs)
                .map_err(fail)?;
            let stream = MFCreateMFByteStreamOnStream(&super::stream::FileStream::make(
                source.file.clone(),
                source.stamp.bytes,
            ))
            .map_err(fail)?;
            let instance = Self {
                engine,
                context,
                device,
                target: None,
                staging: None,
                size: (0, 0),
                error,
                _stream: stream,
                _manager: manager,
                hardware,
            };
            instance.engine.SetVolume(volume).map_err(fail)?;
            instance.engine.SetMuted(muted).map_err(fail)?;
            instance.engine.SetAutoPlay(false).map_err(fail)?;
            let ex: IMFMediaEngineEx = instance.engine.cast().map_err(fail)?;
            // A synthetic local type hint, never a user URL. The byte stream
            // owns the admitted file; BMFF references were rejected beforehand.
            ex.SetSourceFromByteStream(&instance._stream, &BSTR::from(source.kind.hint()))
                .map_err(fail)?;
            Ok(instance)
        }
    }
    pub fn state(&self) -> Result<Option<VideoState>, Error> {
        // SAFETY: synchronous queries on the worker-owned, live engine.
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
                return Err(Error::Io(format!(
                    "Video could not be decoded ({detail}). This codec may be unsupported, or the file is damaged."
                )));
            }
            if self.engine.GetReadyState() < 2 {
                return Ok(None);
            }
            if !self.engine.HasVideo().as_bool() {
                return Err(Error::Io("This file has no supported video track".into()));
            }
            let (mut w, mut h) = (0, 0);
            self.engine
                .GetNativeVideoSize(Some(&mut w), Some(&mut h))
                .map_err(fail)?;
            super::validate_dimensions(w, h)?;
            let duration = self.engine.GetDuration();
            if !duration.is_finite() || duration <= 0.0 || duration > 7.0 * 24.0 * 3600.0 {
                return Err(Error::Io("Invalid or unsupported video duration".into()));
            }
            Ok(Some(VideoState {
                width: w,
                height: h,
                duration,
                position: self.engine.GetCurrentTime().clamp(0.0, duration),
                paused: self.engine.IsPaused().as_bool(),
                ended: self.engine.IsEnded().as_bool(),
                hardware: self.hardware,
            }))
        }
    }
    pub fn configure(
        &self,
        paused: bool,
        volume: f64,
        muted: bool,
        looping: bool,
        seek: Option<f64>,
    ) -> Result<(), Error> {
        // SAFETY: validated finite control values, called only on the worker.
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
    pub fn frame(&mut self, state: &VideoState) -> Result<Option<Frame>, Error> {
        // SAFETY: textures are worker-owned and match the validated dimensions.
        // Map/Unmap bound the lifetime of the readback pointer; each row respects
        // RowPitch. Only the bounded presentation image is copied to the UI.
        unsafe {
            let mut pts = 0;
            let result = (Interface::vtable(&self.engine).OnVideoStreamTick)(
                Interface::as_raw(&self.engine),
                &mut pts,
            );
            if result == S_FALSE {
                return Ok(None);
            }
            result.ok().map_err(fail)?;
            let (w, h) = super::presentation_size(state.width, state.height);
            if self.size != (w, h) {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    ..Default::default()
                };
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut self.target))
                    .map_err(fail)?;
                let desc = D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..desc
                };
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut self.staging))
                    .map_err(fail)?;
                self.size = (w, h);
            }
            let target = self
                .target
                .as_ref()
                .ok_or_else(|| Error::Io("No video surface".into()))?;
            let staging = self
                .staging
                .as_ref()
                .ok_or_else(|| Error::Io("No video readback surface".into()))?;
            let surface: IDXGISurface = target.cast().map_err(fail)?;
            self.engine
                .TransferVideoFrame(
                    &surface,
                    None,
                    &RECT {
                        left: 0,
                        top: 0,
                        right: w as i32,
                        bottom: h as i32,
                    },
                    None,
                )
                .map_err(fail)?;
            self.context.CopyResource(staging, target);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(fail)?;
            let row = w as usize * 4;
            if mapped.pData.is_null() || (mapped.RowPitch as usize) < row {
                self.context.Unmap(staging, 0);
                return Err(Error::Io("Invalid video surface stride".into()));
            }
            let mut rgba = Vec::new();
            if rgba.try_reserve_exact(row * h as usize).is_err() {
                self.context.Unmap(staging, 0);
                return Err(Error::MemoryBudget);
            }
            rgba.resize(row * h as usize, 0);
            for y in 0..h as usize {
                let src = std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    row,
                );
                let dst = &mut rgba[y * row..(y + 1) * row];
                for (src, dst) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                    dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
                }
            }
            self.context.Unmap(staging, 0);
            Ok(Some(Frame {
                width: w,
                height: h,
                rgba,
            }))
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: Shutdown precedes stream/device release and MFShutdown.
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
