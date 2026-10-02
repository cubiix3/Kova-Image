//! AV1 still pictures through rav1d, the Rust port of dav1d, built without its
//! assembly routines. rav1d exposes the dav1d C interface, so this module is the
//! one small `unsafe` boundary around it: every pointer comes from a successful
//! call, a picture is released exactly once, and the decoder is closed on drop.
//!
//! That interface is `extern "C"`, and a panic that reaches such a function
//! cannot unwind: Rust aborts the whole process. rav1d does panic on damaged
//! input (a damaged test file did it within a few hundred byte flips), so the
//! copy in `vendor/rav1d` declares its entry points `extern "C-unwind"` and the
//! panic is caught here (see `scripts/vendor-rav1d.py`). The decoder runs on a
//! thread of its own, which also lets a stale request stop waiting while the
//! decoder finishes by itself, one decode at a time.
use super::yuv::{self, Layout, Planar};
use crate::{error::Error, security::Ticket};
use rav1d::include::dav1d::{
    data::Dav1dData,
    dav1d::{Dav1dContext, Dav1dSettings},
    headers::{
        DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
        DAV1D_PIXEL_LAYOUT_I444,
    },
    picture::Dav1dPicture,
};
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_default_settings, dav1d_get_picture, dav1d_open,
    dav1d_picture_unref, dav1d_send_data,
};
use std::{
    mem::MaybeUninit,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// `-EAGAIN`: the decoder wants more input or has no picture yet.
const AGAIN: i32 = -11;
/// rav1d keeps large per-frame state on the stack of the thread that decodes.
const STACK: usize = 64 * 1024 * 1024;

fn failed(call: &str, code: i32) -> Error {
    Error::Corrupted(format!("AV1 {call} failed ({code})"))
}

/// What to produce from the decoded picture.
#[derive(Clone, Copy, Debug)]
pub enum Want {
    /// Opaque RGBA. The matrix and range from the container, when it has any,
    /// replace those of the AV1 stream.
    Colour(Option<(u16, bool)>),
    /// One byte per pixel, from the luma plane of a monochrome picture.
    Alpha,
}
/// The colour description inside the AV1 stream.
#[derive(Clone, Copy, Debug)]
pub struct Coded {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub coded: Coded,
}

/// Only one picture is decoded at a time, also after the request that wanted it
/// has been replaced, so quick browsing cannot pile up decoders.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Decodes the first picture of an AV1 stream of OBUs, such as an AVIF item.
/// The stream is untrusted: sizes are checked before any plane is used.
pub fn decode(obus: Vec<u8>, want: Want, ticket: &Ticket) -> Result<Output, Error> {
    while BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        ticket.check()?;
        std::thread::sleep(Duration::from_millis(2));
    }
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("av1-decode".into())
        .stack_size(STACK)
        .spawn(move || {
            let result =
                catch_unwind(AssertUnwindSafe(|| run(&obus, want))).unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(|m| (*m).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    Err(Error::Corrupted(format!("AV1 decoder panicked: {message}")))
                });
            BUSY.store(false, Ordering::Release);
            // The receiver is gone when the request was cancelled; the result,
            // pixels included, is dropped here.
            let _ = sender.send(result);
        });
    if let Err(error) = spawned {
        BUSY.store(false, Ordering::Release);
        return Err(Error::Io(error.to_string()));
    }
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => ticket.check()?,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::Corrupted("the AV1 decoder stopped".into()));
            }
        }
    }
}

struct Context(Option<Dav1dContext>);
impl Drop for Context {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // The decoder panicked and its state cannot be trusted: closing it
            // could panic again, which would abort. It is leaked instead.
            return;
        }
        // SAFETY: the context came from `dav1d_open` and is closed once.
        unsafe { dav1d_close(NonNull::new(&mut self.0)) };
    }
}

/// A decoded picture. The planes belong to the decoder and are released on drop,
/// before the decoder itself is closed.
struct Picture {
    raw: Dav1dPicture,
    width: u32,
    height: u32,
    layout: Layout,
    depth: u8,
    coded: Coded,
    _context: Context,
}
impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: `raw` was filled by a successful `dav1d_get_picture` and is
        // released here exactly once.
        unsafe { dav1d_picture_unref(NonNull::new(&mut self.raw)) };
    }
}

fn run(obus: &[u8], want: Want) -> Result<Output, Error> {
    let picture = open(obus)?;
    let pixels = match want {
        Want::Colour(container) => {
            let (matrix, full_range) =
                container.unwrap_or((picture.coded.matrix, picture.coded.full_range));
            yuv::to_rgba(&picture, matrix, full_range)?
        }
        Want::Alpha => yuv::to_alpha(&picture, picture.coded.full_range),
    };
    Ok(Output {
        width: picture.width,
        height: picture.height,
        pixels,
        coded: picture.coded,
    })
}

