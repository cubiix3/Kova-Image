use crate::ui::ViewerWindow;
use kova_image::{
    animation::{Loops, Playback},
    decoder::Decoded,
    error::Error,
    folder_navigation::Navigation,
    image_loader::{Event, Loader},
    input::{self, Action},
    settings::Settings,
    viewer::{Fit, View},
    windows_integration as native,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
            keyboard::ModifiersState,
        },
    },
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

thread_local! { static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) }; }
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref()
            && let Ok(mut app) = app.try_borrow_mut()
        {
            f(&mut app);
        }
    });
}

struct ShellJob {
    action: Action,
    owner: isize,
    path: Option<PathBuf>,
    image: Option<Arc<Decoded>>,
    frame: usize,
    stamp: Option<kova_image::decoder::Stamp>,
    stopped: Option<mpsc::Receiver<()>>,
    settings: Option<Settings>,
}
enum ShellResult {
    Open(Option<PathBuf>),
    Deleted(PathBuf),
    Done(&'static str),
}
struct App {
    ui: slint::Weak<ViewerWindow>,
    loader: Loader,
    shell: SyncSender<ShellJob>,
    shell_busy: bool,
    pending_settings: Option<Settings>,
    requested: Option<PathBuf>,
    displayed: Option<PathBuf>,
    id: u64,
    image: Option<Arc<Decoded>>,
    video: Option<kova_image::video::Player>,
    video_state: Option<kova_image::video::VideoState>,
    video_stamp: Option<kova_image::decoder::Stamp>,
    video_kind: Option<kova_image::media::VideoKind>,
    video_delete: Option<PathBuf>,
    volume: f64,
    muted: bool,
    nav: Navigation,
    view: View,
    settings: Settings,
    settings_ready: bool,
    pending_video: Option<(PathBuf, kova_image::media::VideoSource)>,
    playback: Playback,
    paused: bool,
    hidden: bool,
    animation: Timer,
    hide_chrome: Timer,
    notice: Timer,
    feedback_timer: Timer,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    drag: Option<(f32, f32)>,
    pointer_down: bool,
    started: Instant,
    measure: Option<PathBuf>,
    first_reported: bool,
    render_contains_image: bool,
    render_notifications: bool,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut software = false;
    let mut measure = None;
    while let Some(arg) = args.next() {
        if arg == "--register-file-associations" {
            native::register_associations()?;
            return Ok(());
        } else if arg == "--software" {
            software = true;
        } else if arg == "--measure" {
            measure = Some(PathBuf::from(
                args.next().ok_or("--measure requires an output path")?,
            ));
        } else if arg == "--" {
            path = args.next().map(PathBuf::from);
            break;
        } else if path.is_none() {
            path = Some(PathBuf::from(arg));
        } else {
            return Err("Expected one media path".into());
        }
    }
    // No directory scan, codec initialization, or database before window creation.
    let settings = Settings::default();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(if software { "software" } else { "femtovg" }.into())
        .select()?;
    let ui = ViewerWindow::new()?;
    let (events_tx, events_rx) = mpsc::sync_channel(4);
    let events_rx = Arc::new(std::sync::Mutex::new(events_rx));
    let loader = Loader::new(move |event| {
        // The mailbox caps retained decoded results even while a modal native
        // dialog is open. The foreground receiver never waits for a worker.
        if events_tx.try_send(event).is_ok() {
            let rx = events_rx.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Ok(rx) = rx.lock() {
                    while let Ok(event) = rx.try_recv() {
                        with_app(|app| app.event(event));
                    }
                }
            });
        }
    })?;
    let (shell_tx, shell_rx) = mpsc::sync_channel::<ShellJob>(1);
    std::thread::Builder::new()
        .name("windows-shell".into())
        .spawn(move || {
            let settings = Settings::load();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.settings = settings;
                    app.settings_ready = true;
                    app.view.reset(app.settings.fit);
                    app.sync_settings();
                    app.update_view();
                    if let Some((path, source)) = app.pending_video.take() {
                        app.start_video(path, source);
                    }
                })
            });
            let apartment = native::Apartment::new();
            while let Ok(job) = shell_rx.recv() {
                let result = match &apartment {
                    Ok(_) => shell_job(job),
                    Err(e) => Err(e.clone()),
                };
                let _ =
                    slint::invoke_from_event_loop(move || with_app(|app| app.shell_result(result)));
            }
        })?;
    let mut view = View::default();
    view.reset(settings.fit);
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        loader,
        shell: shell_tx,
        shell_busy: false,
        pending_settings: None,
        requested: None,
        displayed: None,
        id: 0,
        image: None,
        video: None,
        video_state: None,
        video_stamp: None,
        video_kind: None,
        video_delete: None,
        volume: 0.7,
        muted: cfg!(debug_assertions) && std::env::var_os("KOVA_TEST_MUTE").is_some(),
        nav: Navigation::default(),
        view,
        settings,
        settings_ready: false,
        pending_video: None,
        playback: Playback::default(),
        paused: false,
        hidden: false,
        animation: Timer::default(),
        hide_chrome: Timer::default(),
        notice: Timer::default(),
        feedback_timer: Timer::default(),
        modifiers: ModifiersState::empty(),
        cursor: (0., 0.),
        drag: None,
        pointer_down: false,
        started,
        measure,
        first_reported: false,
        render_contains_image: false,
        render_notifications: false,
    }));
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    ui.on_command(|name| {
        if let Some(action) = input::command(&name) {
            with_app(|app| app.action(action));
        }
    });
    ui.on_seek(|fraction| with_app(|app| app.seek_video(f64::from(fraction), true)));
    ui.on_volume_change(|volume| with_app(|app| app.audio_video(f64::from(volume), false)));
    ui.on_setting(|name, value| with_app(|app| app.setting(&name, value)));
    ui.on_activity(|| with_app(|app| app.wake_chrome()));
    let weak = ui.as_weak();
    ui.on_drag_window(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });
    ui.window().on_winit_window_event(|_, event| {
        let mut handled = false;
        with_app(|app| handled = app.window_event(event));
        if handled {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
    // Measures first render completion, not a claimed disk-cold startup time.
    let notifications = ui
        .window()
        .set_rendering_notifier(|state, _| match state {
            slint::RenderingState::BeforeRendering => with_app(|app| {
                app.render_contains_image = app
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.get_has_image() && !ui.get_loading());
            }),
            slint::RenderingState::AfterRendering => with_app(|app| app.first_render()),
            _ => {}
        })
        .is_ok();
    app.borrow_mut().render_notifications = notifications;
    app.borrow().sync_settings();
    ui.show()?;
    ui.window().with_winit_window(|w| {
        if let Ok(h) = w.window_handle()
            && let RawWindowHandle::Win32(h) = h.as_raw()
        {
            native::round_window(h.hwnd.get());
        }
    });
    if let Some(path) = path {
        app.borrow_mut().open(path, true);
    }
    slint::run_event_loop()?;
    APP.with(|slot| slot.borrow_mut().take());
    Ok(())
}

mod actions;
mod diagnostics;
mod events;
mod preferences;
mod presentation;
mod shell;
mod video;
use shell::shell_job;

impl App {
    fn feedback(&self, text: impl Into<slint::SharedString>) {
        if let Some(ui) = self.ui.upgrade()
            && ui.get_has_image()
            && !ui.get_loading()
            && ui.get_error_title().is_empty()
        {
            ui.set_feedback(text.into());
            self.feedback_timer
                .start(TimerMode::SingleShot, Duration::from_millis(1400), || {
                    with_app(|app| {
                        if let Some(ui) = app.ui.upgrade() {
                            ui.set_feedback("".into());
                        }
                    });
                });
        }
    }

    fn status(&self, text: impl Into<slint::SharedString>) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_status(text.into());
            self.notice
                .start(TimerMode::SingleShot, Duration::from_secs(5), || {
                    with_app(|app| {
                        if let Some(ui) = app.ui.upgrade()
                            && !ui.get_loading()
                        {
                            ui.set_status("".into());
                        }
                    });
                });
        }
    }
    fn open(&mut self, path: PathBuf, scan: bool) {
        if scan {
            self.nav = Navigation::default();
        }
        let path = if path.is_absolute() {
            path
        } else {
            match std::env::current_dir() {
                Ok(p) => p.join(path),
                Err(e) => {
                    self.status(e.to_string());
                    return;
                }
            }
        };
        self.animation.stop();
        self.feedback_timer.stop();
        self.pending_video = None;
        if let Some(player) = &self.video {
            player.stop();
        }
        self.video_state = None;
        self.video_stamp = None;
        self.video_kind = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_is_video(false);
            ui.set_feedback("".into());
        }
        self.playback = Playback::default();
        self.paused = !self.settings.autoplay;
        self.requested = Some(path.clone());
        self.id = self
            .loader
            .request(path, self.nav.neighbors(), scan, self.settings.natural_sort);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_error_title("".into());
            ui.set_error_detail("".into());
            ui.set_loading(true);
            ui.set_status("Loading…".into());
            ui.set_show_info(false);
            ui.set_show_more(false);
        }
        self.update_navigation();
        self.wake_chrome();
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Video { id, path, result } if id == self.id => match result {
                Ok(source) => self.start_video(path, source),
                Err(error) => self.media_error(path, error),
            },
            Event::Image {
                id,
                path,
                result,
                elapsed,
                cached,
            } if id == self.id => {
                let Some(ui) = self.ui.upgrade() else {
                    return;
                };
                ui.set_loading(false);
                match result {
                    Ok(image) => {
                        self.view.reset(self.settings.fit);
                        self.displayed = Some(path.clone());
                        self.image = Some(image);
                        self.playback = Playback::default();
                        self.paused = !self.settings.autoplay;
                        ui.set_filename(
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .as_ref()
                                .into(),
                        );
                        ui.set_has_image(true);
                        ui.set_status("".into());
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
                            "load: {:.2}ms, cache={cached}",
                            elapsed.as_secs_f64() * 1000.
                        );
                        let _ = (elapsed, cached);
                    }
                    Err(error) => {
                        self.image = None;
                        self.displayed = None;
                        ui.set_picture(slint::Image::default());
                        ui.set_has_image(false);
                        ui.set_animated(false);
                        ui.set_info_fields(slint::ModelRc::default());
                        ui.set_image_detail("".into());
                        ui.set_error_title(
                            match &error {
                                Error::NotFound => "Image not found",
                                Error::AccessDenied => "Access denied",
                                Error::Unsupported => "This format isn't supported",
                                Error::TooLarge | Error::Dimensions | Error::MemoryBudget => {
                                    "Image exceeds safety limits"
                                }
                                _ => "This image couldn't be opened",
                            }
                            .into(),
                        );
                        ui.set_error_detail(error.to_string().into());
                        ui.set_filename(
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .as_ref()
                                .into(),
                        );
                        self.status(error.to_string());
                    }
                }
            }
            Event::Folder { id, path, result } if id == self.id => match result {
                Ok(files) => {
                    self.nav.set(files, &path);
                    self.update_navigation();
                    self.update_info();
                }
                Err(e) => self.status(format!("Folder navigation: {e}")),
            },
            _ => {}
        }
    }
}
