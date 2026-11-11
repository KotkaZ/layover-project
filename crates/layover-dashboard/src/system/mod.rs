//! The machine the factory runs on: how busy its processors are, how much memory and disk space
//! it has left, and how much of each is Layover's.
//!
//! A thread reads the machine every five seconds and keeps what it read in memory, the last hour
//! as read and the last day as minute averages, so the graphs have a past the moment the tab is
//! opened. Nothing is written to disk: a restart begins them again, and a factory's state
//! directory holds only what is the factory's.
//!
//! Layover's share is this process and everything it started — under `serve`, the Tower and
//! every agent CLI it runs — and every agent run another Layover process has alive now, read
//! from the live records every Tower keeps. So a `--watch-only` dashboard counts the agents of the
//! Tower beside it, though not that Tower's own process.

mod history;
mod reading;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_http::{SystemDisk, SystemReport, SystemSample, SystemWindow};
use layover_store::live::Ledger;
use sysinfo::{Disks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use history::{History, Point};
use reading::Proc;

/// How often the machine is read.
const EVERY: Duration = Duration::from_secs(5);

/// A drive holding the factory or one of its working directories.
#[derive(Debug, Clone)]
struct Drive {
    name: String,
    mount_point: PathBuf,
    total: u64,
    holds: Vec<&'static str>,
}

/// What the machine is: found once, by the reading thread.
#[derive(Debug)]
struct Machine {
    cpus: usize,
    memory_total: u64,
    drives: Vec<Drive>,
}

/// Reads the machine in the background, and says what it read.
#[derive(Debug)]
pub struct Monitor {
    since: Timestamp,
    every: Duration,
    /// Unknown until the reading thread has looked, and for good on a machine it cannot read.
    machine: OnceLock<Machine>,
    history: Mutex<History>,
}

impl Monitor {
    /// Starts reading the machine every five seconds, for the factory whose configuration is at
    /// `config_path`.
    #[must_use]
    pub fn start(config_path: &Path) -> Arc<Self> {
        Self::start_every(config_path, EVERY)
    }

    /// Starts reading the machine every `every`, which a test makes short. Reading stops once
    /// nothing holds the monitor any more.
    ///
    /// Returns at once: the first look at the machine — every process, every drive — is taken on
    /// the reading thread, so a Tower starts no later for it. A Tower that took longer to start
    /// would hold on longer to a `layover stop` asked of it as it started, and then discard it as
    /// a request left by a Tower that went away.
    #[must_use]
    pub fn start_every(config_path: &Path, every: Duration) -> Arc<Self> {
        let monitor = Arc::new(Self {
            since: Timestamp::now(),
            every,
            machine: OnceLock::new(),
            history: Mutex::new(History::default()),
        });
        let watching = Arc::downgrade(&monitor);
        let config_path = config_path.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("layover-machine".to_owned())
            .spawn(move || {
                if let Some(sampler) = Sampler::open(watching, &config_path, every) {
                    sampler.run();
                }
            });
        monitor
    }

    /// What was read over `window`.
    pub(crate) fn report(&self, window: SystemWindow) -> SystemReport {
        let machine = self.machine.get();
        let history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        let (every, points) = match window {
            SystemWindow::LastHour => (self.every.as_secs().max(1), history.last_hour()),
            SystemWindow::Last24h => (60, history.last_day()),
        };
        SystemReport {
            window,
            every_sec: i32::try_from(every).unwrap_or(i32::MAX),
            since: self.since.to_string(),
            cpus: i32::try_from(machine.map_or(0, |machine| machine.cpus)).unwrap_or(i32::MAX),
            memory_total_bytes: bytes(machine.map_or(0, |machine| machine.memory_total)),
            disks: machine
                .map(|machine| machine.drives.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|drive| SystemDisk {
                    name: drive.name.clone(),
                    mount_point: drive.mount_point.display().to_string(),
                    total_bytes: bytes(drive.total),
                    holds: drive.holds.iter().map(|&what| what.to_owned()).collect(),
                })
                .collect(),
            samples: points.iter().map(sample).collect(),
        }
    }

    fn record(&self, point: Point) {
        self.history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(point);
    }
}

/// What the reading thread owns: the readers, and where the live runs are recorded.
struct Sampler {
    monitor: Weak<Monitor>,
    system: System,
    disks: Option<Disks>,
    runs: Ledger,
    every: Duration,
}

impl Sampler {
    /// Looks at the machine once, and says what it is. Nothing, on a machine `sysinfo` cannot
    /// read: the monitor then reports no samples, for good.
    fn open(monitor: Weak<Monitor>, config_path: &Path, every: Duration) -> Option<Self> {
        let mut system = System::new().ok()?;
        system.refresh_memory();
        system.refresh_cpu_usage();
        refresh_processes(&mut system);
        let disks = Disks::new_with_refreshed_list().ok();

        let root = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let machine = Machine {
            cpus: system.cpus().len(),
            memory_total: system.total_memory(),
            drives: drives(&root, config_path, disks.as_deref().unwrap_or_default()),
        };
        let _ = monitor.upgrade()?.machine.set(machine);
        Some(Self {
            monitor,
            system,
            disks,
            runs: Ledger::at(root.join(".layover").join("state").join("runs")),
            every,
        })
    }

    fn run(mut self) {
        loop {
            std::thread::sleep(self.every);
            let Some(monitor) = self.monitor.upgrade() else {
                return;
            };
            let Some(machine) = monitor.machine.get() else {
                return;
            };
            let point = self.read(machine);
            monitor.record(point);
        }
    }

    fn read(&mut self, machine: &Machine) -> Point {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        refresh_processes(&mut self.system);
        if let Some(disks) = &mut self.disks {
            disks.refresh(false);
        }

        let processes: Vec<Proc> = self
            .system
            .processes()
            .values()
            .map(|process| Proc {
                pid: process.pid().as_u32(),
                parent: process.parent().map(Pid::as_u32),
                started: process.start_time(),
                cpu: f64::from(process.cpu_usage()),
                memory: process.memory(),
            })
            .collect();
        let mut roots = vec![std::process::id()];
        roots.extend(
            self.runs
                .live()
                .unwrap_or_default()
                .iter()
                .map(|live| live.pid),
        );
        let share = reading::share(&processes, &roots, machine.cpus);

        let used = |drive: &Drive| {
            self.disks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|disk| disk.mount_point() == drive.mount_point)
                .map_or(0, |disk| {
                    disk.total_space().saturating_sub(disk.available_space())
                })
        };
        Point {
            at: Timestamp::now(),
            cpu: f64::from(self.system.global_cpu_usage()).clamp(0.0, 100.0),
            layover_cpu: share.cpu,
            memory: self.system.used_memory(),
            layover_memory: share.memory,
            layover_processes: share.processes,
            disks: machine.drives.iter().map(used).collect(),
        }
    }
}

