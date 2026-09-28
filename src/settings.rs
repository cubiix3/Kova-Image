use crate::{
    error::Error,
    folder_navigation::{Order, SortBy},
    viewer::Fit,
};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
#[derive(Clone, Debug)]
pub struct Settings {
    pub fit: Fit,
    pub autoplay: bool,
    pub looping: bool,
    pub video_autoplay: bool,
    pub video_loop: bool,
    pub natural_sort: bool,
    pub sort_by: SortBy,
    pub sort_descending: bool,
    pub wrap: bool,
    pub slideshow_seconds: u32,
    pub auto_hide: bool,
    pub wheel_zoom: bool,
    pub light_background: bool,
    pub pixelated: bool,
    pub transparency_grid: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fit: Fit::Window,
            autoplay: true,
            looping: true,
            video_autoplay: true,
            video_loop: false,
            natural_sort: true,
            sort_by: SortBy::Name,
            sort_descending: false,
            wrap: false,
            slideshow_seconds: 5,
            auto_hide: true,
            wheel_zoom: true,
            light_background: false,
            pixelated: false,
            transparency_grid: true,
        }
    }
}
/// Intervals offered for the slideshow, in seconds.
pub const SLIDESHOW_SECONDS: [u32; 4] = [3, 5, 10, 30];
impl Settings {
    pub fn order(&self) -> Order {
        Order {
            natural: self.natural_sort,
            by: self.sort_by,
            descending: self.sort_descending,
        }
    }
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
                    "video_autoplay" => {
                        if let Some(v) = enabled {
                            s.video_autoplay = v;
                        }
                    }
                    "video_loop" => {
                        if let Some(v) = enabled {
                            s.video_loop = v;
                        }
                    }
                    "natural_sort" => {
                        if let Some(v) = enabled {
                            s.natural_sort = v
                        }
                    }
                    "sort" => {
                        s.sort_by = match value {
                            "modified" => SortBy::Modified,
                            "size" => SortBy::Size,
                            _ => SortBy::Name,
                        }
                    }
                    "sort_descending" => {
                        if let Some(v) = enabled {
                            s.sort_descending = v
                        }
                    }
                    "wrap" => {
                        if let Some(v) = enabled {
                            s.wrap = v
                        }
                    }
                    "slideshow_seconds" => {
                        if let Some(v) =
                            value.parse().ok().filter(|v| SLIDESHOW_SECONDS.contains(v))
                        {
                            s.slideshow_seconds = v
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
                    "transparency_grid" => {
                        if let Some(v) = enabled {
                            s.transparency_grid = v
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
            "fit={}\nautoplay={}\nlooping={}\nnatural_sort={}\nsort={}\nsort_descending={}\nwrap={}\nslideshow_seconds={}\nauto_hide={}\nwheel_zoom={}\nlight_background={}\npixelated={}\ntransparency_grid={}\nvideo_autoplay={}\nvideo_loop={}\n",
            match self.fit {
                Fit::Width => "width",
                Fit::Actual => "actual",
                _ => "window",
            },
            self.autoplay,
            self.looping,
            self.natural_sort,
            match self.sort_by {
                SortBy::Modified => "modified",
                SortBy::Size => "size",
                SortBy::Name => "name",
            },
            self.sort_descending,
            self.wrap,
            self.slideshow_seconds,
            self.auto_hide,
            self.wheel_zoom,
            self.light_background,
            self.pixelated,
            self.transparency_grid,
            self.video_autoplay,
            self.video_loop
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
    #[test]
    fn sort_wrap_and_slideshow_round_trip_and_reject_bad_values() {
        let s = Settings::parse("sort=size\nsort_descending=true\nwrap=true\nslideshow_seconds=10");
        assert_eq!(s.sort_by, SortBy::Size);
        assert!(s.sort_descending && s.wrap);
        assert_eq!(s.slideshow_seconds, 10);
        let s = Settings::parse("sort=sideways\nslideshow_seconds=7\nwrap=maybe");
        assert_eq!(s.sort_by, SortBy::Name);
        assert_eq!(s.slideshow_seconds, 5);
        assert!(!s.wrap);
        assert_eq!(Settings::default().order(), Order::default());
    }
}
