use crate::{error::Error, viewer::Fit};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
#[derive(Clone, Debug)]
pub struct Settings {
    pub fit: Fit,
    pub autoplay: bool,
    pub looping: bool,
    pub natural_sort: bool,
    pub auto_hide: bool,
    pub wheel_zoom: bool,
    pub light_background: bool,
    pub pixelated: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fit: Fit::Window,
            autoplay: true,
            looping: true,
            natural_sort: true,
            auto_hide: true,
            wheel_zoom: true,
            light_background: false,
            pixelated: false,
        }
    }
}
impl Settings {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Kova Image").join("settings.conf"))
    }
    pub fn load() -> Self {
        let mut result = Self::default();
        if let Some(path) = Self::path()
            && let Ok(file) = std::fs::File::open(path)
        {
            let mut text = String::new();
            if file.take(8192).read_to_string(&mut text).is_ok() {
                result = Self::parse(&text);
            }
        }
        result
    }
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                let value = value.trim();
                let enabled = match value {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                };
                match key.trim() {
                    "fit" => {
                        s.fit = match value {
                            "width" => Fit::Width,
                            "actual" => Fit::Actual,
                            _ => Fit::Window,
                        }
                    }
                    "autoplay" => {
                        if let Some(v) = enabled {
                            s.autoplay = v
                        }
                    }
                    "looping" => {
                        if let Some(v) = enabled {
                            s.looping = v
                        }
                    }
                    "natural_sort" => {
                        if let Some(v) = enabled {
                            s.natural_sort = v
                        }
                    }
                    "auto_hide" => {
                        if let Some(v) = enabled {
                            s.auto_hide = v
                        }
                    }
                    "wheel_zoom" => {
                        if let Some(v) = enabled {
                            s.wheel_zoom = v
                        }
                    }
                    "light_background" => {
                        if let Some(v) = enabled {
                            s.light_background = v
                        }
                    }
                    "pixelated" => {
                        if let Some(v) = enabled {
                            s.pixelated = v
                        }
                    }
                    _ => {}
                }
            }
        }
        s
    }
    pub fn save(&self) -> Result<(), Error> {
        let path = Self::path().ok_or_else(|| Error::Io("LOCALAPPDATA is unavailable".into()))?;
        let parent = path.parent().ok_or(Error::NotFound)?;
        std::fs::create_dir_all(parent)?;
        let temp = parent.join(format!("settings-{}.tmp", std::process::id()));
        let text = format!(
            "fit={}\nautoplay={}\nlooping={}\nnatural_sort={}\nauto_hide={}\nwheel_zoom={}\nlight_background={}\npixelated={}\n",
            match self.fit {
                Fit::Width => "width",
                Fit::Actual => "actual",
                _ => "window",
            },
            self.autoplay,
            self.looping,
            self.natural_sort,
            self.auto_hide,
            self.wheel_zoom,
            self.light_background,
            self.pixelated
        );
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        // Windows rename replaces the destination atomically through MoveFileExW.
        std::fs::rename(&temp, &path)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_settings_recover() {
        let s = Settings::parse("autoplay=nonsense\nfit=actual\nunknown=x\nlooping=false");
        assert!(s.autoplay);
        assert!(!s.looping);
        assert_eq!(s.fit, Fit::Actual);
    }
}
