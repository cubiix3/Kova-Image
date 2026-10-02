use super::*;

impl App {
    pub(super) fn action(&mut self, action: Action) {
        use Action::*;
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if self.video_stamp.is_some()
            && matches!(
                action,
                ZoomIn
                    | ZoomOut
                    | Actual
                    | FitWidth
                    | RotateLeft
                    | RotateRight
                    | FlipHorizontal
                    | FlipVertical
            )
        {
            return;
        }
        match action {
            Mute => {
                if self.video_stamp.is_some() {
                    self.audio_video(self.volume, !self.muted);
                }
            }
            SeekBack => self.seek_video(-5., false),
            SeekForward => self.seek_video(5., false),
            Previous | Next | First | Last => {
                // The folder order changed while a video played. Rescan first, so
                // the step is taken in the new order rather than the stale list.
                if self.pending_rescan
                    && let Some(path) = self.requested.clone()
                {
                    self.open(path, true);
                    self.deferred_nav = Some(action);
                    return;
                }
                let path = match action {
                    Previous => self.nav.step(-1, self.settings.wrap),
                    Next => self.nav.step(1, self.settings.wrap),
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
                self.ensure_detail();
                self.feedback(match action {
                    Fit => "Fit to window",
                    FitWidth => "Fit to width",
                    _ => "100%",
                });
            }
            RotateLeft | RotateRight => {
                self.view.rotation = (self.view.rotation
                    + if action == RotateLeft { -90 } else { 90 })
                .rem_euclid(360);
                self.update_view();
                self.feedback(format!("Rotation {}°", self.view.rotation));
            }
            FlipHorizontal => {
                self.view.flip_h = !self.view.flip_h;
                self.update_view();
                self.feedback(if self.view.flip_h {
                    "Flipped horizontally"
                } else {
                    "Horizontal flip off"
                });
            }
            FlipVertical => {
                self.view.flip_v = !self.view.flip_v;
                self.update_view();
                self.feedback(if self.view.flip_v {
                    "Flipped vertically"
                } else {
                    "Vertical flip off"
                });
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
                if self.slideshow {
                    self.stop_slideshow(true);
                    return;
                }
                ui.set_show_more(false);
                ui.set_show_info(false);
                ui.set_show_settings(false);
                ui.window().set_fullscreen(false);
                ui.set_fullscreen(false);
                ui.set_chrome(true);
                self.update_view();
                self.wake_chrome();
            }
            Pause if self.video_stamp.is_some() => {
                if let Some(player) = &self.video {
                    if self.video_state.as_ref().is_some_and(|s| s.ended) {
                        player.seek(0.);
                        self.paused = false;
                    } else {
                        self.paused = !self.paused;
                    }
                    player.pause(self.paused);
                    ui.set_paused(self.paused);
                    self.feedback(if self.paused { "Paused" } else { "Playing" });
                }
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
                if ui.get_animated() {
                    self.feedback(if self.paused { "Paused" } else { "Playing" });
                }
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
                if let Some(player) = &self.video {
                    player.hidden(true);
                }
            }
            Maximize => {
                ui.window().set_maximized(!ui.window().is_maximized());
            }
            Close => {
                let _ = slint::quit_event_loop();
            }
            CopyImage
                if self.image.as_ref().is_some_and(|image| {
                    image.frames.len() == 1 && !image.serves(kova_image::decoder::Target::full())
                }) =>
            {
                self.pending_copy = true;
                self.ensure(kova_image::decoder::Target::full());
                if !self.refining {
                    self.pending_copy = false;
                    self.send_shell(CopyImage, None);
                }
            }
            // The list is stale after a sort change made during a video, and the
            // replacement for a deleted file is picked from it. Rescan first.
            Delete if self.pending_rescan && self.requested.is_some() => {
                if let Some(path) = self.requested.clone() {
                    self.open(path, true);
                    self.deferred_nav = Some(Delete);
                }
            }
            Slideshow => self.toggle_slideshow(),
            Undo => self.send_shell(Undo, None),
            Open | CopyImage | CopyPath | CopyFile | Delete | Reveal | OpenWith | Register
            | DefaultApps | Previews | PreviewsOff => self.send_shell(action, None),
        }
    }
}