fn open(obus: &[u8]) -> Result<Picture, Error> {
    let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
    // SAFETY: `dav1d_default_settings` fully initialises the structure it is given.
    let mut settings = unsafe {
        dav1d_default_settings(NonNull::new(settings.as_mut_ptr()).expect("not null"));
        settings.assume_init()
    };
    // A single thread: a panic in a worker thread would leave the decoder
    // waiting for it forever.
    settings.n_threads = 1;
    settings.max_frame_delay = 1;
    settings.frame_size_limit = crate::security::MAX_PIXELS as u32;
    let mut context = Context(None);
    // SAFETY: both pointers refer to live, properly typed values.
    let code = unsafe { dav1d_open(NonNull::new(&mut context.0), NonNull::new(&mut settings)) }.0;
    if code < 0 {
        return Err(failed("open", code));
    }
    let mut data = Dav1dData {
        data: None,
        sz: 0,
        r#ref: None,
        m: Default::default(),
    };
    // SAFETY: `data` is a valid structure; on success the call returns a buffer
    // of `obus.len()` bytes that it owns until it is passed to the decoder.
    let buffer = unsafe { dav1d_data_create(NonNull::new(&mut data), obus.len()) };
    if buffer.is_null() {
        return Err(Error::MemoryBudget);
    }
    // SAFETY: `buffer` has room for `obus.len()` bytes and does not overlap `obus`.
    unsafe { std::ptr::copy_nonoverlapping(obus.as_ptr(), buffer, obus.len()) };
    let mut raw = Dav1dPicture::default();
    loop {
        if data.sz > 0 {
            // SAFETY: the context is open and `data` holds the buffer made above.
            let code = unsafe { dav1d_send_data(context.0, NonNull::new(&mut data)) }.0;
            if code < 0 && code != AGAIN {
                return Err(failed("send", code));
            }
        }
        // SAFETY: the context is open and `raw` is a valid output structure.
        let code = unsafe { dav1d_get_picture(context.0, NonNull::new(&mut raw)) }.0;
        if code == 0 {
            break;
        }
        if code != AGAIN {
            return Err(failed("decode", code));
        }
        if data.sz == 0 {
            // All input is consumed and no picture came out.
            return Err(Error::Corrupted("AV1 stream holds no picture".into()));
        }
    }
    let layout = match raw.p.layout {
        DAV1D_PIXEL_LAYOUT_I400 => Layout::Mono,
        DAV1D_PIXEL_LAYOUT_I420 => Layout::Yuv420,
        DAV1D_PIXEL_LAYOUT_I422 => Layout::Yuv422,
        DAV1D_PIXEL_LAYOUT_I444 => Layout::Yuv444,
        _ => {
            // SAFETY: filled by the successful call above, released once.
            unsafe { dav1d_picture_unref(NonNull::new(&mut raw)) };
            return Err(Error::Unsupported);
        }
    };
    let Some(sequence) = raw.seq_hdr else {
        // SAFETY: as above.
        unsafe { dav1d_picture_unref(NonNull::new(&mut raw)) };
        return Err(failed("header", 0));
    };
    // SAFETY: the sequence header lives as long as the picture holds it.
    let sequence = unsafe { sequence.as_ref() };
    let coded = Coded {
        primaries: sequence.pri as u16,
        transfer: sequence.trc as u16,
        matrix: sequence.mtrx as u16,
        full_range: sequence.color_range != 0,
    };
    let (width, height, depth) = (raw.p.w, raw.p.h, raw.p.bpc);
    // From here the picture owns `raw`, so every return releases it.
    let picture = Picture {
        width: u32::try_from(width)
            .map_err(|_| Error::Dimensions)
            .unwrap_or(0),
        height: u32::try_from(height)
            .map_err(|_| Error::Dimensions)
            .unwrap_or(0),
        layout,
        depth: u8::try_from(depth).unwrap_or(0),
        coded,
        raw,
        _context: context,
    };
    if !matches!(picture.depth, 8 | 10 | 12) {
        return Err(Error::Unsupported);
    }
    crate::security::rgba_bytes(picture.width, picture.height)?;
    let planes = if layout == Layout::Mono { 1 } else { 3 };
    if picture.raw.data[..planes].iter().any(Option::is_none) {
        return Err(Error::Corrupted("AV1 picture lacks a plane".into()));
    }
    Ok(picture)
}

impl Picture {
    /// Bytes per sample in the decoder's planes.
    fn sample_bytes(&self) -> usize {
        if self.depth > 8 { 2 } else { 1 }
    }
    fn plane_size(&self, index: usize) -> (usize, usize) {
        yuv::plane_size(
            self.layout,
            (self.width as usize, self.height as usize),
            index,
        )
    }
}

impl Planar for Picture {
    fn size(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }
    fn layout(&self) -> Layout {
        self.layout
    }
    fn depth(&self) -> u32 {
        u32::from(self.depth)
    }
    /// One row of a plane as 16-bit samples, whatever the depth.
    fn row(&self, index: usize, y: usize, out: &mut [u16]) {
        let (w, h) = self.plane_size(index);
        assert!(y < h && out.len() >= w);
        let stride = self.raw.stride[usize::from(index > 0)];
        let base = self.raw.data[index]
            .expect("plane exists")
            .as_ptr()
            .cast::<u8>();
        // SAFETY: the decoder allocates every plane with `stride` bytes per row
        // and `h` rows, with at least `w` samples in each row. `y < h`.
        let row = unsafe { base.offset(stride * y as isize) };
        if self.sample_bytes() == 1 {
            // SAFETY: as above, one byte per sample.
            let samples = unsafe { std::slice::from_raw_parts(row, w) };
            for (o, s) in out.iter_mut().zip(samples) {
                *o = u16::from(*s);
            }
        } else {
            // SAFETY: as above, two bytes per sample; rows are 2-byte aligned
            // because the decoder aligns planes and strides to 64 bytes.
            let samples = unsafe { std::slice::from_raw_parts(row.cast::<u16>(), w) };
            out.iter_mut().zip(samples).for_each(|(o, s)| *o = *s);
        }
    }
}
