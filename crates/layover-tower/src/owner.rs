//! Which Tower a live run belongs to, and whether that Tower is still alive.
//!
//! A Tower settling the runs it finds recorded as live must never touch a run another Tower is
//! still watching — `layover run` beside a `serve`, or a second Tower started before the first
//! has gone. Stopping those would be recovery causing the very interruption it exists to repair.
//!
//! So each Tower holds an exclusive lock on a file of its own for as long as it lives, and each
//! live record names that file. The operating system drops the lock when the process ends, however
//! it ends, which is precisely the question: a lock nobody holds belongs to a Tower that is gone.
//! No process identifier is compared, so none can be mistaken for a reused one.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;

/// Tells apart Towers one process opens in the same instant, which tests do.
static CLAIMED: AtomicU64 = AtomicU64::new(0);

/// A Tower's claim on the runs it starts, held until it is dropped.
#[derive(Debug)]
pub(crate) struct Owner {
    id: String,
    path: PathBuf,
    file: File,
}

impl Owner {
    /// Claims a fresh identity in `root`, the directory live records are kept in.
    pub(crate) fn claim(root: &Path) -> io::Result<Self> {
        let id = format!(
            "tower-{}-{}-{}",
            std::process::id(),
            Timestamp::now().as_millisecond(),
            CLAIMED.fetch_add(1, Ordering::Relaxed)
        );
        let path = lock_path(root, &id);
        fs::create_dir_all(path.parent().unwrap_or(root))?;

        let file = File::create(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { id, path, file }),
            Err(TryLockError::WouldBlock) => Err(io::Error::other(format!(
                "{} is already held",
                path.display()
            ))),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// The name live records carry.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Whether the Tower that claimed `id` in `root` is still alive.
    ///
    /// When the lock cannot be tested for any reason but its absence, the answer is yes. That
    /// leaves the run alone, which is the safe direction: a run left alone is visible and can be
    /// settled on the next restart, while one stopped under a living Tower is work destroyed.
    pub(crate) fn is_alive(root: &Path, id: &str) -> bool {
        let file = match File::open(lock_path(root, id)) {
            Ok(file) => file,
            Err(error) => return error.kind() != io::ErrorKind::NotFound,
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => true,
        }
    }

    /// Removes what a Tower found gone left behind.
    pub(crate) fn clear(root: &Path, id: &str) {
        let _ = fs::remove_file(lock_path(root, id));
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_path(root: &Path, id: &str) -> PathBuf {
    root.join("owners").join(format!("{id}.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-owner-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_tower_that_holds_its_claim_is_alive_to_everyone_else() {
        let temp = Temp::new("held");
        let owner = Owner::claim(&temp.0).expect("claims");

        assert!(Owner::is_alive(&temp.0, owner.id()));
    }

    #[test]
    fn a_tower_that_let_go_is_gone() {
        // Dropping is what a clean shutdown does; a crash drops the lock the same way, because the
        // operating system releases it with the process.
        let temp = Temp::new("gone");
        let id = Owner::claim(&temp.0).expect("claims").id().to_owned();

        assert!(!Owner::is_alive(&temp.0, &id));
    }

    #[test]
    fn a_claim_whose_file_is_still_there_but_unlocked_is_gone() {
        // What a killed Tower leaves: the file, and nobody holding it.
        let temp = Temp::new("stale");
        let path = lock_path(&temp.0, "tower-dead");
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(&path, "").expect("writes");

        assert!(!Owner::is_alive(&temp.0, "tower-dead"));
        Owner::clear(&temp.0, "tower-dead");
        assert!(!path.exists());
    }

    #[test]
    fn two_towers_in_one_process_do_not_share_a_claim() {
        let temp = Temp::new("two");
        let first = Owner::claim(&temp.0).expect("claims");
        let second = Owner::claim(&temp.0).expect("claims");

        assert_ne!(first.id(), second.id());
        assert!(Owner::is_alive(&temp.0, first.id()));
        assert!(Owner::is_alive(&temp.0, second.id()));
    }
}
