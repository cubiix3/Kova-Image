//! Registration of the Explorer thumbnail provider (`kova_thumbnails.dll`), so
//! that Explorer and the file dialogs show previews for formats Windows cannot
//! preview by itself, such as WebP, AVIF, SVG or JPEG XL.
//!
//! Like the Open with registration it is explicit, per user, idempotent and
//! reversible, and it only writes Kova's own keys under HKCU:
//! - the class of the provider, with the path of the DLL, and
//! - the thumbnail handler key of a file extension, only for extensions that
//!   have no working preview handler of another program. Windows' generic image
//!   handler does not count as one, because it only works when a codec for the
//!   format is installed, and that is the situation this exists for. Neither
//!   does a handler whose DLL is gone, such as the leftover of an uninstalled
//!   program: Explorer fails to load it and shows no preview.
//!   The one exception is [`REPLACED`], where Kova's decoder covers what the
//!   usual third-party provider shows and the user asked for Kova's previews.
//!   HKCU wins over the machine-wide key, so no administrator rights are needed
//!   and unregistering brings the other provider back.
//!
//! Unregistering removes the class and exactly those extension keys that point
//! at it.
use super::associations::{set, wide};
use crate::{error::Error, format};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{Foundation::*, System::Registry::*, UI::Shell::*},
    core::{GUID, PCWSTR},
};

/// Class id of the thumbnail provider. It is part of what is stored in the
/// registry, so it must never change.
pub const CLSID: GUID = GUID::from_u128(0x7c1d3a52_9e84_4b6f_8a07_52f0d9c3b1e6);
const CLSID_TEXT: &str = "{7C1D3A52-9E84-4B6F-8A07-52F0D9C3B1E6}";
/// `IThumbnailProvider`: the key under a file type that names its preview handler.
const HANDLER: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";
/// Windows' own handler for any image type with a codec installed.
const WINDOWS_IMAGE_HANDLER: &str = "{C7657C4A-9F68-40FA-A4DF-96BC08EB3551}";
pub const DLL_NAME: &str = "kova_thumbnails.dll";

/// Extensions where Kova's previews replace those of another program that still
/// works. DDS is the one case: game and texture tools register providers for it,
/// and `codecs/dds.rs` reads the same formats (BC1 to BC7, uncompressed).
const REPLACED: &[&str] = &["dds"];

/// Extensions of the formats Kova Image reads that Windows does not preview on
/// its own installation. JPEG, PNG, GIF, BMP, TIFF and ICO are left to Windows.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    format::IMAGE_EXTENSIONS.iter().copied().filter(|e| {
        !matches!(
            *e,
            "jpg" | "jpeg" | "jpe" | "png" | "apng" | "gif" | "bmp" | "tif" | "tiff" | "ico"
        )
    })
}

