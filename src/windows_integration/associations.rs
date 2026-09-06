use crate::{error::Error, media};
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Dwm::*,
        Storage::FileSystem::*,
        System::Registry::*,
        UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{PCWSTR, w},
};

fn wide(s: &std::ffi::OsStr) -> Result<Vec<u16>, Error> {
    let mut result: Vec<_> = s.encode_wide().collect();
    if result.contains(&0) {
        return Err(Error::Io("Null character in Windows path".into()));
    }
    result.push(0);
    Ok(result)
}
pub fn require_local_file(path: &Path) -> Result<(), Error> {
    use std::path::{Component, Prefix};
    let disk = match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
            _ => {
                return Err(Error::Io(
                    "Only local drive paths are supported for video".into(),
                ));
            }
        },
        _ => return Err(Error::Io("An absolute local video path is required".into())),
    };
    let root = [u16::from(disk), 58, 92, 0];
    // SAFETY: root is a terminated drive root; this query does not open the file.
    let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
    // GetDriveType: removable=2, fixed=3, CD-ROM=5, RAM disk=6.
    if !matches!(kind, 2 | 3 | 5 | 6) {
        return Err(Error::Io("Network video paths are not supported".into()));
    }
    Ok(())
}
pub fn open_command(exe: &Path) -> Result<String, Error> {
    let text = exe
        .to_str()
        .ok_or_else(|| Error::Io("Application path cannot be represented as Unicode".into()))?;
    if !exe.is_absolute() || text.contains(['"', '\0']) {
        return Err(Error::Io("Invalid application path".into()));
    }
    Ok(format!("\"{text}\" -- \"%1\""))
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: only successful RegCreateKeyEx handles are owned here.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
fn set(path: &str, name: &str, value: &str) -> Result<(), Error> {
    let path = wide(path.as_ref())?;
    let name = wide(name.as_ref())?;
    let value = wide(value.as_ref())?;
    let mut key = HKEY::default();
    // SAFETY: all strings are terminated; API copies the supplied bytes. This
    // function writes only the explicitly enumerated application-owned HKCU values.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| Error::Io(e.to_string()))?;
        let key = Key(key);
        let bytes = std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2);
        RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes))
            .ok()
            .map_err(|e| Error::Io(e.to_string()))?;
    }
    Ok(())
}
pub fn register_associations() -> Result<(), Error> {
    let exe = std::env::current_exe()?;
    register_executable(&exe)
}
pub fn register_executable(exe: &Path) -> Result<(), Error> {
    let command = open_command(exe)?;
    let icon = format!("\"{}\",0", exe.display());
    let app = r"Software\Classes\Applications\kova-image.exe";
    set(app, "FriendlyAppName", "Kova Image")?;
    set(&format!(r"{app}\shell\open\command"), "", &command)?;
    set(&format!(r"{app}\DefaultIcon"), "", &icon)?;
    let caps = r"Software\Kova\Image\Capabilities";
    set(caps, "ApplicationName", "Kova Image")?;
    set(
        caps,
        "ApplicationDescription",
        "Local images, animations and videos.",
    )?;
    set(caps, "ApplicationIcon", &icon)?;
    for (extensions, progid, description) in [
        (
            media::IMAGE_EXTENSIONS,
            "KovaImage.Image",
            "Kova Image picture",
        ),
        (
            media::VIDEO_EXTENSIONS,
            "KovaImage.Video",
            "Kova Image video",
        ),
    ] {
        let class = format!(r"Software\Classes\{progid}");
        set(&class, "", description)?;
        set(&format!(r"{class}\DefaultIcon"), "", &icon)?;
        set(&format!(r"{class}\shell\open\command"), "", &command)?;
        for extension in extensions {
            let ext = format!(".{extension}");
            set(&format!(r"{caps}\FileAssociations"), &ext, progid)?;
            set(&format!(r"{app}\SupportedTypes"), &ext, "")?;
            // No extension default value, UserChoice or hash is ever written.
            let sub = wide(format!(r"Software\Classes\{ext}\OpenWithProgids").as_ref())?;
            let name = wide(progid.as_ref())?;
            let mut key = HKEY::default();
            // SAFETY: valid output handle and terminated strings, own HKCU value only.
            unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(sub.as_ptr()),
                    None,
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    None,
                    &mut key,
                    None,
                )
                .ok()
                .map_err(|e| Error::Io(e.to_string()))?;
                let key = Key(key);
                RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_NONE, None)
                    .ok()
                    .map_err(|e| Error::Io(e.to_string()))?;
            }
        }
    }
    set(r"Software\RegisteredApplications", "Kova Image", caps)?;
    // SAFETY: association-change notification has no pointer payload.
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    Ok(())
}
pub fn default_apps(owner: isize) -> Result<(), Error> {
    // SAFETY: fixed Windows Settings URI, no media path or user-controlled command.
    let result = unsafe {
        ShellExecuteW(
            Some(HWND(owner as _)),
            w!("open"),
            w!("ms-settings:defaultapps?registeredAppUser=Kova%20Image"),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err(Error::Io(
            "Windows Default Apps settings could not be opened".into(),
        ));
    }
    Ok(())
}
pub fn round_window(owner: isize) {
    let preference = DWMWCP_ROUND;
    // SAFETY: correctly sized DWM attribute value; unsupported Windows versions
    // simply keep their existing window corners. No custom window-region clipping.
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(owner as _),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&preference) as u32,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_command_quotes_unicode_spaces_and_option_separator() {
        assert_eq!(
            open_command(Path::new("C:\\Kova Test\\猫\\kova-image.exe")).unwrap(),
            "\"C:\\Kova Test\\猫\\kova-image.exe\" -- \"%1\""
        );
        assert!(open_command(Path::new("relative.exe")).is_err());
        assert!(!media::IMAGE_EXTENSIONS.contains(&"avif"));
    }
}
