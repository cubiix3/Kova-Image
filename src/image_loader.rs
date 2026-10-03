use crate::{
    cache::Cache,
    decoder::{self, Decoded, Stamp, Target},
    error::Error,
    folder_navigation::{self, Order},
    security::{self, Generation, Ticket},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long the worker stays idle after a result before it decodes neighbours.
/// A codec cannot be interrupted once its bytes are read, so speculative work
/// started between two quick key presses would delay the picture that matters.
const PRELOAD_DELAY: Duration = Duration::from_millis(150);

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
        preview: bool,
    },
    Folder {
        id: u64,
        path: PathBuf,
        result: Result<Vec<PathBuf>, Error>,
    },
}
impl Event {
    pub fn id(&self) -> u64 {
        match self {
            Self::Video { id, .. } | Self::Image { id, .. } | Self::Folder { id, .. } => *id,
        }
    }
    fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }
}
/// Queue a loader event for the UI without ever dropping the newest result.
/// Requests are served in id order, so older ids are stale once a newer one
/// arrives, and a later image replaces an earlier preview of the same request.
/// The queue therefore holds at most one image, one video and one folder event.
pub fn coalesce(queue: &mut VecDeque<Event>, event: Event) {
    let id = event.id();
    let image = event.is_image();
    queue.retain(|queued| queued.id() >= id && !(image && queued.id() == id && queued.is_image()));
    queue.push_back(event);
}
struct Request {
    ticket: Ticket,
    path: PathBuf,
    neighbors: Vec<PathBuf>,
    scan: bool,
    order: Order,
    target: Target,
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
                        order,
                        target,
                    } = request;
                    let start = Instant::now();
                    // Content decides too, but only after the image decoders decline:
                    // that saves a file open for every ordinary picture.
                    let mut video = crate::media::media_extension(&path);
                    let mut image = None;
                    if !video {
                        let outcome = cached_load(&mut cache, &path, &ticket, target, |image| {
                            if ticket.is_current() {
                                deliver(Event::Image {
                                    id: ticket.id,
                                    path: path.clone(),
                                    result: Ok(image),
                                    elapsed: start.elapsed(),
                                    cached: false,
                                    preview: true,
                                });
                            }
                        });
                        // The image decoders refuse files over 128 MiB before they
                        // look at the format, so a large video reports TooLarge.
                        if matches!(outcome.0, Err(Error::Unsupported | Error::TooLarge))
                            && ticket.is_current()
                            && crate::media::probe(&path).ok().flatten().is_some()
                        {
                            video = true;
                        } else {
                            image = Some(outcome);
                        }
                    }
                    if video {
                        let result = crate::media::open_video(&path, &ticket).map(|mut source| {
                            // A song shows its cover and title, so read them now.
                            if source.kind.is_audio() {
                                let info = crate::audio::read_info(
                                    &source.file,
                                    source.stamp.bytes,
                                    source.kind,
                                    &ticket,
                                    target,
                                );
                                source.audio = Some(Arc::new(info));
                            }
                            source
                        });
                        if ticket.is_current() {
                            deliver(Event::Video {
                                id: ticket.id,
                                path: path.clone(),
                                result,
                            });
                        }
                    } else if let Some((result, cached)) = image {
                        if !ticket.is_current() {
                            continue;
                        }
                        deliver(Event::Image {
                            id: ticket.id,
                            path: path.clone(),
                            result,
                            elapsed: start.elapsed(),
                            cached,
                            preview: false,
                        });
                    }
                    if scan && ticket.is_current() {
                        let result = folder_navigation::scan(&path, &ticket, order);
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
                    // At most the next and previous image are speculated upon,
                    // and only once the user has paused on this one.
                    if neighbors.is_empty() || !idle_for(&worker_mailbox, PRELOAD_DELAY) {
                        continue;
                    }
                    for neighbor in neighbors.into_iter().take(2) {
                        if crate::media::media_extension(&neighbor) {
                            continue;
                        }
                        if !ticket.is_current() {
                            break;
                        }
                        // The same admission limits apply to speculative decodes.
                        let _ = cached_load(&mut cache, &neighbor, &ticket, target, |_| {});
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
        order: Order,
        target: Target,
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
                order,
                target,
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
/// Waits up to `delay` and reports whether nothing new arrived meanwhile.
fn idle_for(mailbox: &(Mutex<Mailbox>, Condvar), delay: Duration) -> bool {
    let (lock, signal) = mailbox;
    let deadline = Instant::now() + delay;
    let Ok(mut state) = lock.lock() else {
        return false;
    };
    loop {
        if state.stop || state.newest.is_some() {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        state = match signal.wait_timeout(state, deadline - now) {
            Ok((state, _)) => state,
            Err(_) => return false,
        };
    }
}
fn cached_load(
    cache: &mut Cache,
    path: &std::path::Path,
    ticket: &Ticket,
    target: Target,
    mut preview: impl FnMut(Arc<Decoded>),
) -> (Result<Arc<Decoded>, Error>, bool) {
    if let Err(e) = ticket.check() {
        return (Err(e), false);
    }
    let stamp = match Stamp::read(path) {
        Ok(s) => s,
        Err(e) => return (Err(e), false),
    };
    if let Some(image) = cache.get_for(path, &stamp, target) {
        return (Ok(image), true);
    }
    match decoder::load_target(path, ticket, target, &mut |image| preview(Arc::new(image))) {
        Ok(image) => {
            let image = Arc::new(image);
            cache.insert(path.to_path_buf(), image.clone());
            (Ok(image), false)
        }
        Err(error) => (Err(error), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn folder(id: u64) -> Event {
        Event::Folder {
            id,
            path: PathBuf::new(),
            result: Ok(Vec::new()),
        }
    }
    fn image(id: u64, preview: bool) -> Event {
        Event::Image {
            id,
            path: PathBuf::new(),
            result: Err(Error::NotFound),
            elapsed: std::time::Duration::ZERO,
            cached: false,
            preview,
        }
    }
    #[test]
    fn newest_events_are_kept_and_stale_ones_dropped() {
        let mut queue = VecDeque::new();
        for id in 1..=10 {
            coalesce(&mut queue, image(id, true));
            coalesce(&mut queue, image(id, false));
            coalesce(&mut queue, folder(id));
        }
        assert_eq!(queue.len(), 2);
        assert!(matches!(
            queue[0],
            Event::Image {
                id: 10,
                preview: false,
                ..
            }
        ));
        assert!(matches!(queue[1], Event::Folder { id: 10, .. }));
    }
}
