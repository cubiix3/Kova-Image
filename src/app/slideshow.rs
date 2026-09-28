use super::*;

impl App {
    /// Sorting is decided while the folder is scanned, so a changed order needs
    /// a fresh scan. The open picture comes from the cache. A running video is
    /// left alone; the scan is remembered and happens when the next file opens.
    pub(super) fn rescan_folder(&mut self) {
        if self.video_stamp.is_some() {
            self.pending_rescan = true;
            return;
        }
        if let Some(path) = self.requested.clone() {
            self.open(path, true);
        }
    }
    /// Settings load after a file given on the command line was already
    /// requested with the default order. Returns true when that file was
    /// requested again with the saved order.
    pub(super) fn apply_saved_order(&mut self) -> bool {
        if self.requested.is_none()
            || self.video_stamp.is_some()
            || self.scan_order == self.settings.order()
        {
            return false;
        }
        self.rescan_folder();
        true
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
        // A slideshow plays each video once, so it overrides pausing and
        // looping for as long as it runs.
        if self.video_stamp.is_some()
            && let Some(player) = &self.video
        {
            player.looping(false);
            player.pause(false);
            self.paused = false;
            if let Some(ui) = self.ui.upgrade() {
                ui.set_paused(false);
            }
        }
        self.arm_slideshow();
        self.feedback(format!("Slideshow · {} s", self.settings.slideshow_seconds));
    }
    pub(super) fn stop_slideshow(&mut self, announce: bool) {
        self.slideshow_timer.stop();
        if self.slideshow {
            self.slideshow = false;
            if let Some(player) = &self.video {
                player.looping(self.settings.video_loop);
            }
            if announce {
                self.feedback("Slideshow stopped");
            }
        }
    }
    /// Starts the single-shot interval for the picture on screen. A playing
    /// video arms it when it ends instead, and a loading picture when it shows.
    pub(super) fn arm_slideshow(&mut self) {
        self.slideshow_timer.stop();
        // While the folder is being scanned the list is empty, and a step
        // would read that as the end. The Folder event arms the timer instead.
        if !self.slideshow || self.displayed != self.requested || self.pending_scan {
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
        if self.pending_rescan {
            self.action(Action::Next);
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
