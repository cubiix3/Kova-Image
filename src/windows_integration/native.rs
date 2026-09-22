use crate::{decoder::Stamp, error::Error, security};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        Graphics::Gdi::{
            COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT,
            GetSysColor,
        },
        System::{Com::*, DataExchange::*, Memory::*},
        UI::{
            Shell::{Common::COMDLG_FILTERSPEC, *},
            WindowsAndMessaging::{
                SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                SystemParametersInfoW,
            },
        },
    },
    core::{PCWSTR, w},
};

fn failure(e: windows::core::Error) -> Error {
    Error::Io(e.to_string())
}
pub fn startup_error(error: &str) {
    let message: Vec<u16> = format!(
        "Kova Image could not start.\n{error}\n\nTry --software for the rendering fallback."
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    // SAFETY: synchronous message box borrows terminated UTF-16 text.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("Kova Image"),
            windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
        );
    }
}
fn wide(path: &Path) -> Result<Vec<u16>, Error> {
    let mut s: Vec<u16> = path.as_os_str().encode_wide().collect();
    if s.contains(&0) {
        return Err(Error::Io("Path contains a null character".into()));
    }
    s.push(0);
    Ok(s)
}
pub struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub fn new() -> Result<Self, Error> {
        // SAFETY: each shell worker owns an STA; Drop balances successful init.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(failure)?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balanced on the initializing worker thread.
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn open_image(owner: isize) -> Result<Option<PathBuf>, Error> {
    // SAFETY: COM apartment is established by the worker. Filter strings are
    // static, the dialog owns its result, and the allocated path is freed once.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(failure)?;
        dialog
            .SetOptions(
                FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM | FOS_DONTADDTORECENT,
            )
            .map_err(failure)?;
        dialog
            .SetTitle(w!("Open local media — Kova Image"))
            .map_err(failure)?;
        dialog
            .SetFileTypes(&[
                COMDLG_FILTERSPEC {
                    pszName: w!("Images"),
                    pszSpec: w!("*.jpg;*.jpeg;*.png;*.apng;*.gif;*.webp;*.bmp;*.tif;*.tiff;*.ico;*.mp4;*.m4v;*.mov;*.webm;*.mkv"),
                },
                COMDLG_FILTERSPEC {
                    pszName: w!("All files"),
                    pszSpec: w!("*.*"),
                },
            ])
            .map_err(failure)?;
        if let Err(e) = dialog.Show(Some(HWND(owner as _))) {
            if e.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(failure(e));
        }
        let item = dialog.GetResult().map_err(failure)?;
        let text = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(failure)?;
        let path = PathBuf::from(OsString::from_wide(text.as_wide()));
        CoTaskMemFree(Some(text.0.cast()));
        Ok(Some(path))
    }
}

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: this guard only exists after OpenClipboard succeeds.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
struct Global(HGLOBAL);
impl Drop for Global {
    fn drop(&mut self) {
        // SAFETY: allocation has not been transferred to the clipboard.
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}
fn clipboard_bytes(owner: isize, format: u32, bytes: &[u8]) -> Result<(), Error> {
    // SAFETY: allocate exactly bytes.len(), copy into a locked allocation,
    // unlock before publication. Clipboard assumes ownership only on success.
    unsafe {
        let allocation = Global(GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(failure)?);
        let dst = GlobalLock(allocation.0);
        if dst.is_null() {
            return Err(failure(windows::core::Error::from_thread()));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.cast(), bytes.len());
        let _ = GlobalUnlock(allocation.0);
        OpenClipboard(Some(HWND(owner as _))).map_err(failure)?;
        let _guard = Clipboard;
        EmptyClipboard().map_err(failure)?;
        SetClipboardData(format, Some(HANDLE(allocation.0.0))).map_err(failure)?;
        std::mem::forget(allocation);
    }
    Ok(())
}
pub fn copy_path(owner: isize, path: &Path) -> Result<(), Error> {
    let utf16 = wide(path)?;
    let bytes: Vec<u8> = utf16.iter().flat_map(|v| v.to_le_bytes()).collect();
    clipboard_bytes(owner, 13, &bytes) // CF_UNICODETEXT
}
pub fn copy_image(owner: isize, width: u32, height: u32, rgba: &[u8]) -> Result<(), Error> {
    let len = security::rgba_bytes(width, height)?;
    if rgba.len() != len {
        return Err(Error::Dimensions);
    }
    let mut dib = Vec::new();
    dib.try_reserve_exact(124 + len)
        .map_err(|_| Error::MemoryBudget)?;
    dib.resize(124, 0);
    // BITMAPV5HEADER, top-down BGRA with explicit alpha and sRGB masks.
    for (offset, value) in [
        (0, 124u32),
        (4, width),
        (8, (-(height as i32)) as u32),
        (16, 3),
        (20, len as u32),
        (40, 0x00ff0000),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32u16.to_le_bytes());
    for p in rgba.chunks_exact(4) {
        dib.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    clipboard_bytes(owner, 17, &dib) // CF_DIBV5
}
pub struct Desktop {
    pub high_contrast: bool,
    pub reduce_motion: bool,
    pub window: u32,
    pub window_text: u32,
    pub highlight: u32,
    pub gray: u32,
    pub hot: u32,
}
pub fn desktop() -> Desktop {
    let mut scheme = [0u16; 260];
    #[repr(C)]
    struct HighContrast {
        cb_size: u32,
        flags: u32,
        scheme: *mut u16,
    }
    let mut contrast = HighContrast {
        cb_size: std::mem::size_of::<HighContrast>() as u32,
        flags: 0,
        scheme: scheme.as_mut_ptr(),
    };
    let mut animations = 1i32;
    // SAFETY: the structures and the BOOL live for the synchronous query.
    // SPI_GETHIGHCONTRAST is 0x0042 and SPI_GETCLIENTAREAANIMATION is 0x1042.
    unsafe {
        let _ = SystemParametersInfoW(
            SYSTEM_PARAMETERS_INFO_ACTION(0x0042),
            contrast.cb_size,
            Some((&mut contrast as *mut HighContrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let _ = SystemParametersInfoW(
            SYSTEM_PARAMETERS_INFO_ACTION(0x1042),
            0,
            Some((&mut animations as *mut i32).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let color = |index| unsafe { GetSysColor(index) };
    Desktop {
        high_contrast: contrast.flags & 1 != 0,
        reduce_motion: animations == 0,
        window: color(COLOR_WINDOW),
        window_text: color(COLOR_WINDOWTEXT),
        highlight: color(COLOR_HIGHLIGHT),
        gray: color(COLOR_GRAYTEXT),
        hot: color(COLOR_HOTLIGHT),
    }
}
pub fn recycle(owner: isize, path: &Path, stamp: &Stamp) -> Result<Option<PathBuf>, Error> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err(Error::Io("Only regular image files can be recycled".into()));
    }
    if &Stamp::from_metadata(&meta) != stamp {
        return Err(Error::Changed);
    }
    let path = wide(path)?;
    // SAFETY: live terminated path; COM objects are confined to this STA. A
    // progress sink vetoes permanent deletion, including shell fallback.
    unsafe {
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None).map_err(failure)?;
        let op: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).map_err(failure)?;
        op.SetOwnerWindow(HWND(owner as _)).map_err(failure)?;
        op.SetOperationFlags(
            FOFX_RECYCLEONDELETE
                | FOF_ALLOWUNDO
                | FOF_WANTNUKEWARNING
                | FOFX_EARLYFAILURE
                | FOF_NOCONFIRMATION
                | FOF_NOERRORUI,
        )
        .map_err(failure)?;
        let outcome = std::sync::Arc::new(std::sync::Mutex::new(RecycleOutcome::default()));
        let sink: IFileOperationProgressSink = RecycleOnly {
            outcome: outcome.clone(),
        }
        .into();
        op.DeleteItem(&item, &sink).map_err(failure)?;
        op.PerformOperations().map_err(failure)?;
        if op.GetAnyOperationsAborted().map_err(failure)?.as_bool() {
            return Err(Error::Io("Recycle cancelled or unavailable".into()));
        }
        let outcome = outcome
            .lock()
            .map_err(|_| Error::Io("Windows did not confirm recycling the file".into()))?;
        if outcome.code.is_none_or(|code| code < 0) {
            return Err(Error::Io(
                "Windows did not confirm recycling the file".into(),
            ));
        }
        Ok(outcome.recycled.clone())
    }
}
pub fn restore(owner: isize, recycled: &Path, original: &Path) -> Result<(), Error> {
    if original.exists() {
        return Err(Error::Io("The original file is already there".into()));
    }
    if !recycled.is_file() {
        return Err(Error::Io("The recycled file is no longer available".into()));
    }
    let parent = original.parent().ok_or(Error::NotFound)?;
    let name = wide(Path::new(original.file_name().ok_or(Error::NotFound)?))?;
    let info = recycle_info(recycled);
    let recycled = wide(recycled)?;
    let parent = wide(parent)?;
    // SAFETY: terminated paths live for the call. The operation moves the
    // recycled file back to its original folder. Confirmations stay enabled so
    // a file that appears at the original path meanwhile is never replaced
    // silently; Windows asks the user instead.
    unsafe {
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(recycled.as_ptr()), None).map_err(failure)?;
        let folder: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(parent.as_ptr()), None).map_err(failure)?;
        let op: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).map_err(failure)?;
        op.SetOwnerWindow(HWND(owner as _)).map_err(failure)?;
        op.SetOperationFlags(FOF_NOERRORUI | FOFX_EARLYFAILURE)
            .map_err(failure)?;
        op.MoveItem(&item, &folder, PCWSTR(name.as_ptr()), None)
            .map_err(failure)?;
        op.PerformOperations().map_err(failure)?;
        if op.GetAnyOperationsAborted().map_err(failure)?.as_bool() {
            return Err(Error::Io("Restore cancelled or unavailable".into()));
        }
    }
    if !original.is_file() {
        return Err(Error::Io("Windows did not restore the file".into()));
    }
    // Windows' own restore also removes the $I metadata record; without this
    // the Recycle Bin keeps an entry for a file that is no longer there.
    if let Some(info) = info {
        let _ = std::fs::remove_file(info);
    }
    Ok(())
}
/// The `$I` metadata record paired with a `$R` payload inside `$Recycle.Bin`.
fn recycle_info(recycled: &Path) -> Option<PathBuf> {
    let name = recycled.file_name()?.to_str()?;
    let rest = name.strip_prefix("$R")?;
    let in_bin = recycled.ancestors().any(|dir| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("$Recycle.Bin"))
    });
    let info = recycled.with_file_name(format!("$I{rest}"));
    (in_bin && info.is_file()).then_some(info)
}
pub fn reveal(path: &Path) -> Result<(), Error> {
    let path = wide(path)?;
    // SAFETY: the Shell allocates a PIDL, consumed synchronously and freed once.
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut pidl, 0, None).map_err(failure)?;
        let result = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(failure);
        CoTaskMemFree(Some(pidl.cast()));
        result
    }
}
pub fn open_with(owner: isize, path: &Path) -> Result<(), Error> {
    let path = wide(path)?;
    let info = OPENASINFO {
        pcszFile: PCWSTR(path.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_EXEC,
    };
    // SAFETY: borrowed path remains alive for the modal Shell call.
    unsafe { SHOpenWithDialog(Some(HWND(owner as _)), &info).map_err(failure) }
}

#[derive(Default)]
struct RecycleOutcome {
    code: Option<i32>,
    recycled: Option<PathBuf>,
}
fn shell_path(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: the shell allocates the display name; it is copied and freed here.
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let mut len = 0usize;
        while len < 32768 && *name.0.add(len) != 0 {
            len += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(name.0, len));
        CoTaskMemFree(Some(name.0.cast()));
        let path = PathBuf::from(path);
        path.is_file().then_some(path)
    }
}
#[windows::core::implement(IFileOperationProgressSink)]
struct RecycleOnly {
    outcome: std::sync::Arc<std::sync::Mutex<RecycleOutcome>>,
}
// These method names/signatures are imposed by the COM interface.
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for RecycleOnly_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, hr: windows::core::HRESULT) -> windows::core::Result<()> {
        hr.ok()
    }
    fn PreDeleteItem(
        &self,
        flags: u32,
        _: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            Err(windows::core::Error::new(
                windows::Win32::Foundation::E_ABORT,
                "Permanent deletion is disabled",
            ))
        } else {
            Ok(())
        }
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        hr: windows::core::HRESULT,
        created: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        if let Ok(mut outcome) = self.outcome.lock() {
            outcome.code = Some(hr.0);
            if hr.is_ok() {
                outcome.recycled = created.as_ref().and_then(shell_path);
            }
        }
        hr.ok()
    }
    fn PreRenameItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: windows::core::HRESULT,
        _: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        hr.ok()
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: windows::core::HRESULT,
        _: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        hr.ok()
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: windows::core::HRESULT,
        _: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        hr.ok()
    }
    fn PreNewItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: windows::core::Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        hr: windows::core::HRESULT,
        _: windows::core::Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        hr.ok()
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}
