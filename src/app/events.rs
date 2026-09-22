use super::*;

impl App {
    pub(super) fn wake_chrome(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.set_chrome(true);
        self.hide_chrome.stop();
        if ui.get_has_image() && !ui.get_loading() && self.settings.auto_hide {
            self.hide_chrome
                .start(TimerMode::SingleShot, Duration::from_secs(2), || {
                    with_app(|app| {
                        if let Some(ui) = app.ui.upgrade()
                            && app.settings.auto_hide
                            && ui.get_has_image()
                            && !ui.get_loading()
                            && !ui.get_show_more()
                            && !ui.get_show_info()
                            && !ui.get_show_settings()
                            && !ui.get_chrome_hovered()
                            && !ui.get_control_focused()
                            && !app.pointer_down
                        {
                            ui.set_chrome(false);
                        }
                    })
                });
        }
    }
    pub(super) fn begin_window_drag(&mut self) {
        self.window_drag = Some(WindowDrag {
            grab: self.cursor_physical,
            restore: None,
        });
    }
    fn move_window(&mut self, position: (f64, f64)) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(drag) = self.window_drag.as_mut() else {
            return;
        };
        if ui
            .window()
            .with_winit_window(|window| window.is_maximized())
            .unwrap_or(false)
        {
            if drag.restore.is_none() {
                ui.window().with_winit_window(|window| {
                    let width = f64::from(window.inner_size().width).max(1.);
                    drag.restore = Some(((position.0 / width).clamp(0., 1.), position.1));
                    window.set_maximized(false);
                });
            }
            drag.grab = None;
            return;
        }
        if let Some((fraction, y)) = drag.restore.take() {
            // Restoring applies the saved normal bounds, which can be anywhere
            // on screen. Move them so the pointer keeps its relative spot.
            ui.window().with_winit_window(|window| {
                let (Ok(inner), Ok(outer)) = (window.inner_position(), window.outer_position())
                else {
                    return;
                };
                let size = window.inner_size();
                let screen = (
                    f64::from(inner.x) + position.0,
                    f64::from(inner.y) + position.1,
                );
                let grab = (
                    fraction * f64::from(size.width),
                    y.clamp(0., f64::from(size.height.saturating_sub(1))),
                );
                window.set_outer_position(PhysicalPosition::new(
                    (screen.0 - grab.0).round() as i32 - (inner.x - outer.x),
                    (screen.1 - grab.1).round() as i32 - (inner.y - outer.y),
                ));
                drag.grab = Some(grab);
            });
            return;
        }
        let grab = match drag.grab {
            Some(grab) => grab,
            None => {
                drag.grab = Some(position);
                return;
            }
        };
        let (dx, dy) = (
            (position.0 - grab.0).round() as i32,
            (position.1 - grab.1).round() as i32,
        );
        ui.window().with_winit_window(|window| {
            if let Ok(current) = window.outer_position() {
                let target = PhysicalPosition::new(current.x + dx, current.y + dy);
                if target != current {
                    window.set_outer_position(target);
                }
            }
        });
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
                let action = input::shortcut(&event.logical_key, self.modifiers);
                // Viewing shortcuts use feedback without uncovering the image.
                // Tab and commands that open UI restore the controls first.
                if ui.get_chrome()
                    || action.is_none()
                    || matches!(
                        action,
                        Some(
                            Action::Open
                                | Action::Info
                                | Action::Settings
                                | Action::Escape
                                | Action::Fullscreen
                        )
                    )
                {
                    self.wake_chrome();
                }
                #[cfg(debug_assertions)]
                if std::env::var_os("KOVA_TEST_CAPTURE").is_some() {
                    eprintln!("test input: {:?} {:?}", event.logical_key, self.modifiers);
                }
                if let Some(action) = action {
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
                self.cursor_physical = Some((position.x, position.y));
                self.wake_chrome();
                if self.window_drag.is_some() {
                    self.move_window((position.x, position.y));
                } else if let Some(old) = self.drag {
                    self.view.pan.0 += self.cursor.0 - old.0;
                    self.view.pan.1 += self.cursor.1 - old.1;
                    self.drag = Some(self.cursor);
                    self.update_view();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                ui.set_keyboard_mode(false);
                self.pointer_down = *state == ElementState::Pressed;
                self.wake_chrome();
                if *state == ElementState::Released {
                    self.drag = None;
                    self.window_drag = None;
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
                    // Touchpads send many small pixel deltas; step one image
                    // per notch-equivalent instead of one per event.
                    if self.wheel.signum() != amount.signum() {
                        self.wheel = 0.;
                    }
                    self.wheel += amount;
                    if self.wheel.abs() >= 1. {
                        let back = self.wheel > 0.;
                        self.wheel = 0.;
                        self.action(if back { Action::Previous } else { Action::Next });
                    }
                }
                return true;
            }
            WindowEvent::Resized(_) => {
                self.hidden = ui
                    .window()
                    .with_winit_window(|w| w.is_minimized().unwrap_or(false))
                    .unwrap_or(false);
                self.schedule();
                self.note_viewport();
                let _ = slint::invoke_from_event_loop(|| with_app(|app| app.update_view()));
            }
            WindowEvent::Occluded(hidden) => {
                self.hidden = *hidden;
                self.schedule();
            }
            WindowEvent::Focused(true) => self.apply_desktop(),
            WindowEvent::Focused(false) => {
                ui.set_keyboard_mode(false);
                self.drag = None;
                self.window_drag = None;
                self.pointer_down = false;
                self.modifiers = ModifiersState::empty();
            }
            _ => {}
        }
        false
    }
    pub(super) fn note_viewport(&mut self) {
        let (width, height) = self.presentation_viewport();
        if let Some(player) = &self.video {
            player.viewport(width, height);
        }
        self.ensure_detail();
    }
    pub(super) fn in_canvas(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| {
            self.cursor.0 >= ui.get_viewport_left()
                && self.cursor.0 < ui.get_viewport_left() + ui.get_viewport_width()
                && self.cursor.1 >= ui.get_viewport_top()
                && self.cursor.1 < ui.get_viewport_top() + ui.get_viewport_height()
                && (!ui.get_fullscreen() || !ui.get_chrome() || self.cursor.1 > ui.get_chrome_top())
                && (!ui.get_chrome()
                    || self.cursor.0 < ui.get_controls_left()
                    || self.cursor.0 >= ui.get_controls_left() + ui.get_controls_width()
                    || self.cursor.1 < ui.get_controls_top()
                    || self.cursor.1 >= ui.get_controls_top() + ui.get_controls_height())
        })
    }
}
