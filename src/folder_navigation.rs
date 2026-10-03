use crate::{error::Error, security::Ticket};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const EXTENSIONS: &[&str] = crate::media::IMAGE_EXTENSIONS;
pub fn supported_extension(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|s| {
        EXTENSIONS
            .iter()
            .chain(crate::media::VIDEO_EXTENSIONS)
            .chain(crate::media::AUDIO_EXTENSIONS)
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    Name,
    Modified,
    Size,
}
/// How a folder is ordered. `natural` compares numbers inside names by value
/// the way Explorer does; without it names compare as plain text. The other
/// keys fall back to the name, and `descending` reverses the whole order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub natural: bool,
    pub by: SortBy,
    pub descending: bool,
}
impl Default for Order {
    fn default() -> Self {
        Self {
            natural: true,
            by: SortBy::Name,
            descending: false,
        }
    }
}
struct Entry {
    path: PathBuf,
    modified: Option<SystemTime>,
    size: u64,
}
impl Entry {
    fn new(path: PathBuf, meta: Option<std::fs::Metadata>) -> Self {
        Self {
            path,
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            size: meta.map_or(0, |m| m.len()),
        }
    }
}
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let l = left.to_lowercase();
    let r = right.to_lowercase();
    natural_cmp_folded(&l, &r).then_with(|| left.cmp(right))
}
fn natural_cmp_folded(left: &str, right: &str) -> Ordering {
    let (a, b) = (left.as_bytes(), right.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x = left[si..i].trim_start_matches('0');
            let y = right[sj..j].trim_start_matches('0');
            let cmp = x
                .len()
                .cmp(&y.len())
                .then_with(|| x.cmp(y))
                .then_with(|| (i - si).cmp(&(j - sj)));
            if cmp != Ordering::Equal {
                return cmp;
            }
        } else {
            let cmp = a[i].cmp(&b[j]);
            if cmp != Ordering::Equal {
                return cmp;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}
pub fn scan(path: &Path, ticket: &Ticket, order: Order) -> Result<Vec<PathBuf>, Error> {
    let folder = path.parent().ok_or(Error::NotFound)?;
    let mut entries = Vec::new();
    let mut path_bytes = 0usize;
    for item in std::fs::read_dir(folder)? {
        ticket.check()?;
        let Ok(item) = item else {
            continue;
        };
        let p = item.path();
        if supported_extension(&p) && item.file_type().is_ok_and(|t| t.is_file()) {
            path_bytes = path_bytes.saturating_add(p.as_os_str().len().saturating_mul(2));
            if path_bytes > 16 * 1024 * 1024 {
                return Err(Error::Io(
                    "Folder names exceed the 16 MiB navigation budget".into(),
                ));
            }
            // Directory enumeration already carries size and time on Windows,
            // so ordering by them reads no file content.
            let meta = (order.by != SortBy::Name)
                .then(|| item.metadata().ok())
                .flatten();
            entries.push(Entry::new(p, meta));
        }
        if entries.len() >= 100_000 {
            return Err(Error::Io("Folder exceeds 100,000 supported entries".into()));
        }
    }
    // Include an explicitly opened image with an unusual extension.
    if !entries.iter().any(|e| e.path == path) {
        let meta = (order.by != SortBy::Name)
            .then(|| std::fs::metadata(path).ok())
            .flatten();
        entries.push(Entry::new(path.to_path_buf(), meta));
    }
    let entries = sort_entries(entries, ticket, order)?;
    ticket.check()?;
    Ok(entries)
}
/// Comparison key for a file name. Windows compares with the Shell's own
/// logical order so the sequence matches Explorer, including umlauts.
#[cfg(windows)]
type NameKey = Vec<u16>;
#[cfg(windows)]
fn name_key(path: &Path) -> NameKey {
    crate::windows_integration::logical_key(path.file_name().unwrap_or_default())
}
#[cfg(windows)]
fn compare_names(left: &NameKey, right: &NameKey) -> Ordering {
    crate::windows_integration::compare_logical(left, right)
}
#[cfg(not(windows))]
type NameKey = String;
#[cfg(not(windows))]
fn name_key(path: &Path) -> NameKey {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
#[cfg(not(windows))]
fn compare_names(left: &NameKey, right: &NameKey) -> Ordering {
    natural_cmp_folded(left, right)
}
struct Keyed {
    entry: Entry,
    name: NameKey,
}
fn sort_entries(entries: Vec<Entry>, ticket: &Ticket, order: Order) -> Result<Vec<PathBuf>, Error> {
    let mut keyed = Vec::with_capacity(entries.len());
    for entry in entries {
        ticket.check()?;
        let name = if order.natural {
            name_key(&entry.path)
        } else {
            NameKey::default()
        };
        keyed.push(Keyed { entry, name });
    }
    let by_name = |left: &Keyed, right: &Keyed| {
        if order.natural {
            compare_names(&left.name, &right.name)
        } else {
            Ordering::Equal
        }
        .then_with(|| left.entry.path.cmp(&right.entry.path))
    };
    let compare = |left: &Keyed, right: &Keyed, by_name: &dyn Fn(&Keyed, &Keyed) -> Ordering| {
        let primary = match order.by {
            SortBy::Name => Ordering::Equal,
            SortBy::Modified => left.entry.modified.cmp(&right.entry.modified),
            SortBy::Size => left.entry.size.cmp(&right.entry.size),
        };
        let result = primary.then_with(|| by_name(left, right));
        if order.descending {
            result.reverse()
        } else {
            result
        }
    };
    // The Shell comparison is not guaranteed to be a strict total order, and
    // the standard sort may panic on one. Fall back to the portable comparison
    // rather than losing the loader thread.
    let sorted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        keyed.sort_by(|a, b| compare(a, b, &by_name));
    }));
    if sorted.is_err() {
        for item in &mut keyed {
            item.name = NameKey::default();
        }
        keyed.sort_by(|a, b| {
            compare(a, b, &|l: &Keyed, r: &Keyed| {
                natural_cmp(
                    &l.entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    &r.entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                )
            })
        });
    }
    Ok(keyed.into_iter().map(|item| item.entry.path).collect())
}
#[derive(Default)]
pub struct Navigation {
    pub files: Vec<PathBuf>,
    pub index: usize,
    /// Direction of the last move: 1 forward, -1 back, 0 none yet.
    direction: isize,
}
impl Navigation {
    pub fn set(&mut self, files: Vec<PathBuf>, current: &Path) {
        self.index = files.iter().position(|p| p == current).unwrap_or(0);
        self.files = files;
        self.direction = 0;
    }
    /// Moves by `delta`. At either end the position stays put, or with `wrap`
    /// continues at the other end.
    pub fn step(&mut self, delta: isize, wrap: bool) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        if delta != 0 {
            self.direction = delta.signum();
        }
        let last = self.files.len() - 1;
        self.index = if wrap && delta != 0 {
            let len = self.files.len() as isize;
            (self.index as isize + delta).rem_euclid(len) as usize
        } else {
            self.index.saturating_add_signed(delta).min(last)
        };
        self.files.get(self.index).cloned()
    }
    pub fn first(&mut self) -> Option<PathBuf> {
        self.index = 0;
        self.direction = 1;
        self.files.first().cloned()
    }
    pub fn last(&mut self) -> Option<PathBuf> {
        self.index = self.files.len().saturating_sub(1);
        self.direction = -1;
        self.files.last().cloned()
    }
    /// Preload candidates, most likely first: the next two in the direction
    /// of travel, or one on each side before the user has moved.
    pub fn neighbors(&self) -> Vec<PathBuf> {
        let offsets: [isize; 2] = match self.direction {
            1 => [1, 2],
            -1 => [-1, -2],
            _ => [1, -1],
        };
        offsets
            .into_iter()
            .filter_map(|offset| self.index.checked_add_signed(offset))
            .filter_map(|index| self.files.get(index).cloned())
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_numbers_do_not_overflow() {
        let mut names = vec!["image10.png", "image02.png", "image2.png", "image1.png"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec!["image1.png", "image2.png", "image02.png", "image10.png"]
        );
        assert_eq!(
            natural_cmp("a999999999999999999999", "a1000000000000000000000"),
            Ordering::Less
        );
    }
    #[test]
    fn extensions() {
        assert!(supported_extension(Path::new("猫.JpG")));
        assert!(supported_extension(Path::new("a.APNG")));
        assert!(!supported_extension(Path::new("a.jpg.exe")));
    }
    #[test]
    fn empty_and_boundaries() {
        let mut n = Navigation::default();
        assert!(n.step(1, false).is_none());
        assert!(n.first().is_none());
        assert!(n.last().is_none());
        n.set(vec!["1".into(), "2".into(), "3".into()], Path::new("2"));
        assert_eq!(n.step(1, false), Some("3".into()));
        assert_eq!(n.step(1, false), Some("3".into()));
        assert_eq!(n.first(), Some("1".into()));
        assert_eq!(n.step(-1, false), Some("1".into()));
        assert_eq!(n.last(), Some("3".into()));
    }
    #[test]
    fn wrap_around_continues_at_the_other_end() {
        let mut n = Navigation::default();
        n.set(vec!["1".into(), "2".into(), "3".into()], Path::new("3"));
        assert_eq!(n.step(1, true), Some("1".into()));
        assert_eq!(n.step(-1, true), Some("3".into()));
        assert_eq!(n.step(-1, true), Some("2".into()));
        // A zero step never wraps and never moves.
        assert_eq!(n.step(0, true), Some("2".into()));
        let mut single = Navigation::default();
        single.set(vec!["only".into()], Path::new("only"));
        assert_eq!(single.step(1, true), Some("only".into()));
    }
    #[test]
    fn preloads_follow_the_direction_of_travel() {
        let files: Vec<PathBuf> = ["1", "2", "3", "4", "5"].map(PathBuf::from).into();
        let mut n = Navigation::default();
        n.set(files, Path::new("3"));
        assert_eq!(n.neighbors(), vec![PathBuf::from("4"), "2".into()]);
        n.step(1, false);
        assert_eq!(n.neighbors(), vec![PathBuf::from("5")]);
        n.step(-1, false);
        assert_eq!(n.neighbors(), vec![PathBuf::from("2"), "1".into()]);
        n.first();
        assert_eq!(n.neighbors(), vec![PathBuf::from("2"), "3".into()]);
    }
    fn entries(names: &[&str]) -> Vec<Entry> {
        names
            .iter()
            .map(|name| Entry::new(PathBuf::from(name), None))
            .collect()
    }
    fn names(paths: Vec<PathBuf>) -> Vec<String> {
        paths
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }
    #[test]
    fn natural_sort_orders_numbers_by_value_and_supports_cancellation() {
        let generation = crate::security::Generation::default();
        let ticket = generation.next();
        let order = Order::default();
        let sorted = names(
            sort_entries(
                entries(&["Bild10.png", "bild2.png", "Bild1.png", "bild20.png"]),
                &ticket,
                order,
            )
            .unwrap(),
        );
        assert_eq!(
            sorted,
            ["Bild1.png", "bild2.png", "Bild10.png", "bild20.png"]
        );
        // Without natural sorting, names compare as plain text.
        let plain = Order {
            natural: false,
            ..order
        };
        let sorted =
            names(sort_entries(entries(&["b10.png", "b2.png", "b1.png"]), &ticket, plain).unwrap());
        assert_eq!(sorted, ["b1.png", "b10.png", "b2.png"]);
        generation.next();
        assert_eq!(
            sort_entries(entries(&["a.png"]), &ticket, order),
            Err(Error::Cancelled)
        );
    }
    #[cfg(windows)]
    #[test]
    fn umlauts_sort_next_to_their_base_letter_like_in_explorer() {
        let ticket = crate::security::Generation::default().next();
        let sorted = names(
            sort_entries(
                entries(&[
                    "zebra.png",
                    "\u{e4}pfel3.png",
                    "apfel12.png",
                    "\u{fc}ber.png",
                    "b.png",
                ]),
                &ticket,
                Order::default(),
            )
            .unwrap(),
        );
        // Byte order would put both umlaut names after "zebra".
        assert_eq!(
            sorted,
            [
                "\u{e4}pfel3.png",
                "apfel12.png",
                "b.png",
                "\u{fc}ber.png",
                "zebra.png"
            ]
        );
    }
    #[test]
    fn size_and_date_orders_fall_back_to_the_name() {
        let ticket = crate::security::Generation::default().next();
        let at = |secs| Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        let entry = |name: &str, secs, size| Entry {
            path: PathBuf::from(name),
            modified: at(secs),
            size,
        };
        let files = || {
            vec![
                entry("b.png", 20, 5),
                entry("a.png", 30, 5),
                entry("c.png", 10, 9),
            ]
        };
        let by = |by, descending| Order {
            by,
            descending,
            ..Order::default()
        };
        let sort = |order| names(sort_entries(files(), &ticket, order).unwrap());
        assert_eq!(
            sort(by(SortBy::Modified, false)),
            ["c.png", "b.png", "a.png"]
        );
        assert_eq!(
            sort(by(SortBy::Modified, true)),
            ["a.png", "b.png", "c.png"]
        );
        // Equal sizes keep name order; descending reverses everything.
        assert_eq!(sort(by(SortBy::Size, false)), ["a.png", "b.png", "c.png"]);
        assert_eq!(sort(by(SortBy::Size, true)), ["c.png", "b.png", "a.png"]);
        assert_eq!(sort(by(SortBy::Name, true)), ["c.png", "b.png", "a.png"]);
    }
}
