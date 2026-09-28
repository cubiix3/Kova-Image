use super::*;

impl App {
    /// Sorting is decided while the folder is scanned, so a changed order needs
    /// a fresh scan. The open picture comes from the cache; a running video is
    /// left alone and the new order applies from the next file that is opened.
    pub(super) fn rescan_folder(&mut self) {
        if self.video_stamp.is_some() {
            return;
        }
        if let Some(path) = self.requested.clone() {
            self.open(path, true);
        }
    }
    pub(super) fn toggle_slideshow(&mut self) {
        if self.slideshow {
            self.stop_slideshow(true);
            return;
        }
        if self.displayed.is_none() || self.nav.files.len() < 2 {
            self.status("A slideshow needs an open folder with more than one file");
            return;
        }
        self.slideshow = true;
        self.arm_slideshow();
        self.feedback(format!("Slideshow · {} s", self.settings.slideshow_seconds));
    }
    pub(super) fn stop_slideshow(&mut self, announce: bool) {
        self.slideshow_timer.stop();
        if self.slideshow {
            self.slideshow = false;
            if announce {
                self.feedback("Slideshow stopped");
            }
        }
    }
    /// Starts the single-shot interval for the picture on screen. A playing
    /// video arms it when it ends instead, and a loading picture when it shows.
    pub(super) fn arm_slideshow(&mut self) {
        self.slideshow_timer.stop();
        if !self.slideshow || self.displayed != self.requested {
            return;
        }
        if self.video_stamp.is_some() && !self.video_state.as_ref().is_some_and(|s| s.ended) {
            return;
        }
        self.start_slideshow_timer();
    }
    /// A file that cannot be shown does not end the slideshow.
    pub(super) fn slideshow_after_error(&mut self) {
        if self.slideshow {
            self.start_slideshow_timer();
        }
    }
    fn start_slideshow_timer(&self) {
        let interval = Duration::from_secs(u64::from(self.settings.slideshow_seconds));
        self.slideshow_timer
            .start(TimerMode::SingleShot, interval, || {
                with_app(|app| app.slideshow_step())
            });
    }
    fn slideshow_step(&mut self) {
        if !self.slideshow {
            return;
        }
        let current = self.requested.clone();
        match self.nav.step(1, self.settings.wrap) {
            Some(next) if Some(&next) != current.as_ref() => self.open(next, false),
            _ => {
                self.stop_slideshow(false);
                self.status("Slideshow finished");
            }
        }
    }
}
