//! Serialising writes to one file across the threads of one process.
//!
//! With runs in parallel, several threads write the same files: two runs of one agent append to
//! its memory, every run's tool calls append help requests and reports, and learnings are read,
//! changed and written back by runs and by the Tower at once. A read-modify-write that interleaves
//! with another loses whichever change was read first, and it loses it silently.
//!
//! One lock per path, shared by everything in the process that writes it. Held in a registry
//! rather than by whoever opened the file, because the same file is opened by several owners — the
//! Tower and the dashboard each open the journal — and a lock that belonged to one of them would
//! protect nothing against the other.
//!
//! This closes the in-process case, which is the one Layover creates. Two processes writing one
//! factory directory would still race, and that is a reason not to run two, not something this
//! guards.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Every lock handed out, by the absolute path it guards.
fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Mutex::default)
}

/// The lock that guards `path`: the same one for every caller in this process.
#[must_use]
pub fn for_path(path: &Path) -> Arc<Mutex<()>> {
    let key = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut locks = registry().lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(locks.entry(key).or_default())
}

/// Runs `act` while holding the lock for `path`.
///
/// A lock poisoned by a writer that panicked is taken anyway. It guards no data of its own, only
/// the order of writes to a file, and refusing every later write because one failed would turn a
/// single lost write into a factory that can write nothing.
pub fn exclusive<T>(path: &Path, act: impl FnOnce() -> T) -> T {
    let lock = for_path(path);
    let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
    act()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_caller_gets_the_same_lock_for_one_path() {
        let a = for_path(Path::new("some/dir/file.jsonl"));
        let b = for_path(Path::new("some/dir/file.jsonl"));
        let other = for_path(Path::new("some/dir/other.jsonl"));

        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &other));
    }

    #[test]
    fn a_read_modify_write_under_the_lock_loses_nothing() {
        let path = std::env::temp_dir().join(format!("layover-lock-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        std::thread::scope(|scope| {
            for n in 0..16 {
                let path = &path;
                scope.spawn(move || {
                    exclusive(path, || {
                        let mut text = std::fs::read_to_string(path).unwrap_or_default();
                        text.push_str(&n.to_string());
                        text.push('\n');
                        std::fs::write(path, text).expect("writes");
                    });
                });
            }
        });

        let lines = std::fs::read_to_string(&path)
            .expect("reads")
            .lines()
            .count();
        let _ = std::fs::remove_file(&path);
        assert_eq!(lines, 16);
    }
}
