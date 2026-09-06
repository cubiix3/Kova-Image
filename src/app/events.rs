use super::*;

impl App {
    pub(super) fn wake_chrome(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.set_chrome(true);
        if ui.get_fullscreen() && self.settings.auto_hide {
            self.hide_chrome
                .start(TimerMode::SingleShot, Duration::from_secs(2), || {
                    with_app(|app| {
                        if let Some(ui) = app.ui.upgrade()
                            && ui.get_fullscreen()
                            && !ui.get_show_more()
                            && !ui.get_show_info()
                            && !ui.get_show_settings()
                            && !ui.get_chrome_hovered()
                            && !ui.get_control_focused()
                        {
                            ui.set_chrome(false);
                        }
                    })
                });
        }
    }
    pub(super) fn window_event(&mut self, event: &WindowEvent) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        match event {
            #[cfg(debug_assertions)]
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key
                        == slint::winit_030::winit::keyboard::Key::Named(
                            slint::winit_030::winit::keyboard::NamedKey::F12,
                        ) =>
            {
                self.capture_test_frame();
                return true;
            }
            WindowEvent::DroppedFile(path) => {
                self.open(path.clone(), true);
                return true;
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                ui.set_keyboard_mode(true);
                self.wake_chrome();
                #[cfg(debug_assertions)]
                if std::env::var_os("KOVA_TEST_CAPTURE").is_some() {
                    eprintln!("test input: {:?} {:?}", event.logical_key, self.modifiers);
                }
                if let Some(action) = input::shortcut(&event.logical_key, self.modifiers) {
                    // Space activates the focused control, including toggles;
                    // otherwise it remains the viewer's playback shortcut.
                    if (ui.get_slider_focused()
                        && matches!(action, Action::Previous | Action::Next))
                        || (action == Action::Pause && ui.get_control_focused())
                    {
                        return false;
                    }
                    if (ui.get_show_settings() || ui.get_show_info())
                        && !matches!(
                            action,
                            Action::Escape | Action::Register | Action::DefaultApps
                        )
                    {
                        return false;
                    }
                    if ui.get_show_more()
                        && !matches!(
                            action,
                            Action::Escape | Action::Register | Action::DefaultApps
                        )
                    {
                        return false;
                    }
                    if !event.repeat
                        || !matches!(
                            action,
                            Action::Delete | Action::Open | Action::CopyImage | Action::CopyPath
                        )
                    {
                        self.action(action);
                    }
                    return true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                ui.set_keyboard_mode(false);
                let scale = ui.window().scale_factor();
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                self.wake_chrome();
                if let Some(old) = self.drag {
                    self.view.pan.0 += self.cursor.0 - old.0;
                    self.view.pan.1 += self.cursor.1 - old.1;
                    self.drag = Some(self.cursor);
                    self.update_view();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                ui.set_keyboard_mode(false);
                if *state == ElementState::Released {
                    self.drag = None;
                }
                if ui.get_show_settings() || ui.get_show_info() || ui.get_show_more() {
                    return false;
                }
                if *state == ElementState::Pressed {
                    match button {
                        MouseButton::Back => {
                            self.action(Action::Previous);
                            return true;
                        }
                        MouseButton::Forward => {
                            self.action(Action::Next);
                            return true;
                        }
                        MouseButton::Left if self.in_canvas() => {
                            ui.invoke_focus_viewer();
                            if self.video_stamp.is_none() {
                                self.drag = Some(self.cursor);
                            }
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. }
                if self.in_canvas()
                    && !ui.get_show_more()
                    && !ui.get_show_settings()
                    && !ui.get_show_info() =>
            {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.,
                };
                if self.settings.wheel_zoom {
                    self.zoom(1.2f32.powf(amount.clamp(-4., 4.)), true);
                } else if amount != 0. {
                    self.action(if amount > 0. {
                        Action::Previous
                    } else {
                        Action::Next
                    });
                }
                return true;
            }
            WindowEvent::Resized(_) => {
                self.hidden = ui
                    .window()
                    .with_winit_window(|w| w.is_minimized().unwrap_or(false))
                    .unwrap_or(false);
                self.schedule();
                let _ = slint::invoke_from_event_loop(|| with_app(|app| app.update_view()));
            }
            WindowEvent::Occluded(hidden) => {
                self.hidden = *hidden;
                self.schedule();
            }
            WindowEvent::Focused(false) => {
                ui.set_keyboard_mode(false);
                self.drag = None;
                self.modifiers = ModifiersState::empty();
            }
            _ => {}
        }
        false
    }
    pub(super) fn in_canvas(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| {
            self.cursor.0 >= ui.get_viewport_left()
                && self.cursor.0 < ui.get_viewport_left() + ui.get_viewport_width()
                && self.cursor.1 >= ui.get_viewport_top()
                && self.cursor.1 < ui.get_viewport_top() + ui.get_viewport_height()
                && (!ui.get_fullscreen()
                    || !ui.get_chrome()
                    || (self.cursor.1 > ui.get_chrome_top()
                        && self.cursor.1 < ui.get_viewport_height() - ui.get_chrome_bottom()))
        })
    }
}
