use super::*;
use kova_image::{
    media::VideoSource,
    video::{Player, Update, VideoState},
};
/// The cover of an audio file as a picture for the window.
fn cover_picture(cover: &kova_image::decoder::Decoded) -> slint::Image {
    let (width, height) = (cover.width, cover.height);
    cover
        .frames
        .first()
        .filter(|f| f.rgba.len() == width as usize * height as usize * 4)
        .map_or_else(slint::Image::default, |frame| {
            slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &frame.rgba,
                width,
                height,
            ))
        })
}

struct PresentedVideo {
    id: u64,
    state: Result<VideoState, Error>,
    frame: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,
}

impl App {
    pub(super) fn start_video(&mut self, path: PathBuf, source: VideoSource) {
        // Settings load on the Shell worker. Never play even a first audio
        // sample before an explicitly saved video-autoplay=false is known.
        if !self.settings_ready {
            self.pending_video = Some((path, source));
            return;
        }
        if self.video.is_none() {
            // At most one pending frame and one event-loop wake-up. A slow UI
            // replaces stale frames instead of retaining a playback backlog.
            let slot = Arc::new(std::sync::Mutex::new(None::<PresentedVideo>));
            match Player::new(move |Update { id, state, frame }| {
                // Prepare the renderer's shared buffer on the playback worker.
                // The UI only adopts it; no full-frame CPU copy blocks input.
                let mut update = PresentedVideo {
                    id,
                    state,
                    frame: frame.map(|frame| {
                        slint::SharedPixelBuffer::clone_from_slice(
                            &frame.rgba,
                            frame.width,
                            frame.height,
                        )
                    }),
                };
                let wake = if let Ok(mut pending) = slot.lock() {
                    let wake = pending.is_none();
                    if update.frame.is_none()
                        && let Some(old) = pending.as_mut()
                        && old.id == update.id
                    {
                        update.frame = old.frame.take();
                    }
                    *pending = Some(update);
                    wake
                } else {
                    false
                };
                if wake {
                    let slot = slot.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let update = slot.lock().ok().and_then(|mut p| p.take());
                        if let Some(update) = update {
                            with_app(|app| app.video_update(update));
                        }
                    });
                }
            }) {
                Ok(player) => self.video = Some(player),
                Err(e) => {
                    self.media_error(path, e.into());
                    return;
                }
            }
        }
        self.image = None;
        self.displayed = Some(path.clone());
        self.video_stamp = Some(source.stamp.clone());
        self.video_kind = Some(source.kind);
        self.audio_info = source.audio.clone();
        let audio = source.kind.is_audio();
        let cover = source.audio.as_ref().and_then(|a| a.cover.clone());
        // A slideshow plays every video once, whatever the saved settings say.
        let autoplay = self.settings.video_autoplay || self.slideshow;
        let looping = self.settings.video_loop && !self.slideshow;
        self.paused = !autoplay;
        self.view.reset(Fit::Window);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_muted(self.muted);
            ui.set_is_video(true);
            ui.set_is_audio(audio);
            ui.set_has_image(false);
            ui.set_has_cover(cover.is_some());
            // A cover is shown once the file is ready, with the controls.
            ui.set_picture(
                cover
                    .as_deref()
                    .map_or_else(slint::Image::default, cover_picture),
            );
            let tags = source
                .audio
                .as_ref()
                .map(|a| a.tags.clone())
                .unwrap_or_default();
            ui.set_audio_title(tags.title.as_str().into());
            ui.set_audio_artist(tags.artist.as_str().into());
            ui.set_audio_album(tags.album.as_str().into());
            ui.set_animated(true);
            ui.set_paused(self.paused);
            ui.set_filename(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref()
                    .into(),
            );
            ui.set_time_label("0:00 / 0:00".into());
            ui.set_video_progress(0.);
        }
        if let Some(player) = &self.video {
            player.audio(self.volume, self.muted);
            player.hidden(self.hidden);
            player.open(
                self.id,
                source,
                autoplay,
                looping,
                self.presentation_viewport(),
            );
        }
    }
    fn video_update(&mut self, update: PresentedVideo) {
        if update.id != self.id || self.video_stamp.is_none() || self.video_delete.is_some() {
            return;
        }
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        match update.state {
            Err(error) => {
                if let Some(path) = self.requested.clone() {
                    self.media_error(path, error);
                }
            }
            Ok(mut state) => {
                let audio = self.video_kind.is_some_and(|k| k.is_audio());
                if audio {
                    // Sound has no frame size; the cover, if any, sets the view.
                    (state.width, state.height) = self.audio_size();
                }
                ui.set_time_label(
                    format!(
                        "{} / {}",
                        kova_image::video::time_label(state.position),
                        kova_image::video::time_label(state.duration)
                    )
                    .into(),
                );
                ui.set_video_progress((state.position / state.duration) as f32);
                ui.set_paused(self.paused || state.ended);
                let first = self.video_state.is_none();
                let was_ended = self.video_state.as_ref().is_some_and(|s| s.ended);
                let ended = state.ended;
                self.video_state = Some(state);
                if ended && !was_ended {
                    // A slideshow waits for a video to finish, then moves on.
                    self.arm_slideshow();
                }
                if let Some(frame) = update.frame {
                    let was_loading = ui.get_loading();
                    ui.set_picture(slint::Image::from_rgba8(frame));
                    ui.set_has_image(true);
                    ui.set_loading(false);
                    // Only the first frame replaces the loading status; later
                    // frames must not wipe notices such as "Path copied".
                    if was_loading {
                        self.media_shown(false);
                        self.wake_chrome();
                    }
                    if !self.render_notifications {
                        self.image_ready_without_notifier();
                    }
                }
                if first && audio {
                    // No frame ever arrives: the first state shows the file.
                    let was_loading = ui.get_loading();
                    ui.set_has_image(true);
                    ui.set_loading(false);
                    if was_loading {
                        self.media_shown(false);
                        self.wake_chrome();
                    }
                    if !self.render_notifications {
                        self.image_ready_without_notifier();
                    }
                }
                if first {
                    self.update_view();
                    self.video_info();
                }
            }
        }
    }
    pub(super) fn media_error(&mut self, path: PathBuf, error: Error) {
        if let Some(player) = &self.video {
            player.stop();
        }
        self.image = None;
        self.displayed = None;
        self.video_state = None;
        self.video_stamp = None;
        self.video_kind = None;
        self.audio_info = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_picture(slint::Image::default());
            ui.set_has_image(false);
            ui.set_has_cover(false);
            ui.set_loading(false);
            ui.set_is_video(false);
            ui.set_is_audio(false);
            ui.set_animated(false);
            ui.set_image_detail("".into());
            ui.set_info_fields(slint::ModelRc::default());
            ui.set_filename(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref()
                    .into(),
            );
            ui.set_error_title(
                match error {
                    Error::NotFound => "File not found",
                    Error::AccessDenied => "Access denied",
                    Error::Unsupported => "This format isn't supported",
                    _ => "This media file couldn't be opened",
                }
                .into(),
            );
            ui.set_error_detail(error.to_string().into());
            ui.set_status("".into());
        }
        self.slideshow_after_error();
    }
    /// The size of the picture an audio file is shown with: its cover, or a
    /// nominal pixel when there is none (the panel does not scale).
    fn audio_size(&self) -> (u32, u32) {
        self.audio_info
            .as_ref()
            .and_then(|a| a.cover.as_ref())
            .map_or((1, 1), |c| (c.width.max(1), c.height.max(1)))
    }
    pub(super) fn seek_video(&mut self, value: f64, fraction: bool) {
        if let (Some(player), Some(state)) = (&self.video, &self.video_state) {
            let target = if fraction {
                value * state.duration
            } else {
                state.position + value
            };
            let target = target.clamp(0.0, state.duration);
            player.seek(target);
            self.feedback(if fraction {
                kova_image::video::time_label(target)
            } else {
                format!("{:+.0} s", target - state.position)
            });
        }
    }
    pub(super) fn audio_video(&mut self, volume: f64, muted: bool) {
        self.volume = volume.clamp(0., 1.);
        self.muted = muted;
        if let Some(player) = &self.video {
            player.audio(self.volume, self.muted);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_volume(self.volume as f32);
            ui.set_muted(muted);
        }
        self.feedback(if muted || self.volume == 0. {
            "Muted".to_string()
        } else {
            format!("Volume {:.0}%", self.volume * 100.)
        });
    }
    pub(super) fn video_info(&self) {
        if let (Some(ui), Some(state), Some(stamp), Some(path)) = (
            self.ui.upgrade(),
            &self.video_state,
            &self.video_stamp,
            &self.displayed,
        ) {
            let audio = self.video_kind.is_some_and(|k| k.is_audio());
            let format =
                self.video_kind
                    .map(|k| k.name())
                    .unwrap_or(if audio { "Audio" } else { "Video" });
            let tags = self
                .audio_info
                .as_ref()
                .map(|a| a.tags.clone())
                .unwrap_or_default();
            let duration = kova_image::video::time_label(state.duration);
            let detail = if audio {
                let who = [tags.artist.as_str(), tags.title.as_str()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" \u{2014} ");
                if who.is_empty() {
                    format!("{format}  \u{b7}  {duration}")
                } else {
                    format!("{format}  \u{b7}  {duration}  \u{b7}  {who}")
                }
            } else {
                format!(
                    "{format}  \u{b7}  {} \u{d7} {}  \u{b7}  {duration}",
                    state.width, state.height
                )
            };
            ui.set_image_detail(detail.into());
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut fields = vec![("Name", name)];
            if audio {
                for (label, value) in [
                    ("Title", tags.title),
                    ("Artist", tags.artist),
                    ("Album", tags.album),
                ] {
                    if !value.is_empty() {
                        fields.push((label, value));
                    }
                }
                fields.push(("Format", format.into()));
            } else {
                fields.push(("Container", format.into()));
                fields.push((
                    "Dimensions",
                    format!("{} \u{d7} {} px", state.width, state.height),
                ));
            }
            fields.push(("Duration", duration));
            fields.push((
                "File size",
                format!("{:.2} MiB", stamp.bytes as f64 / 1048576.),
            ));
            fields.push(("Playback", "Windows Media Foundation".into()));
            fields.push(("Location", path.to_string_lossy().into_owned()));
            let fields = fields
                .into_iter()
                .map(|(label, value)| crate::ui::InfoField {
                    label: label.into(),
                    value: value.into(),
                })
                .collect::<Vec<_>>();
            ui.set_info_fields(slint::ModelRc::new(slint::VecModel::from(fields)));
        }
    }
}
