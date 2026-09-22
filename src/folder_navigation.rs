use crate::{error::Error, security::Ticket};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

pub const EXTENSIONS: &[&str] = crate::media::IMAGE_EXTENSIONS;
pub fn supported_extension(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|s| {
        EXTENSIONS
            .iter()
            .chain(crate::media::VIDEO_EXTENSIONS)
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
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
pub fn scan(path: &Path, ticket: &Ticket, natural: bool) -> Result<Vec<PathBuf>, Error> {
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
            entries.push(p);
        }
        if entries.len() >= 100_000 {
            return Err(Error::Io("Folder exceeds 100,000 supported entries".into()));
        }
    }
    // Include an explicitly opened image with an unusual extension.
    if !entries.iter().any(|p| p == path) {
        entries.push(path.to_path_buf());
    }
    let entries = sort_entries(entries, ticket, natural)?;
    ticket.check()?;
    Ok(entries)
}
fn sort_entries(
    mut entries: Vec<PathBuf>,
    ticket: &Ticket,
    natural: bool,
) -> Result<Vec<PathBuf>, Error> {
    if !natural {
        entries.sort_unstable();
        return Ok(entries);
    }
    let mut keyed = Vec::with_capacity(entries.len());
    for path in entries {
        ticket.check()?;
        let folded = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        keyed.push((path, folded));
    }
    keyed.sort_unstable_by(|(left_path, left_key), (right_path, right_key)| {
        natural_cmp_folded(left_key, right_key).then_with(|| {
            left_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .cmp(&right_path.file_name().unwrap_or_default().to_string_lossy())
        })
    });
    Ok(keyed.into_iter().map(|(path, _)| path).collect())
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
    pub fn step(&mut self, delta: isize) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        if delta != 0 {
            self.direction = delta.signum();
        }
        self.index = self
            .index
            .saturating_add_signed(delta)
            .min(self.files.len() - 1);
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
        assert!(n.step(1).is_none());
        assert!(n.first().is_none());
        assert!(n.last().is_none());
        n.set(vec!["1".into(), "2".into(), "3".into()], Path::new("2"));
        assert_eq!(n.step(1), Some("3".into()));
        assert_eq!(n.step(1), Some("3".into()));
        assert_eq!(n.first(), Some("1".into()));
        assert_eq!(n.step(-1), Some("1".into()));
        assert_eq!(n.last(), Some("3".into()));
    }
    #[test]
    fn preloads_follow_the_direction_of_travel() {
        let files: Vec<PathBuf> = ["1", "2", "3", "4", "5"].map(PathBuf::from).into();
        let mut n = Navigation::default();
        n.set(files, Path::new("3"));
        assert_eq!(n.neighbors(), vec![PathBuf::from("4"), "2".into()]);
        n.step(1);
        assert_eq!(n.neighbors(), vec![PathBuf::from("5")]);
        n.step(-1);
        assert_eq!(n.neighbors(), vec![PathBuf::from("2"), "1".into()]);
        n.first();
        assert_eq!(n.neighbors(), vec![PathBuf::from("2"), "3".into()]);
    }
    #[test]
    fn cached_sort_preserves_natural_order_and_cancellation() {
        let names: Vec<PathBuf> = [
            "Bild10.png",
            "bild2.png",
            "Bild02.png",
            "Äpfel3.png",
            "äpfel12.png",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        let mut expected = names.clone();
        expected.sort_unstable_by(|a, b| {
            natural_cmp(
                &a.file_name().unwrap_or_default().to_string_lossy(),
                &b.file_name().unwrap_or_default().to_string_lossy(),
            )
        });
        let generation = crate::security::Generation::default();
        let ticket = generation.next();
        let sorted = sort_entries(names.clone(), &ticket, true).unwrap();
        assert_eq!(expected, sorted);
        generation.next();
        assert_eq!(sort_entries(names, &ticket, true), Err(Error::Cancelled));
    }
}
