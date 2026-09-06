use super::*;

impl App {
    pub(super) fn sync_settings(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_autoplay(self.settings.autoplay);
            ui.set_looping(self.settings.looping);
            ui.set_natural_sort(self.settings.natural_sort);
            ui.set_auto_hide(self.settings.auto_hide);
            ui.set_wheel_zoom(self.settings.wheel_zoom);
            ui.set_light_background(self.settings.light_background);
            ui.set_pixelated(self.settings.pixelated);
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
        match name {
            "autoplay" => self.settings.autoplay = value,
            "looping" => self.settings.looping = value,
            "natural" => self.settings.natural_sort = value,
            "hide" => self.settings.auto_hide = value,
            "wheel" => self.settings.wheel_zoom = value,
            "background" => self.settings.light_background = value,
            "pixels" => self.settings.pixelated = value,
            "fit-window" => self.settings.fit = Fit::Window,
            "fit-width" => self.settings.fit = Fit::Width,
            "fit-actual" => self.settings.fit = Fit::Actual,
            _ => return,
        }
        self.sync_settings();
        self.send_shell(Action::Settings, Some(self.settings.clone()));
    }
}
