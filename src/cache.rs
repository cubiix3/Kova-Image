use crate::decoder::{Decoded, Stamp};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Cache {
    entries: VecDeque<(PathBuf, Arc<Decoded>)>,
    budget: usize,
    used: usize,
}
impl Cache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            used: 0,
        }
    }
    pub fn used(&self) -> usize {
        self.used
    }
    pub fn get(&mut self, path: &Path, stamp: &Stamp) -> Option<Arc<Decoded>> {
        let index = self.entries.iter().position(|(p, _)| p == path)?;
        let entry = self.entries.remove(index)?;
        if &entry.1.stamp != stamp {
            self.used -= entry.1.weight();
            return None;
        }
        let result = entry.1.clone();
        self.entries.push_front(entry);
        Some(result)
    }
    pub fn remove(&mut self, path: &Path) {
        if let Some(i) = self.entries.iter().position(|(p, _)| p == path)
            && let Some((_, v)) = self.entries.remove(i)
        {
            self.used -= v.weight();
        }
    }
    pub fn insert(&mut self, path: PathBuf, value: Arc<Decoded>) {
        self.remove(&path);
        let weight = value.weight();
        if weight > self.budget {
            return;
        }
        while self.used.saturating_add(weight) > self.budget || self.entries.len() >= 32 {
            let Some((_, old)) = self.entries.pop_back() else {
                break;
            };
            self.used -= old.weight();
        }
        self.used += weight;
        self.entries.push_front((path, value));
    }
}
