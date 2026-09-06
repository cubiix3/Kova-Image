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
    let (a, b) = (l.as_bytes(), r.as_bytes());
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
            let x = l[si..i].trim_start_matches('0');
            let y = r[sj..j].trim_start_matches('0');
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
    a.len().cmp(&b.len()).then_with(|| left.cmp(right))
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
    if natural {
        entries.sort_unstable_by(|a, b| {
            natural_cmp(
                &a.file_name().unwrap_or_default().to_string_lossy(),
                &b.file_name().unwrap_or_default().to_string_lossy(),
            )
        });
    } else {
        entries.sort_unstable();
    }
    ticket.check()?;
    Ok(entries)
}
#[derive(Default)]
pub struct Navigation {
    pub files: Vec<PathBuf>,
    pub index: usize,
}
impl Navigation {
    pub fn set(&mut self, files: Vec<PathBuf>, current: &Path) {
        self.index = files.iter().position(|p| p == current).unwrap_or(0);
        self.files = files;
    }
    pub fn step(&mut self, delta: isize) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        self.index = self
            .index
            .saturating_add_signed(delta)
            .min(self.files.len() - 1);
        self.files.get(self.index).cloned()
    }
    pub fn first(&mut self) -> Option<PathBuf> {
        self.index = 0;
        self.files.first().cloned()
    }
    pub fn last(&mut self) -> Option<PathBuf> {
        self.index = self.files.len().saturating_sub(1);
        self.files.last().cloned()
    }
    pub fn neighbors(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(p) = self.files.get(self.index + 1) {
            paths.push(p.clone());
        }
        if self.index > 0
            && let Some(p) = self.files.get(self.index - 1)
        {
            paths.push(p.clone());
        }
        paths
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
}
