use super::*;

impl App {
    pub(super) fn sync_settings(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_autoplay(self.settings.autoplay);
            ui.set_looping(self.settings.looping);
            ui.set_video_autoplay(self.settings.video_autoplay);
            ui.set_video_loop(self.settings.video_loop);
            ui.set_natural_sort(self.settings.natural_sort);
            ui.set_sort_by(
                match self.settings.sort_by {
                    SortBy::Modified => "modified",
                    SortBy::Size => "size",
                    SortBy::Name => "name",
                }
                .into(),
            );
            ui.set_sort_descending(self.settings.sort_descending);
            ui.set_wrap_around(self.settings.wrap);
            ui.set_slideshow_seconds(self.settings.slideshow_seconds as i32);
            ui.set_auto_hide(self.settings.auto_hide);
            ui.set_wheel_zoom(self.settings.wheel_zoom);
            ui.set_light_background(self.settings.light_background);
            ui.set_pixelated(self.settings.pixelated);
            ui.set_transparency_grid(self.settings.transparency_grid);
            ui.set_default_fit(
                match self.settings.fit {
                    Fit::Width => "width",
                    Fit::Actual => "actual",
                    _ => "window",
                }
                .into(),
            );
        }
    }
    pub(super) fn setting(&mut self, name: &str, value: bool) {
        let mut resort = false;
        match name {
            "video-autoplay" => self.settings.video_autoplay = value,
            "video-loop" => {
                self.settings.video_loop = value;
                if let Some(p) = &self.video {
                    p.looping(value);
                }
            }
            "autoplay" => self.settings.autoplay = value,
            "looping" => self.settings.looping = value,
            "natural" => {
                self.settings.natural_sort = value;
                resort = true;
            }
            "sort-name" => {
                self.settings.sort_by = SortBy::Name;
                resort = true;
            }
            "sort-date" => {
                self.settings.sort_by = SortBy::Modified;
                resort = true;
            }
            "sort-size" => {
                self.settings.sort_by = SortBy::Size;
                resort = true;
            }
            "descending" => {
                self.settings.sort_descending = value;
                resort = true;
            }
            "wrap" => self.settings.wrap = value,
            "slide-3" => self.settings.slideshow_seconds = 3,
            "slide-5" => self.settings.slideshow_seconds = 5,
            "slide-10" => self.settings.slideshow_seconds = 10,
            "slide-30" => self.settings.slideshow_seconds = 30,
            "hide" => self.settings.auto_hide = value,
            "wheel" => self.settings.wheel_zoom = value,
            "background" => self.settings.light_background = value,
            "pixels" => self.settings.pixelated = value,
            "grid" => self.settings.transparency_grid = value,
            "fit-window" => self.settings.fit = Fit::Window,
            "fit-width" => self.settings.fit = Fit::Width,
            "fit-actual" => self.settings.fit = Fit::Actual,
            _ => return,
        }
        self.sync_settings();
        self.update_navigation();
        self.wake_chrome();
        self.send_shell(Action::Settings, Some(self.settings.clone()));
        if resort {
            self.rescan_folder();
        }
    }
}
