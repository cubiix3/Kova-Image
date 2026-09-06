use super::*;

impl App {
    pub(super) fn action(&mut self, action: Action) {
        use Action::*;
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        match action {
            Previous | Next | First | Last => {
                let path = match action {
                    Previous => self.nav.step(-1),
                    Next => self.nav.step(1),
                    First => self.nav.first(),
                    _ => self.nav.last(),
                };
                if let Some(path) = path
                    && self.requested.as_ref() != Some(&path)
                {
                    self.open(path, false);
                }
            }
            ZoomIn => self.zoom(1.2, false),
            ZoomOut => self.zoom(1. / 1.2, false),
            Fit | FitWidth | Actual => {
                self.view.set_fit(match action {
                    Fit => kova_image::viewer::Fit::Window,
                    FitWidth => kova_image::viewer::Fit::Width,
                    _ => kova_image::viewer::Fit::Actual,
                });
                self.update_view();
            }
            RotateLeft | RotateRight => {
                self.view.rotation = (self.view.rotation
                    + if action == RotateLeft { -90 } else { 90 })
                .rem_euclid(360);
                self.update_view();
            }
            FlipHorizontal => {
                self.view.flip_h = !self.view.flip_h;
                self.update_view();
            }
            FlipVertical => {
                self.view.flip_v = !self.view.flip_v;
                self.update_view();
            }
            Fullscreen => {
                let full = !ui.get_fullscreen();
                ui.window().set_fullscreen(full);
                ui.set_fullscreen(full);
                ui.set_chrome(true);
                self.wake_chrome();
                self.update_view();
            }
            Escape => {
                if ui.get_show_more() || ui.get_show_info() || ui.get_show_settings() {
                    ui.set_show_more(false);
                    ui.set_show_info(false);
                    ui.set_show_settings(false);
                    return;
                }
                ui.set_show_more(false);
                ui.set_show_info(false);
                ui.set_show_settings(false);
                ui.window().set_fullscreen(false);
                ui.set_fullscreen(false);
                ui.set_chrome(true);
                self.update_view();
            }
            Pause => {
                if self.playback.finished {
                    self.playback = Playback::default();
                    self.paused = false;
                    self.frame();
                } else {
                    self.paused = !self.paused;
                }
                ui.set_paused(self.paused);
                self.schedule();
            }
            Info => {
                self.update_info();
                ui.set_show_info(!ui.get_show_info());
            }
            Settings => ui.set_show_settings(!ui.get_show_settings()),
            Minimize => {
                ui.window().set_minimized(true);
                self.hidden = true;
                self.animation.stop();
            }
            Maximize => {
                ui.window().set_maximized(!ui.window().is_maximized());
            }
            Close => {
                let _ = slint::quit_event_loop();
            }
            Open | CopyImage | CopyPath | Delete | Reveal | OpenWith => {
                self.send_shell(action, None)
            }
        }
    }
}
