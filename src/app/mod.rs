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
    nav: Navigation,
    view: View,
    settings: Settings,
    playback: Playback,
    paused: bool,
    hidden: bool,
    animation: Timer,
    hide_chrome: Timer,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    drag: Option<(f32, f32)>,
    started: Instant,
    measure: Option<PathBuf>,
    first_reported: bool,
    render_contains_image: bool,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut software = false;
    let mut measure = None;
    while let Some(arg) = args.next() {
        if arg == "--software" {
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
            return Err("Expected one image path".into());
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
                    app.view.reset(app.settings.fit);
                    app.sync_settings();
                    app.update_view();
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
        nav: Navigation::default(),
        view,
        settings,
        playback: Playback::default(),
        paused: false,
        hidden: false,
        animation: Timer::default(),
        hide_chrome: Timer::default(),
        modifiers: ModifiersState::empty(),
        cursor: (0., 0.),
        drag: None,
        started,
        measure,
        first_reported: false,
        render_contains_image: false,
    }));
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    ui.on_command(|name| {
        if let Some(action) = input::command(&name) {
            with_app(|app| app.action(action));
        }
    });
    ui.on_setting(|name, value| with_app(|app| app.setting(&name, value)));
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
    ui.window().set_rendering_notifier(|state, _| match state {
        slint::RenderingState::BeforeRendering => with_app(|app| {
            app.render_contains_image = app
                .ui
                .upgrade()
                .is_some_and(|ui| ui.get_has_image() && !ui.get_loading());
        }),
        slint::RenderingState::AfterRendering => with_app(|app| app.first_render()),
        _ => {}
    })?;
    app.borrow().sync_settings();
    ui.show()?;
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
use shell::shell_job;

impl App {
    fn status(&self, text: impl Into<slint::SharedString>) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_status(text.into());
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
        self.playback = Playback::default();
        self.paused = !self.settings.autoplay;
        self.requested = Some(path.clone());
        self.id = self
            .loader
            .request(path, self.nav.neighbors(), scan, self.settings.natural_sort);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_loading(true);
            ui.set_status("Loading…".into());
            ui.set_show_info(false);
        }
    }
    fn event(&mut self, event: Event) {
        match event {
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
                        ui.set_info("".into());
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
                    self.update_info();
                }
                Err(e) => self.status(format!("Folder navigation: {e}")),
            },
            _ => {}
        }
    }
}
