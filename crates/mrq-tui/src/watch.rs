//! Live reload: watching the open files for changes on disk. The [`Watch`] seam is what
//! the app tells about each file it opens; [`FileWatcher`] implements it with `notify`
//! and reports changed files to its [`Changes`], which debounces them into one
//! [`Event::Changed`](crate::Event::Changed) per burst for the event loop.
//!
//! Each file's directory is watched rather than the file, so a save that writes a
//! temporary file and renames it over the original (as `w` does, and many editors) is
//! still seen, and so is a file that is deleted and created again. Events are matched
//! by canonical path and reported under the path the app knows the file by. Whether a
//! change is news (our own `w`, a `touch`) is the app's call: it compares the text.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// How long a file must be quiet after a change before it is reported: a save often
/// comes as several events (write, rename, metadata).
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Watches files for changes. Tests substitute one that records the paths.
pub trait Watch: fmt::Debug {
    /// Watches the file `path` from now on; watching it again does nothing.
    fn watch(&mut self, path: &Path) -> io::Result<()>;
}

/// The watched files by canonical path, with the paths the app knows them by.
type Files = Arc<Mutex<HashMap<PathBuf, PathBuf>>>;

/// A [`Watch`] on the file system with `notify`; see the [module docs](self).
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    files: Files,
    /// The directories watched.
    dirs: HashSet<PathBuf>,
}

impl fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileWatcher")
            .field("dirs", &self.dirs)
            .finish_non_exhaustive()
    }
}

impl FileWatcher {
    /// A watcher watching nothing yet, and the changes it reports.
    pub fn new() -> notify::Result<(Self, Changes)> {
        let files = Files::default();
        let (sender, receiver) = mpsc::channel();
        let known = Arc::clone(&files);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            if event.kind.is_access() {
                return;
            }
            let known = known.lock().unwrap_or_else(PoisonError::into_inner);
            for path in &event.paths {
                if let Some(file) = known.get(path).or_else(|| known.get(&canonical(path)?)) {
                    // The receiver is gone only when the viewer is.
                    let _ = sender.send(file.clone());
                }
            }
        })?;
        let watcher = Self {
            watcher,
            files,
            dirs: HashSet::new(),
        };
        let changes = Changes {
            receiver,
            debounce: Debounce::default(),
        };
        Ok((watcher, changes))
    }
}

impl Watch for FileWatcher {
    fn watch(&mut self, path: &Path) -> io::Result<()> {
        let key = canonical(path).ok_or_else(|| io::Error::other("not a file path"))?;
        let dir = key
            .parent()
            .expect("a canonical file path has a parent")
            .to_owned();
        if !self.dirs.contains(&dir) {
            self.watcher
                .watch(&dir, RecursiveMode::NonRecursive)
                .map_err(io::Error::other)?;
            self.dirs.insert(dir);
        }
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, path.to_owned());
        Ok(())
    }
}

/// `path` with its directory canonicalized (the file itself may be gone): how the
/// watcher matches event paths, which can come through other symlinks (`/var` and
/// `/private/var` on macOS).
fn canonical(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    Some(fs::canonicalize(dir).ok()?.join(name))
}

/// The changed files a [`FileWatcher`] reports, debounced.
#[derive(Debug)]
pub struct Changes {
    receiver: Receiver<PathBuf>,
    debounce: Debounce,
}

impl Changes {
    /// A file that changed and has been quiet for [`DEBOUNCE`] at `now`, if any; it is
    /// reported once per burst of changes. Changes are timed when they are taken from the
    /// watcher, here, or at `now` if that is earlier.
    pub fn next(&mut self, now: Instant) -> Option<PathBuf> {
        while let Ok(path) = self.receiver.try_recv() {
            self.debounce.push(path, Instant::now().min(now));
        }
        self.debounce.ready(now)
    }
}

/// Changed paths with the time of their latest change, the earliest first.
#[derive(Debug, Default)]
struct Debounce {
    pending: Vec<(PathBuf, Instant)>,
}

impl Debounce {
    /// Notes that `path` changed at `at`.
    fn push(&mut self, path: PathBuf, at: Instant) {
        self.pending.retain(|(pending, _)| *pending != path);
        self.pending.push((path, at));
    }

    /// Takes a path quiet since [`DEBOUNCE`] before `now`.
    fn ready(&mut self, now: Instant) -> Option<PathBuf> {
        let index = self
            .pending
            .iter()
            .position(|(_, at)| now.saturating_duration_since(*at) >= DEBOUNCE)?;
        Some(self.pending.remove(index).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_changes_is_one_change_once_it_is_quiet() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut debounce = Debounce::default();
        debounce.push("a.md".into(), at(0));
        debounce.push("a.md".into(), at(50));
        assert_eq!(debounce.ready(at(120)), None);
        debounce.push("b.md".into(), at(130));
        assert_eq!(debounce.ready(at(150)), Some("a.md".into()));
        assert_eq!(debounce.ready(at(150)), None);
        assert_eq!(debounce.ready(at(230)), Some("b.md".into()));
        assert_eq!(debounce.ready(at(1000)), None);
    }

    #[test]
    fn changes_come_through_the_channel() {
        let (sender, receiver) = mpsc::channel();
        let mut changes = Changes {
            receiver,
            debounce: Debounce::default(),
        };
        sender.send(PathBuf::from("a.md")).unwrap();
        let now = Instant::now();
        assert_eq!(changes.next(now), None);
        assert_eq!(changes.next(now + DEBOUNCE), Some("a.md".into()));
        assert_eq!(changes.next(now + DEBOUNCE * 2), None);
    }

    #[test]
    fn paths_are_matched_through_their_canonical_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.md");
        let canonical_dir = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(canonical(&path), Some(canonical_dir.join("gone.md")));
        assert_eq!(canonical(&dir.path().join("no/such.md")), None);
    }
}
