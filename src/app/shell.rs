use super::*;

impl App {
    pub(super) fn send_shell(&mut self, action: Action, settings: Option<Settings>) {
        if self.shell_busy {
            if settings.is_some() {
                self.pending_settings = settings;
                return;
            }
            self.status("A Windows operation is already in progress");
            return;
        }
        if settings.is_none()
            && !matches!(
                action,
                Action::Open | Action::Register | Action::DefaultApps
            )
            && ((self.image.is_none() && self.video_stamp.is_none())
                || self.displayed != self.requested)
        {
            self.status("Wait for the media to finish loading");
            return;
        }
        let owner = self
            .ui
            .upgrade()
            .and_then(|ui| {
                ui.window().with_winit_window(|w| {
                    w.window_handle().ok().and_then(|h| match h.as_raw() {
                        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
                        _ => None,
                    })
                })
            })
            .flatten()
            .unwrap_or(0);
        let action = if action == Action::CopyImage && self.video_stamp.is_some() {
            Action::CopyPath
        } else {
            action
        };
        let stopped = if action == Action::Delete && self.video_stamp.is_some() {
            self.video_delete = self.displayed.clone();
            self.video.as_ref().map(|p| p.stop_confirmed())
        } else {
            None
        };
        let job = ShellJob {
            action,
            owner,
            path: self.displayed.clone(),
            image: self.image.clone(),
            frame: self.playback.frame,
            stamp: self
                .video_stamp
                .clone()
                .or_else(|| self.image.as_ref().map(|i| i.stamp.clone())),
            stopped,
            settings,
        };
        if self.shell.try_send(job).is_ok() {
            self.shell_busy = true;
        } else {
            if let Some(path) = self.video_delete.take()
                && self.requested.as_ref() == Some(&path)
            {
                self.open(path, false);
            }
            self.status("Windows worker unavailable");
        }
    }
    pub(super) fn shell_result(&mut self, result: Result<ShellResult, Error>) {
        self.shell_busy = false;
        let video_delete = self.video_delete.take();
        match result {
            Ok(ShellResult::Open(Some(path))) => self.open(path, true),
            Ok(ShellResult::Open(None)) => {}
            Ok(ShellResult::Done(message)) => self.status(message),
            Ok(ShellResult::Deleted(path)) => {
                self.nav.files.retain(|p| p != &path);
                if self.requested.as_ref() == Some(&path) {
                    self.animation.stop();
                    if let Some(next) = self.nav.step(0) {
                        self.open(next, true);
                    } else {
                        self.image = None;
                        self.video_stamp = None;
                        self.video_state = None;
                        self.video_kind = None;
                        self.displayed = None;
                        self.requested = None;
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_picture(slint::Image::default());
                            ui.set_has_image(false);
                            ui.set_is_video(false);
                            ui.set_animated(false);
                            ui.set_filename("Kova Image".into());
                            ui.set_image_detail("".into());
                            ui.set_error_title("".into());
                            ui.set_error_detail("".into());
                        }
                        self.update_navigation();
                        self.status("Moved to Recycle Bin");
                    }
                }
            }
            Err(e) => {
                if let Some(path) = video_delete
                    && self.requested.as_ref() == Some(&path)
                {
                    self.open(path, false);
                }
                self.status(e.to_string());
            }
        }
        if let Some(settings) = self.pending_settings.take() {
            self.send_shell(Action::Settings, Some(settings));
        }
    }
}

pub(super) fn shell_job(job: ShellJob) -> Result<ShellResult, Error> {
    if let Some(settings) = job.settings {
        settings.save()?;
        return Ok(ShellResult::Done("Settings saved"));
    }
    if job.action == Action::Open {
        return native::open_image(job.owner).map(ShellResult::Open);
    }
    if job.action == Action::Register {
        native::register_associations()?;
        return Ok(ShellResult::Done(
            "Registered for Open with. Choose defaults in Windows Settings.",
        ));
    }
    if job.action == Action::DefaultApps {
        native::default_apps(job.owner)?;
        return Ok(ShellResult::Done(""));
    }
    let path = job.path.ok_or(Error::NotFound)?;
    match job.action {
        Action::CopyPath => {
            native::copy_path(job.owner, &path)?;
            Ok(ShellResult::Done("Path copied"))
        }
        Action::CopyImage => {
            let image = job.image.ok_or(Error::NotFound)?;
            let frame = image.frames.get(job.frame).ok_or(Error::NotFound)?;
            native::copy_image(job.owner, image.width, image.height, &frame.rgba)?;
            Ok(ShellResult::Done("Image copied"))
        }
        Action::Delete => {
            if let Some(stopped) = job.stopped {
                stopped
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|_| Error::Io("Playback is still closing. Try again.".into()))?;
            }
            let stamp = job.stamp.ok_or(Error::NotFound)?;
            native::recycle(job.owner, &path, &stamp)?;
            Ok(ShellResult::Deleted(path))
        }
        Action::Reveal => {
            native::reveal(&path)?;
            Ok(ShellResult::Done(""))
        }
        Action::OpenWith => {
            native::open_with(job.owner, &path)?;
            Ok(ShellResult::Done(""))
        }
        _ => Ok(ShellResult::Done("")),
    }
}
