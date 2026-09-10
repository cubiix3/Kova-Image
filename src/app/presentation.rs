use super::*;

impl App {
    pub(super) fn frame(&self) {
        let (Some(ui), Some(image)) = (self.ui.upgrade(), &self.image) else {
            return;
        };
        if let Some(frame) = image.frames.get(self.playback.frame) {
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                &frame.rgba,
                image.width,
                image.height,
            );
            ui.set_picture(slint::Image::from_rgba8(buffer));
            ui.set_animated(image.frames.len() > 1);
            ui.set_paused(self.paused || self.playback.finished);
        }
    }
    pub(super) fn schedule(&self) {
        self.animation.stop();
        if self.video_stamp.is_some() {
            if let Some(player) = &self.video {
                player.hidden(self.hidden);
            }
            return;
        }
        if self.paused || self.hidden || self.playback.finished {
            return;
        }
        let Some(image) = &self.image else {
            return;
        };
        if image.frames.len() < 2 {
            return;
        }
        let delay = image.frames[self.playback.frame].delay;
        self.animation
            .start(TimerMode::SingleShot, delay, || with_app(|app| app.tick()));
    }
    pub(super) fn tick(&mut self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if ui
            .window()
            .with_winit_window(|w| w.is_minimized().unwrap_or(false))
            .unwrap_or(false)
        {
            self.hidden = true;
            return;
        }
        let Some(image) = &self.image else {
            return;
        };
        let loops = if self.settings.looping {
            image.loops
        } else {
            Loops(Some(1))
        };
        if self.playback.advance(image.frames.len(), loops) {
            self.frame();
            self.schedule();
        } else {
            ui.set_paused(true);
        }
    }
    pub(super) fn geometry(&self) -> ((f32, f32), (f32, f32)) {
        let dpi = self
            .ui
            .upgrade()
            .map(|ui| ui.window().scale_factor())
            .unwrap_or(1.0);
        let image = self
            .image
            .as_ref()
            .map(|i| kova_image::viewer::logical_image_size(i.width, i.height, dpi))
            .or_else(|| {
                self.video_state
                    .as_ref()
                    .map(|s| kova_image::viewer::logical_image_size(s.width, s.height, dpi))
            })
            .unwrap_or((1., 1.));
        let viewport = self
            .ui
            .upgrade()
            .map(|ui| (ui.get_viewport_width(), ui.get_viewport_height()))
            .unwrap_or((1., 1.));
        (image, viewport)
    }
    pub(super) fn update_view(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let (image, viewport) = self.geometry();
        let scale = if self.video_stamp.is_some() && ui.get_fullscreen() {
            (viewport.0 / image.0)
                .min(viewport.1 / image.1)
                .clamp(0.001, 64.)
        } else {
            self.view.scale(image, viewport)
        };
        ui.set_display_width(image.0 * scale);
        ui.set_display_height(image.1 * scale);
        ui.set_pan_x(self.view.pan.0);
        ui.set_pan_y(self.view.pan.1);
        ui.set_rotation(self.view.rotation);
        ui.set_flip_h(self.view.flip_h);
        ui.set_flip_v(self.view.flip_v);
        ui.set_zoom_label(format!("{:.0}%", scale * 100.).into());
        ui.set_fit_active(self.view.fit == Fit::Window);
        ui.set_actual_active(self.view.fit == Fit::Actual);
    }
    pub(super) fn update_navigation(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_can_previous(self.nav.index > 0 && !self.nav.files.is_empty());
            ui.set_can_next(self.nav.index + 1 < self.nav.files.len());
            ui.set_position_label(if self.nav.files.is_empty() {
                "".into()
            } else {
                format!("{} / {}", self.nav.index + 1, self.nav.files.len()).into()
            });
        }
    }
    pub(super) fn update_info(&self) {
        if self.video_stamp.is_some() {
            self.video_info();
            return;
        }
        if let (Some(ui), Some(image), Some(path)) =
            (self.ui.upgrade(), &self.image, &self.displayed)
        {
            let fields = [
                (
                    "Name",
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
                ("Format", format!("{:?}", image.format).to_uppercase()),
                (
                    "Dimensions",
                    format!("{} × {} px", image.width, image.height),
                ),
                (
                    "File size",
                    format!("{:.2} MiB", image.stamp.bytes as f64 / (1024. * 1024.)),
                ),
                ("Frames", image.frames.len().to_string()),
                ("Display", "8-bit RGBA".to_string()),
                ("Location", path.to_string_lossy().into_owned()),
            ]
            .into_iter()
            .map(|(label, value)| crate::ui::InfoField {
                label: label.into(),
                value: value.into(),
            })
            .collect::<Vec<_>>();
            ui.set_info_fields(slint::ModelRc::new(slint::VecModel::from(fields)));
            ui.set_image_detail(
                format!(
                    "{:?}  ·  {} × {}{}",
                    image.format,
                    image.width,
                    image.height,
                    if image.frames.len() > 1 {
                        "  ·  Animated"
                    } else {
                        ""
                    }
                )
                .into(),
            );
        }
    }
    pub(super) fn zoom(&mut self, factor: f32, mouse: bool) {
        if self.video_stamp.is_some() {
            return;
        }
        let (image, viewport) = self.geometry();
        let anchor = if mouse {
            let (left, top) = self
                .ui
                .upgrade()
                .map(|ui| (ui.get_viewport_left(), ui.get_viewport_top()))
                .unwrap_or((0., 0.));
            (
                self.cursor.0 - left - viewport.0 / 2.,
                self.cursor.1 - top - viewport.1 / 2.,
            )
        } else {
            (0., 0.)
        };
        self.view.zoom_at(factor, anchor, image, viewport);
        self.update_view();
        if let Some(ui) = self.ui.upgrade() {
            self.feedback(ui.get_zoom_label());
        }
    }
}