/// The default text value of a key, or `None` when there is none.
fn read_default(root: HKEY, path: &str) -> Option<String> {
    let path = wide(path.as_ref()).ok()?;
    let mut size = 0u32;
    // SAFETY: terminated path, size query only.
    let first = unsafe {
        RegGetValueW(
            root,
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if first != ERROR_SUCCESS {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer is `size` bytes long.
    let second = unsafe {
        RegGetValueW(
            root,
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    // The reported size can include more than the string when Windows expanded
    // environment variables, so the text ends at its own terminator.
    (second == ERROR_SUCCESS).then(|| {
        let units = &buffer[..(size as usize / 2).min(buffer.len())];
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    })
}
/// As Windows resolves it: the current user's key over the machine's.
fn default_value(path: &str) -> Option<String> {
    read_default(HKEY_CLASSES_ROOT, path)
}
fn hkcu_default(path: &str) -> Option<String> {
    read_default(HKEY_CURRENT_USER, path)
}

/// Whether a key exists, whatever it holds.
fn key_exists(root: HKEY, path: &str) -> bool {
    let Ok(path) = wide(path.as_ref()) else {
        return false;
    };
    let mut key = HKEY::default();
    // SAFETY: terminated path; a handle that was opened is closed again.
    unsafe {
        let found =
            RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) == ERROR_SUCCESS;
        if found {
            let _ = RegCloseKey(key);
        }
        found
    }
}

/// Replaces `%NAME%` with the value of the environment variable, as registry
/// paths of Shell extensions often use `%SystemRoot%` or `%ProgramFiles%`.
fn expand(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after
            .find('%')
            .and_then(|end| std::env::var(&after[..end]).ok().map(|v| (end, v)))
        {
            Some((end, value)) => {
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether Explorer can no longer load the handler class `clsid`: its class is
/// not registered, or its DLL does not exist. That is what an uninstalled
/// program leaves behind. A handler that merely lives somewhere unusual (a
/// packaged app, a server of another kind) counts as alive.
fn dead(clsid: &str) -> bool {
    match default_value(&format!(r"CLSID\{clsid}\InprocServer32")) {
        Some(path) => {
            let path = expand(path.trim().trim_matches('"'));
            !Path::new(&path).is_file()
        }
        None => {
            !key_exists(HKEY_CLASSES_ROOT, &format!(r"CLSID\{clsid}"))
                && !key_exists(
                    HKEY_CLASSES_ROOT,
                    &format!(r"PackagedCom\ClassIndex\{clsid}"),
                )
        }
    }
}

/// Whether the preview handler of `.ext` may be set to Kova's.
fn available(ext: &str) -> bool {
    REPLACED.contains(&ext)
        || [
            format!(r".{ext}\ShellEx\{HANDLER}"),
            format!(r"SystemFileAssociations\.{ext}\ShellEx\{HANDLER}"),
        ]
        .iter()
        .filter_map(|key| default_value(key))
        .filter(|value| !value.is_empty())
        .all(|value| {
            value.eq_ignore_ascii_case(WINDOWS_IMAGE_HANDLER)
                || value.eq_ignore_ascii_case(CLSID_TEXT)
                || dead(&value)
        })
}

/// The DLL that belongs to this executable.
pub fn dll_path() -> Result<PathBuf, Error> {
    let path = std::env::current_exe()?.with_file_name(DLL_NAME);
    if path.is_file() {
        Ok(path)
    } else {
        Err(Error::Io(format!(
            "{DLL_NAME} is missing next to the program"
        )))
    }
}

/// Registers the provider for the extensions that allow it. Returns those.
pub fn register(dll: &Path) -> Result<Vec<&'static str>, Error> {
    let text = dll
        .to_str()
        .filter(|t| dll.is_absolute() && !t.contains(['"', '\0']))
        .ok_or_else(|| Error::Io("Invalid thumbnail provider path".into()))?;
    if !dll.is_file() {
        return Err(Error::Io(format!("{DLL_NAME} was not found")));
    }
    let class = format!(r"Software\Classes\CLSID\{CLSID_TEXT}");
    set(&class, "", "Kova Image thumbnail provider")?;
    set(&format!(r"{class}\InprocServer32"), "", text)?;
    set(
        &format!(r"{class}\InprocServer32"),
        "ThreadingModel",
        "Both",
    )?;
    let mut registered = Vec::new();
    for ext in extensions() {
        if available(ext) {
            set(
                &format!(r"Software\Classes\.{ext}\ShellEx\{HANDLER}"),
                "",
                CLSID_TEXT,
            )?;
            registered.push(ext);
        }
    }
    notify();
    Ok(registered)
}

/// Removes the class and the extension keys that point at it. Another program's
/// handler, registered later, is never touched.
pub fn unregister() -> Result<(), Error> {
    for ext in extensions() {
        let handler = format!(r"Software\Classes\.{ext}\ShellEx\{HANDLER}");
        if hkcu_default(&handler).is_some_and(|v| v.eq_ignore_ascii_case(CLSID_TEXT)) {
            delete(&handler);
            delete(&format!(r"Software\Classes\.{ext}\ShellEx"));
            delete(&format!(r"Software\Classes\.{ext}"));
        }
    }
    delete_tree(&format!(r"Software\Classes\CLSID\{CLSID_TEXT}"));
    notify();
    Ok(())
}

/// True when the provider class is registered for this user.
pub fn registered() -> bool {
    hkcu_default(&format!(
        r"Software\Classes\CLSID\{CLSID_TEXT}\InprocServer32"
    ))
    .is_some()
}

/// Deletes one key if it has no subkeys. Failure (missing, or not empty) is fine.
fn delete(path: &str) {
    if let Ok(path) = wide(path.as_ref()) {
        // SAFETY: terminated path in HKCU; an own key or one that is left empty.
        unsafe {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()));
        }
    }
}
fn delete_tree(path: &str) {
    if let Ok(path) = wide(path.as_ref()) {
        // SAFETY: terminated path in HKCU; only ever Kova's own class key.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()));
        }
    }
}
/// Tells Explorer that associations changed, so it asks for previews again.
fn notify() {
    // SAFETY: association-change notification has no pointer payload.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_and_png_are_left_to_windows_and_new_formats_are_not() {
        let all: Vec<_> = extensions().collect();
        for left in ["jpg", "png", "gif", "bmp", "tif", "ico"] {
            assert!(!all.contains(&left), "{left}");
        }
        for added in ["webp", "avif", "jxl", "svg", "qoi", "tga", "exr", "arw"] {
            assert!(all.contains(&added), "{added}");
        }
    }
    #[test]
    fn environment_variables_in_registry_paths_are_expanded() {
        // SAFETY: the test binary does not read this variable elsewhere.
        unsafe { std::env::set_var("KOVA_THUMBNAIL_TEST", r"C:\Programs") };
        assert_eq!(expand(r"%KOVA_THUMBNAIL_TEST%\x.dll"), r"C:\Programs\x.dll");
        // An unknown variable and a lone percent sign stay as they are.
        assert_eq!(
            expand(r"%KOVA_NO_SUCH_VARIABLE%\x"),
            r"%KOVA_NO_SUCH_VARIABLE%\x"
        );
        assert_eq!(expand("100% sure"), "100% sure");
    }
    #[test]
    fn a_handler_without_a_class_or_dll_is_dead_and_a_system_one_is_not() {
        // The class id of a thumbnail provider that was never registered.
        assert!(dead("{0F0E1D2C-3B4A-4958-8776-655443322110}"));
        // Windows' own generic image handler lives in a DLL that exists.
        assert!(!dead(WINDOWS_IMAGE_HANDLER));
    }
    #[test]
    fn the_class_id_text_matches_the_guid() {
        let text = format!("{{{:?}}}", CLSID).to_uppercase();
        assert_eq!(text, CLSID_TEXT);
    }
}
