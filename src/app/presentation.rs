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
            .map(|i| kova_image::viewer::logical_image_size(i.source_width, i.source_height, dpi))
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
            let mut rows = vec![
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
                    format!("{} × {} px", image.source_width, image.source_height),
                ),
            ];
            if image.width != image.source_width || image.height != image.source_height {
                rows.push(("Decoded", format!("{} × {} px", image.width, image.height)));
            }
            if let Some(taken) = &image.photo.taken {
                rows.push(("Taken", taken.clone()));
            }
            if let Some(camera) = &image.photo.camera {
                rows.push(("Camera", camera.clone()));
            }
            if let Some(exposure) = &image.photo.exposure {
                rows.push(("Exposure", exposure.clone()));
            }
            rows.extend([
                (
                    "File size",
                    format!("{:.2} MiB", image.stamp.bytes as f64 / (1024. * 1024.)),
                ),
                ("Frames", image.frames.len().to_string()),
                ("Display", "8-bit sRGB".to_string()),
                ("Location", path.to_string_lossy().into_owned()),
            ]);
            let fields = rows
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
                    image.source_width,
                    image.source_height,
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
        self.ensure_detail();
        if let Some(ui) = self.ui.upgrade() {
            self.feedback(ui.get_zoom_label());
        }
    }
    pub(super) fn show_image(
        &mut self,
        path: PathBuf,
        image: Arc<Decoded>,
        reset: bool,
        preview: bool,
        elapsed: std::time::Duration,
        cached: bool,
    ) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.set_loading(false);
        if reset {
            self.view.reset(self.settings.fit);
            self.playback = Playback::default();
            self.paused = !self.settings.autoplay;
        } else if self.playback.frame >= image.frames.len() {
            self.playback.frame = 0;
        }
        self.displayed = Some(path.clone());
        self.image = Some(image);
        ui.set_filename(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref()
                .into(),
        );
        ui.set_has_image(true);
        ui.set_error_title("".into());
        ui.set_error_detail("".into());
        if self.undo.is_some() {
            self.status("Moved to the Recycle Bin. Ctrl+Z restores it.");
        } else {
            ui.set_status("".into());
        }
        self.frame();
        self.update_view();
        self.schedule();
        self.update_info();
        self.wake_chrome();
        if !self.render_notifications {
            self.image_ready_without_notifier();
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "load: {:.2}ms, cache={cached}, preview={preview}",
            elapsed.as_secs_f64() * 1000.
        );
        let _ = (elapsed, cached, preview);
        if preview {
            return;
        }
        if self.pending_copy {
            self.pending_copy = false;
            if self
                .image
                .as_ref()
                .is_some_and(|image| image.serves(kova_image::decoder::Target::full()))
            {
                self.send_shell(Action::CopyImage, None);
            }
        } else {
            self.ensure_detail();
        }
    }
    pub(super) fn physical_viewport(&self) -> Option<(u32, u32)> {
        let ui = self.ui.upgrade()?;
        let scale = ui.window().scale_factor();
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let width = ui.get_viewport_width() * scale;
        let height = ui.get_viewport_height() * scale;
        if width < 64.0 || height < 64.0 {
            return None;
        }
        Some((
            width.round().clamp(1.0, 8192.0) as u32,
            height.round().clamp(1.0, 8192.0) as u32,
        ))
    }
    pub(super) fn open_target(&self) -> kova_image::decoder::Target {
        let Some((w, h)) = self.physical_viewport() else {
            return kova_image::decoder::Target::full();
        };
        match self.settings.fit {
            Fit::Actual => kova_image::decoder::Target::full(),
            Fit::Width => kova_image::decoder::Target {
                max_width: w,
                max_height: kova_image::security::MAX_DIMENSION,
            },
            _ => kova_image::decoder::Target {
                max_width: w,
                max_height: h,
            },
        }
    }
    pub(super) fn needed_target(&self) -> kova_image::decoder::Target {
        let Some(image) = &self.image else {
            return self.open_target();
        };
        if self.view.fit == Fit::Actual {
            return kova_image::decoder::Target::full();
        }
        let ui = self.ui.upgrade();
        let dpi = ui
            .as_ref()
            .map(|ui| ui.window().scale_factor())
            .unwrap_or(1.0);
        let dpi = if dpi.is_finite() && dpi > 0.0 {
            dpi
        } else {
            1.0
        };
        let (logical, viewport) = self.geometry();
        let scale = self.view.scale(logical, viewport);
        let width = (logical.0 * scale * dpi)
            .round()
            .clamp(1.0, image.source_width as f32) as u32;
        let height = (logical.1 * scale * dpi)
            .round()
            .clamp(1.0, image.source_height as f32) as u32;
        kova_image::decoder::Target {
            max_width: width,
            max_height: height,
        }
    }
    pub(super) fn ensure_detail(&mut self) {
        self.ensure(self.needed_target());
    }
    pub(super) fn ensure(&mut self, target: kova_image::decoder::Target) {
        if self.refining || self.video_stamp.is_some() || self.preview_id.is_some() {
            return;
        }
        let Some(image) = &self.image else {
            return;
        };
        if image.serves(target) {
            return;
        }
        let Some(path) = self.requested.clone() else {
            return;
        };
        self.refining = true;
        self.id = self
            .loader
            .request(path, Vec::new(), false, self.settings.natural_sort, target);
    }
    pub(super) fn presentation_viewport(&self) -> (u32, u32) {
        self.physical_viewport()
            .map(|(w, h)| (w.min(3840), h.min(2160)))
            .unwrap_or((1920, 1080))
    }
    pub(super) fn apply_desktop(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let desktop = native::desktop();
        ui.set_high_contrast(desktop.high_contrast);
        ui.set_reduce_motion(desktop.reduce_motion);
        let theme = ui.global::<crate::ui::Theme>();
        let color = |value: u32| {
            slint::Color::from_rgb_u8(
                (value & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                ((value >> 16) & 0xff) as u8,
            )
        };
        if desktop.high_contrast {
            let window = color(desktop.window);
            let text = color(desktop.window_text);
            let highlight = color(desktop.highlight);
            let gray = color(desktop.gray);
            theme.set_canvas(window);
            theme.set_surface(window);
            theme.set_raised(window);
            theme.set_hover(highlight);
            theme.set_pressed(highlight);
            theme.set_stroke(text);
            theme.set_divider(text);
            theme.set_text(text);
            theme.set_secondary(text);
            theme.set_muted(gray);
            theme.set_disabled(gray);
            theme.set_accent(highlight);
            theme.set_selected(highlight);
            theme.set_danger(color(desktop.hot));
        } else {
            theme.set_canvas(slint::Color::from_rgb_u8(0x10, 0x12, 0x14));
            theme.set_surface(slint::Color::from_rgb_u8(0x1b, 0x1e, 0x22));
            theme.set_raised(slint::Color::from_rgb_u8(0x24, 0x28, 0x2d));
            theme.set_hover(slint::Color::from_rgb_u8(0x30, 0x36, 0x3d));
            theme.set_pressed(slint::Color::from_rgb_u8(0x3a, 0x43, 0x4c));
            theme.set_stroke(slint::Color::from_rgb_u8(0x36, 0x3d, 0x45));
            theme.set_divider(slint::Color::from_rgb_u8(0x2b, 0x30, 0x36));
            theme.set_text(slint::Color::from_rgb_u8(0xf0, 0xf2, 0xf5));
            theme.set_secondary(slint::Color::from_rgb_u8(0xb2, 0xba, 0xc4));
            theme.set_muted(slint::Color::from_rgb_u8(0x8d, 0x98, 0xa5));
            theme.set_disabled(slint::Color::from_rgb_u8(0x62, 0x6d, 0x79));
            theme.set_accent(slint::Color::from_rgb_u8(0x86, 0xd5, 0xf4));
            theme.set_selected(slint::Color::from_rgb_u8(0x25, 0x3e, 0x4b));
            theme.set_danger(slint::Color::from_rgb_u8(0xf4, 0x9b, 0x9b));
        }
    }
}
