use super::*;

impl App {
    pub(super) fn image_ready_without_notifier(&mut self) {
        if self.first_reported {
            return;
        }
        self.first_reported = true;
        if let Some(path) = self.measure.take() {
            let ms = self.started.elapsed().as_secs_f64() * 1000.0;
            let _ = std::thread::spawn(move || {
                let _ = std::fs::write(
                    path,
                    format!(
                        "{{\"first_render_ms\":null,\"image_ready_ms\":{ms:.3},\"metric\":\"image_ready_render_callback_unavailable\"}}\n"
                    ),
                );
            });
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("KOVA_TEST_CAPTURE").is_some() {
            Timer::single_shot(Duration::from_millis(150), || {
                with_app(|app| app.capture_test_frame())
            });
        }
    }
    pub(super) fn first_render(&mut self) {
        if self.render_contains_image && !self.first_reported {
            self.first_reported = true;
            #[cfg(debug_assertions)]
            if std::env::var_os("KOVA_TEST_CAPTURE").is_some() {
                let _ = slint::invoke_from_event_loop(|| with_app(|app| app.capture_test_frame()));
            }
            if let Some(path) = self.measure.take() {
                let ms = self.started.elapsed().as_secs_f64() * 1000.;
                let _=std::thread::Builder::new().name("measurement".into()).spawn(move ||{let _=std::fs::write(path,format!("{{\"first_render_ms\":{ms:.3},\"metric\":\"process_main_to_first_image_after_render\"}}\n"));});
            }
        }
    }
    /// Internal visual QA only; absent from release builds and product controls.
    #[cfg(debug_assertions)]
    pub(super) fn capture_test_frame(&self) {
        let Some(path) = std::env::var_os("KOVA_TEST_CAPTURE") else {
            return;
        };
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        match ui.window().take_snapshot() {
            Ok(pixels) => {
                let (width, height) = (pixels.width(), pixels.height());
                let bytes = pixels.as_bytes().to_vec();
                let mut state = format!(
                    "filename={}\nstatus={}\nrotation={}\nflip_h={}\nflip_v={}\nzoom={}\npaused={}\nframe={}\nfullscreen={}\nwidth={}\nheight={}\npan_x={}\npan_y={}\n",
                    ui.get_filename(),
                    ui.get_status(),
                    self.view.rotation,
                    self.view.flip_h,
                    self.view.flip_v,
                    ui.get_zoom_label(),
                    self.paused,
                    self.playback.frame,
                    ui.get_fullscreen(),
                    width,
                    height,
                    self.view.pan.0,
                    self.view.pan.1
                );
                state.push_str(&format!(
                    "chrome_hovered={}\ncursor_x={}\ncursor_y={}\n",
                    ui.get_chrome_hovered(),
                    self.cursor.0,
                    self.cursor.1
                ));
                state.push_str(&format!(
                    "chrome={}\nmore={}\nsettings={}\ninfo={}\nfocused={}\nfit_active={}\nactual_active={}\ncan_previous={}\ncan_next={}\nerror={}\n",
                    ui.get_chrome(), ui.get_show_more(), ui.get_show_settings(), ui.get_show_info(),
                    ui.get_control_focused(), ui.get_fit_active(), ui.get_actual_active(),
                    ui.get_can_previous(), ui.get_can_next(), ui.get_error_title()
                ));
                let _ = std::thread::spawn(move || {
                    let path = PathBuf::from(path);
                    let _ =
                        image::save_buffer(&path, &bytes, width, height, image::ColorType::Rgba8);
                    let _ = std::fs::write(path.with_extension("txt"), state);
                });
            }
            Err(error) => eprintln!("visual QA capture failed: {error}"),
        }
    }
}