/// Processes with their processor time and memory, and without Linux's threads, which share
/// their process's memory and would count it again.
fn refresh_processes(system: &mut System) {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .without_tasks(),
    );
}

/// The drives holding the factory and every working directory it names, each once.
fn drives(root: &Path, config_path: &Path, disks: &[sysinfo::Disk]) -> Vec<Drive> {
    let mut places: Vec<(PathBuf, &'static str)> = vec![(root.to_path_buf(), "factory")];
    if let Ok(config) = Config::load(config_path) {
        places.push((root.join(&config.layover.work_dir), "work_dir"));
        places.extend(
            config
                .agents
                .values()
                .filter_map(|agent| agent.work_dir.as_ref())
                .map(|dir| (root.join(dir), "work_dir")),
        );
    }

    let mounts: Vec<PathBuf> = disks
        .iter()
        .map(|disk| disk.mount_point().to_path_buf())
        .collect();
    let mut drives: Vec<(usize, Drive)> = Vec::new();
    for (place, what) in places {
        let Some(index) = reading::holder(&reading::resolved(&place), &mounts) else {
            continue;
        };
        if let Some((_, drive)) = drives.iter_mut().find(|(at, _)| *at == index) {
            if !drive.holds.contains(&what) {
                drive.holds.push(what);
            }
            continue;
        }
        let disk = &disks[index];
        drives.push((
            index,
            Drive {
                name: disk.name().to_string_lossy().into_owned(),
                mount_point: disk.mount_point().to_path_buf(),
                total: disk.total_space(),
                holds: vec![what],
            },
        ));
    }
    drives.into_iter().map(|(_, drive)| drive).collect()
}

fn sample(point: &Point) -> SystemSample {
    SystemSample {
        at: point.at.to_string(),
        cpu_percent: round(point.cpu),
        layover_cpu_percent: round(point.layover_cpu),
        memory_used_bytes: bytes(point.memory),
        layover_memory_bytes: bytes(point.layover_memory),
        layover_processes: i32::try_from(point.layover_processes).unwrap_or(i32::MAX),
        disk_used_bytes: point.disks.iter().copied().map(bytes).collect(),
    }
}

fn bytes(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A percentage to a tenth, which is as fine as a graph is read and keeps the response small.
fn round(percent: f64) -> f64 {
    (percent * 10.0).round() / 10.0
}
