//! Which processes are Layover's, and which drive holds a path: the parts of reading the machine
//! that are decided from what was read rather than read themselves.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A process as the sampler read it.
#[derive(Debug, Clone)]
pub(crate) struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    /// When it started, in seconds since the epoch.
    pub started: u64,
    /// Processor time, as a percentage of one processor.
    pub cpu: f64,
    /// Physical memory it holds, in bytes.
    pub memory: u64,
}

/// What Layover's processes use.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct Share {
    /// A share of every processor, 0 to 100.
    pub cpu: f64,
    pub memory: u64,
    pub processes: u32,
}

/// What the trees rooted at `roots` use, each process counted once.
///
/// A child counts only if it started no earlier than its parent: Windows reuses the identifier of
/// a process that has ended, so a process an unrelated one started can name the same parent as
/// one of Layover's.
pub(crate) fn share(processes: &[Proc], roots: &[u32], cpus: usize) -> Share {
    let by_pid: HashMap<u32, &Proc> = processes.iter().map(|proc| (proc.pid, proc)).collect();
    let mut children: HashMap<u32, Vec<&Proc>> = HashMap::new();
    for proc in processes {
        if let Some(parent) = proc.parent.filter(|parent| *parent != proc.pid) {
            children.entry(parent).or_default().push(proc);
        }
    }

    let mut seen = HashSet::new();
    let mut open: Vec<&Proc> = roots
        .iter()
        .filter_map(|pid| by_pid.get(pid).copied())
        .collect();
    let mut share = Share::default();
    while let Some(proc) = open.pop() {
        if !seen.insert(proc.pid) {
            continue;
        }
        share.cpu += proc.cpu;
        share.memory += proc.memory;
        share.processes += 1;
        open.extend(
            children
                .get(&proc.pid)
                .into_iter()
                .flatten()
                .filter(|child| child.started >= proc.started),
        );
    }
    let cpus = u32::try_from(cpus.max(1)).unwrap_or(u32::MAX);
    share.cpu = (share.cpu / f64::from(cpus)).clamp(0.0, 100.0);
    share
}

/// Which of `mounts` holds `path`: the deepest one it lies under.
pub(crate) fn holder(path: &Path, mounts: &[PathBuf]) -> Option<usize> {
    mounts
        .iter()
        .enumerate()
        .filter(|(_, mount)| path.starts_with(mount))
        .max_by_key(|(_, mount)| mount.components().count())
        .map(|(index, _)| index)
}

/// `path` as mount points are named: the nearest part of it that exists, resolved, without the
/// `\\?\` Windows puts in front of a resolved path.
pub(crate) fn resolved(path: &Path) -> PathBuf {
    let mut at = Some(path);
    while let Some(part) = at {
        if let Ok(real) = std::fs::canonicalize(part) {
            return plain(&real);
        }
        at = part.parent();
    }
    path.to_path_buf()
}

fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    text.strip_prefix(r"\\?\")
        .map_or_else(|| path.to_path_buf(), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, started: u64, cpu: f64, memory: u64) -> Proc {
        Proc {
            pid,
            parent,
            started,
            cpu,
            memory,
        }
    }

    #[test]
    fn a_tree_is_counted_whole_and_each_process_once() {
        let processes = [
            proc(10, Some(1), 100, 40.0, 1_000),
            proc(11, Some(10), 110, 80.0, 2_000),
            proc(12, Some(10), 120, 20.0, 3_000),
            proc(13, Some(12), 130, 60.0, 4_000),
            proc(20, Some(1), 100, 400.0, 9_000),
        ];
        // The Tower, and a live run it started: one tree, named twice.
        let share = share(&processes, &[10, 12, 99], 4);
        assert_eq!(share.processes, 4);
        assert_eq!(share.memory, 10_000);
        assert!((share.cpu - 50.0).abs() < 1e-9, "{share:?}");
    }

    #[test]
    fn a_process_older_than_the_parent_it_names_is_not_counted() {
        let processes = [
            proc(10, Some(1), 500, 10.0, 1_000),
            // Its parent ended long ago, and the Tower was given the same identifier since.
            proc(30, Some(10), 400, 90.0, 8_000),
        ];
        let share = share(&processes, &[10], 1);
        assert_eq!(share.processes, 1);
        assert_eq!(share.memory, 1_000);
    }

    #[test]
    fn a_share_never_exceeds_the_machine() {
        let processes = [proc(10, None, 1, 900.0, 1)];
        assert!((share(&processes, &[10], 2).cpu - 100.0).abs() < 1e-9);
        assert_eq!(share(&processes, &[11], 2), Share::default());
    }

    #[test]
    fn the_deepest_mount_holding_a_path_is_its_drive() {
        let mounts = [
            PathBuf::from("/"),
            PathBuf::from("/home"),
            PathBuf::from("/home2"),
        ];
        assert_eq!(holder(Path::new("/home/karl/factory"), &mounts), Some(1));
        assert_eq!(holder(Path::new("/srv/factory"), &mounts), Some(0));
        assert_eq!(holder(Path::new("relative"), &mounts), None);
    }

    #[test]
    fn a_path_that_does_not_exist_yet_is_held_where_its_nearest_part_is() {
        let here = resolved(Path::new("."));
        let later = resolved(&here.join("not-made-yet").join("workspace"));
        assert_eq!(later, here);
        assert!(!here.to_string_lossy().starts_with(r"\\?\"));
    }
}
