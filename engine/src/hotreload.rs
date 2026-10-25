//! Watches files on disk so tuning values and shaders can be reloaded while the game runs.
//!
//! We watch the *parent directories* (non-recursively) rather than the files themselves because
//! most editors save by writing a temp file and renaming it over the original, which would
//! otherwise drop the watch.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct FileWatcher {
    rx: Receiver<notify::Result<notify::Event>>,
    watched: HashSet<PathBuf>,
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Watches the given files (they don't have to exist yet, but their directories do).
    pub fn new(files: &[&Path]) -> anyhow::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })?;
        let mut watched = HashSet::new();
        let mut dirs = HashSet::new();
        for file in files {
            let abs = absolute(file);
            if let Some(dir) = abs.parent() {
                dirs.insert(dir.to_path_buf());
            }
            watched.insert(abs);
        }
        for dir in &dirs {
            watcher.watch(dir, RecursiveMode::NonRecursive)?;
        }
        Ok(Self {
            rx,
            watched,
            _watcher: watcher,
        })
    }

    /// Watches every file in `dir` (non-recursively).
    pub fn new_dir(dir: &Path) -> anyhow::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })?;
        watcher.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Self {
            rx,
            watched: HashSet::new(),
            _watcher: watcher,
        })
    }

    /// Returns every watched file that changed since the last call (deduplicated).
    pub fn poll_changed(&self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            let Ok(event) = event else { continue };
            if !(event.kind.is_modify() || event.kind.is_create()) {
                continue;
            }
            for path in event.paths {
                let path = absolute(&path);
                let interesting = self.watched.is_empty() || self.watched.contains(&path);
                if interesting && !changed.contains(&path) {
                    changed.push(path);
                }
            }
        }
        changed
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
