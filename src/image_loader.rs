use crate::{
    cache::Cache,
    decoder::{self, Decoded, Stamp},
    error::Error,
    folder_navigation,
    security::{self, Generation, Ticket},
};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

pub enum Event {
    Video {
        id: u64,
        path: PathBuf,
        result: Result<crate::media::VideoSource, Error>,
    },
    Image {
        id: u64,
        path: PathBuf,
        result: Result<Arc<Decoded>, Error>,
        elapsed: std::time::Duration,
        cached: bool,
    },
    Folder {
        id: u64,
        path: PathBuf,
        result: Result<Vec<PathBuf>, Error>,
    },
}
struct Request {
    ticket: Ticket,
    path: PathBuf,
    neighbors: Vec<PathBuf>,
    scan: bool,
    natural: bool,
}
#[derive(Default)]
struct Mailbox {
    newest: Option<Request>,
    stop: bool,
}
pub struct Loader {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    generation: Generation,
    worker: Option<JoinHandle<()>>,
}
impl Loader {
    pub fn new(deliver: impl Fn(Event) + Send + 'static) -> std::io::Result<Self> {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker_mailbox = mailbox.clone();
        let worker = thread::Builder::new()
            .name("image-loader".into())
            .spawn(move || {
                let mut cache = Cache::new(security::CACHE_BUDGET);
                loop {
                    let request = {
                        let (lock, signal) = &*worker_mailbox;
                        let Ok(mut state) = lock.lock() else {
                            return;
                        };
                        while state.newest.is_none() && !state.stop {
                            state = match signal.wait(state) {
                                Ok(s) => s,
                                Err(_) => return,
                            };
                        }
                        if state.stop {
                            return;
                        }
                        let Some(r) = state.newest.take() else {
                            continue;
                        };
                        r
                    };
                    let Request {
                        ticket,
                        path,
                        mut neighbors,
                        scan,
                        natural,
                    } = request;
                    let start = Instant::now();
                    let video = crate::media::video_extension(&path)
                        || crate::media::probe(&path).ok().flatten().is_some();
                    if video {
                        let result = crate::media::open_video(&path, &ticket);
                        if ticket.is_current() {
                            deliver(Event::Video {
                                id: ticket.id,
                                path: path.clone(),
                                result,
                            });
                        }
                    } else {
                        let (result, cached) = cached_load(&mut cache, &path, &ticket);
                        if !ticket.is_current() {
                            continue;
                        }
                        deliver(Event::Image {
                            id: ticket.id,
                            path: path.clone(),
                            result,
                            elapsed: start.elapsed(),
                            cached,
                        });
                    }
                    if scan && ticket.is_current() {
                        let result = folder_navigation::scan(&path, &ticket, natural);
                        if let Ok(files) = &result {
                            let mut nav = folder_navigation::Navigation::default();
                            nav.set(files.clone(), &path);
                            neighbors = nav.neighbors();
                        }
                        if ticket.is_current() {
                            deliver(Event::Folder {
                                id: ticket.id,
                                path: path.clone(),
                                result,
                            });
                        }
                    }
                    // Single worker: foreground always wins over queued preloads.
                    // At most the next and previous image are speculated upon.
                    for neighbor in neighbors.into_iter().take(2) {
                        if crate::media::video_extension(&neighbor) {
                            continue;
                        }
                        if !ticket.is_current() {
                            break;
                        }
                        // The same admission limits apply to speculative decodes.
                        let _ = cached_load(&mut cache, &neighbor, &ticket);
                    }
                }
            })?;
        Ok(Self {
            mailbox,
            generation: Generation::default(),
            worker: Some(worker),
        })
    }
    pub fn request(
        &self,
        path: PathBuf,
        neighbors: Vec<PathBuf>,
        scan: bool,
        natural: bool,
    ) -> u64 {
        let ticket = self.generation.next();
        let id = ticket.id;
        let (lock, signal) = &*self.mailbox;
        if let Ok(mut mailbox) = lock.lock() {
            mailbox.newest = Some(Request {
                ticket,
                path,
                neighbors,
                scan,
                natural,
            });
            signal.notify_one();
        }
        id
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        self.generation.next();
        let (lock, signal) = &*self.mailbox;
        if let Ok(mut mailbox) = lock.lock() {
            mailbox.stop = true;
            mailbox.newest = None;
            signal.notify_one();
        }
        // Never block the closing UI on a codec with no internal cancellation.
        // Dropping JoinHandle detaches; process exit terminates remaining work.
        self.worker.take();
    }
}
fn cached_load(
    cache: &mut Cache,
    path: &std::path::Path,
    ticket: &Ticket,
) -> (Result<Arc<Decoded>, Error>, bool) {
    if let Err(e) = ticket.check() {
        return (Err(e), false);
    }
    let stamp = match Stamp::read(path) {
        Ok(s) => s,
        Err(e) => return (Err(e), false),
    };
    if let Some(image) = cache.get(path, &stamp) {
        return (Ok(image), true);
    }
    match decoder::load(path, ticket) {
        Ok(image) => {
            let image = Arc::new(image);
            cache.insert(path.to_path_buf(), image.clone());
            (Ok(image), false)
        }
        Err(error) => (Err(error), false),
    }
}
