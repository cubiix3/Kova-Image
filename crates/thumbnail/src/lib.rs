//! Explorer thumbnail provider for the formats Kova Image reads.
//!
//! Explorer, the file dialogs and anything else that asks Windows for a preview
//! load this DLL into a separate, short-lived surrogate process (`dllhost.exe`),
//! hand it a file and ask for a bitmap. The decoding is the viewer's own
//! (`kova_image::decoder`), with the same limits: untrusted file, bounded size
//! and pixels, no network, nothing written.
//!
//! The provider is registered per user by `kova-image.exe --register-thumbnails`
//! (see `kova_image::windows_integration::thumbnails`). It exports only the two
//! entry points COM needs, and never lets a panic reach the host.
use kova_image::{
    decoder::{self, Decoded, Source, Target},
    error::Error as ImageError,
    security::Generation,
    windows_integration::thumbnails::CLSID,
};
use std::{
    ffi::{OsString, c_void},
    io::{self, Read, Seek, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    ptr,
    sync::Mutex,
};
use windows::{
    Win32::{
        Foundation::{
            CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED,
            S_FALSE,
        },
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
        },
        System::Com::{
            CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream, STATFLAG_DEFAULT, STATSTG,
            STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
        },
        UI::Shell::{
            IThumbnailProvider, IThumbnailProvider_Impl,
            PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl},
            WTS_ALPHATYPE, WTSAT_ARGB,
        },
    },
    core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement},
};

/// Largest preview Explorer ever asks for is 1024; refuse nothing, but never
/// build a bitmap larger than this.
const MAX_SIDE: u32 = 4096;

/// A COM stream as a reader the decoder can use.
struct StreamSource(IStream);
// SAFETY: the stream is only used on the thread that is asked for the preview,
// which is the thread that received it.
unsafe impl Send for StreamSource {}
impl Read for StreamSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut read = 0u32;
        let wanted = buffer.len().min(1 << 30) as u32;
        // SAFETY: the buffer holds at least `wanted` bytes.
        let result = unsafe {
            self.0
                .Read(buffer.as_mut_ptr().cast(), wanted, Some(&mut read))
        };
        if result.is_err() {
            return Err(io::Error::other(windows::core::Error::from(result)));
        }
        Ok(read as usize)
    }
}
impl Seek for StreamSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let (offset, origin) = match position {
            SeekFrom::Start(n) => (i64::try_from(n).map_err(io::Error::other)?, STREAM_SEEK_SET),
            SeekFrom::Current(n) => (n, STREAM_SEEK_CUR),
            SeekFrom::End(n) => (n, STREAM_SEEK_END),
        };
        let mut new = 0u64;
        // SAFETY: `new` is a valid out value.
        unsafe { self.0.Seek(offset, origin, Some(&mut new)) }.map_err(io::Error::other)?;
        Ok(new)
    }
}

#[implement(IInitializeWithStream, IThumbnailProvider)]
#[derive(Default)]
struct Provider {
    stream: Mutex<Option<IStream>>,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?.clone();
        *self.stream.lock().map_err(|_| E_UNEXPECTED)? = Some(stream);
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(
        &self,
        side: u32,
        bitmap: *mut HBITMAP,
        alpha: *mut WTS_ALPHATYPE,
    ) -> Result<()> {
        if bitmap.is_null() || alpha.is_null() {
            return Err(E_POINTER.into());
        }
        let stream = self
            .stream
            .lock()
            .map_err(|_| E_UNEXPECTED)?
            .clone()
            .ok_or(E_UNEXPECTED)?;
        // A panic must not unwind into the host.
        let made = catch_unwind(AssertUnwindSafe(|| {
            render(&stream, side.clamp(16, MAX_SIDE))
        }))
        .map_err(|_| E_FAIL)??;
        // SAFETY: both pointers were checked and are valid for writes by contract.
        unsafe {
            *bitmap = made;
            *alpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

/// Size and file extension of the stream's file. The extension tells formats
/// without a signature apart (camera RAW files from TIFF, TGA).
fn describe(stream: &IStream) -> Result<(u64, Option<String>)> {
    let mut info = STATSTG::default();
    // SAFETY: `info` is a valid out value; the name it may hold is freed below.
    unsafe { stream.Stat(&mut info, STATFLAG_DEFAULT)? };
    let name = if info.pwcsName.is_null() {
        None
    } else {
        // SAFETY: the name is a terminated string allocated by the stream.
        let text = unsafe { info.pwcsName.to_string().ok() };
        // SAFETY: allocated with CoTaskMemAlloc, freed once.
        unsafe { CoTaskMemFree(Some(info.pwcsName.0.cast())) };
        text
    };
    let extension = name.and_then(|name| {
        PathBuf::from(OsString::from(name))
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_owned)
    });
    Ok((info.cbSize, extension))
}

/// Decodes the picture in `stream` to fit `side` and returns it as a 32-bit
/// bitmap with premultiplied alpha, the form Explorer composites.
fn render(stream: &IStream, side: u32) -> Result<HBITMAP> {
    let (length, extension) = describe(stream)?;
    let ticket = Generation::default().next();
    // The decoder uses one reader at a time and starts each at the top, so they
    // may share the stream (not every stream supports `Clone`).
    let open = || -> std::result::Result<Box<dyn Source>, ImageError> {
        let copy = stream.clone();
        // SAFETY: plain COM call on a live stream.
        unsafe { copy.Seek(0, STREAM_SEEK_SET, None) }
            .map_err(|e| ImageError::Io(e.to_string()))?;
        Ok(Box::new(StreamSource(copy)))
    };
    let image: Decoded = decoder::load_stream(
        &open,
        length,
        extension.as_deref(),
        &ticket,
        Target {
            max_width: side,
            max_height: side,
        },
    )
    .map_err(|_| E_FAIL)?;
    let frame = image.frames.first().ok_or(E_FAIL)?;
    let (width, height) = (image.width, image.height);
    let pixels = (width as usize) * (height as usize);
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || frame.rgba.len() != pixels * 4
    {
        return Err(E_FAIL.into());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // A negative height makes the rows run top-down.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = ptr::null_mut();
    // SAFETY: `info` describes a 32-bit top-down bitmap; the call allocates the
    // pixels and returns a pointer to them in `bits`.
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
    if bits.is_null() {
        return Err(E_FAIL.into());
    }
    // SAFETY: the section holds `width * height` pixels of four bytes.
    let out = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), pixels * 4) };
    let premultiply = |c: u8, a: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
    for (to, from) in out.chunks_exact_mut(4).zip(frame.rgba.chunks_exact(4)) {
        let a = from[3];
        // The bitmap is blue, green, red, alpha.
        to.copy_from_slice(&[
            premultiply(from[2], a),
            premultiply(from[1], a),
            premultiply(from[0], a),
            a,
        ]);
    }
    Ok(bitmap)
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `object` was checked and is valid for a write by contract.
        unsafe { *object = ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider::default().into();
        // SAFETY: `iid` and `object` are valid pointers supplied by the caller.
        unsafe { provider.query(iid, object).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM entry point: hands out the class factory for the provider.
///
/// # Safety
/// `class`, `iid` and `object` must be valid pointers, as COM guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    class: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if object.is_null() {
        return E_POINTER;
    }
    // SAFETY: `object` was checked; `class` and `iid` are read after null checks.
    unsafe {
        *object = ptr::null_mut();
        if class.is_null() || iid.is_null() || *class != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

/// The DLL stays loaded for the life of its short-lived host process.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
