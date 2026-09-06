//! A seekable, read-only COM stream over an already-admitted file handle.
//! No URL resolver and no reopening the path after admission.
use std::{
    ffi::c_void,
    fs::File,
    os::windows::fs::FileExt,
    sync::{Arc, Mutex},
};
use windows::{
    Win32::{Foundation::*, System::Com::*},
    core::{HRESULT, Ref, Result, implement},
};

#[implement(IStream)]
pub(super) struct FileStream {
    file: Arc<File>,
    position: Mutex<u64>,
    length: u64,
}
impl FileStream {
    pub fn make(file: Arc<File>, length: u64) -> IStream {
        Self {
            file,
            position: Mutex::new(0),
            length,
        }
        .into()
    }
}
impl ISequentialStream_Impl for FileStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, read: *mut u32) -> HRESULT {
        if pv.is_null() && cb != 0 {
            return E_POINTER;
        }
        let Ok(mut position) = self.position.lock() else {
            return E_FAIL;
        };
        // SAFETY: ISequentialStream's caller supplies cb writable bytes and an
        // optional valid count pointer. MF is the sole caller, never image data.
        unsafe {
            if !read.is_null() {
                *read = 0;
            }
            if cb == 0 {
                return S_OK;
            }
            let data = std::slice::from_raw_parts_mut(pv.cast::<u8>(), cb as usize);
            match self.file.seek_read(data, *position) {
                Ok(n) => {
                    *position += n as u64;
                    if !read.is_null() {
                        *read = n as u32;
                    }
                    if n == cb as usize { S_OK } else { S_FALSE }
                }
                Err(_) => E_FAIL,
            }
        }
    }
    fn Write(&self, _: *const c_void, _: u32, _: *mut u32) -> HRESULT {
        E_ACCESSDENIED
    }
}
impl IStream_Impl for FileStream_Impl {
    fn Seek(&self, offset: i64, origin: STREAM_SEEK, new_position: *mut u64) -> Result<()> {
        let mut pos = self
            .position
            .lock()
            .map_err(|_| windows::core::Error::from(E_FAIL))?;
        let base = match origin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => *pos,
            STREAM_SEEK_END => self.length,
            _ => return Err(E_INVALIDARG.into()),
        };
        let next = base
            .checked_add_signed(offset)
            .ok_or(windows::core::Error::from(E_INVALIDARG))?;
        if next > self.length {
            return Err(E_INVALIDARG.into());
        }
        *pos = next;
        // SAFETY: optional output pointer follows IStream::Seek's COM contract.
        if !new_position.is_null() {
            unsafe {
                *new_position = next;
            }
        }
        Ok(())
    }
    fn SetSize(&self, _: u64) -> Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn CopyTo(&self, _: Ref<IStream>, _: u64, _: *mut u64, _: *mut u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Commit(&self, _: &STGC) -> Result<()> {
        Ok(())
    }
    fn Revert(&self) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Stat(&self, out: *mut STATSTG, _: &STATFLAG) -> Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: writes one caller-owned STATSTG; no string allocation to free.
        unsafe {
            *out = STATSTG {
                r#type: STGTY_STREAM.0 as u32,
                cbSize: self.length,
                grfMode: STGM_READ,
                ..Default::default()
            };
        }
        Ok(())
    }
    fn Clone(&self) -> Result<IStream> {
        let pos = *self
            .position
            .lock()
            .map_err(|_| windows::core::Error::from(E_FAIL))?;
        Ok(FileStream {
            file: self.file.clone(),
            position: Mutex::new(pos),
            length: self.length,
        }
        .into())
    }
}
