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
            && action != Action::Open
            && (self.image.is_none() || self.displayed != self.requested)
        {
            self.status("Wait for an image to finish loading");
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
        let job = ShellJob {
            action,
            owner,
            path: self.displayed.clone(),
            image: self.image.clone(),
            frame: self.playback.frame,
            settings,
        };
        if self.shell.try_send(job).is_ok() {
            self.shell_busy = true;
        } else {
            self.status("Windows worker unavailable");
        }
    }
    pub(super) fn shell_result(&mut self, result: Result<ShellResult, Error>) {
        self.shell_busy = false;
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
                        self.displayed = None;
                        self.requested = None;
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_picture(slint::Image::default());
                            ui.set_has_image(false);
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
            Err(e) => self.status(e.to_string()),
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
            let image = job.image.ok_or(Error::NotFound)?;
            native::recycle(job.owner, &path, &image.stamp)?;
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
