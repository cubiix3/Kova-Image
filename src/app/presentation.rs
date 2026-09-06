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
        let scale = self.view.scale(image, viewport);
        ui.set_display_width(image.0 * scale);
        ui.set_display_height(image.1 * scale);
        ui.set_pan_x(self.view.pan.0);
        ui.set_pan_y(self.view.pan.1);
        ui.set_rotation(self.view.rotation);
        ui.set_flip_h(self.view.flip_h);
        ui.set_flip_v(self.view.flip_v);
        ui.set_zoom_label(format!("{:.0}%", scale * 100.).into());
    }
    pub(super) fn update_info(&self) {
        if let (Some(ui), Some(image), Some(path)) =
            (self.ui.upgrade(), &self.image, &self.displayed)
        {
            ui.set_info(format!("{}\n{:?} · {} × {} · {:.2} MiB\n{} frame(s) · 8-bit RGBA display\nImage {} of {}\n\n{}",path.file_name().unwrap_or_default().to_string_lossy(),image.format,image.width,image.height,image.stamp.bytes as f64/(1024.*1024.),image.frames.len(),self.nav.index+1,self.nav.files.len().max(1),path.display()).into());
        }
    }
    pub(super) fn zoom(&mut self, factor: f32, mouse: bool) {
        let (image, viewport) = self.geometry();
        let anchor = if mouse {
            let top = self
                .ui
                .upgrade()
                .map(|ui| ui.get_viewport_top())
                .unwrap_or(0.);
            (
                self.cursor.0 - viewport.0 / 2.,
                self.cursor.1 - top - viewport.1 / 2.,
            )
        } else {
            (0., 0.)
        };
        self.view.zoom_at(factor, anchor, image, viewport);
        self.update_view();
    }
}
