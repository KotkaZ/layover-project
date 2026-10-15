//! Swapping a binary in place — including one that is running — and putting it back.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use layover_update::swap::{install_lock, previous_path, restore, swap};
use layover_update::{Settings, Source, Version};

mod fake;
use fake::Temp;

fn v(text: &str) -> Version {
    text.parse().expect("valid")
}

fn name() -> &'static str {
    if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    }
}

/// A real program at `path`, started and kept running: the old binary, mid-run.
fn running_at(path: &Path) -> Child {
    let (program, args): (PathBuf, &[&str]) = if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        (
            PathBuf::from(root).join("System32").join("PING.EXE"),
            &["-n", "30", "127.0.0.1"],
        )
    } else {
        (PathBuf::from("/bin/sleep"), &["30"])
    };
    std::fs::copy(&program, path).expect("copies a real program");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmods");
    }
    Command::new(path)
        .args(args)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("starts")
}

#[test]
fn a_running_binary_is_replaced_and_kept_for_a_rollback() {
    let temp = Temp::new("swap-running");
    let binary = temp.0.join(name());
    let mut old = running_at(&binary);
    let before = std::fs::read(&binary).expect("reads");
    let staged = temp.0.join("staged");
    std::fs::write(&staged, b"the new release").expect("writes");

    let swapped = swap(&binary, &staged, &v("1.14.0")).expect("swaps under a running image");

    assert_eq!(std::fs::read(&binary).expect("reads"), b"the new release");
    assert_eq!(swapped.previous, previous_path(&binary, &v("1.14.0")));
    assert_eq!(std::fs::read(&swapped.previous).expect("kept"), before);
    assert!(
        old.try_wait().expect("asks").is_none(),
        "the old process runs on, from the image it started with"
    );

    restore(&swapped).expect("restores");
    assert_eq!(std::fs::read(&binary).expect("reads"), before);
    assert!(swapped.previous.exists(), "kept until forgotten on purpose");

    let _ = old.kill();
    let _ = old.wait();
}

#[test]
fn a_staged_file_that_is_not_there_leaves_the_binary_as_it_was() {
    let temp = Temp::new("swap-missing");
    let binary = temp.0.join(name());
    std::fs::write(&binary, b"old").expect("writes");

    let failed = swap(&binary, &temp.0.join("nowhere"), &v("1.14.0"));

    assert!(failed.is_err());
    assert_eq!(std::fs::read(&binary).expect("reads"), b"old");
    let left: Vec<_> = std::fs::read_dir(&temp.0)
        .expect("lists")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, [name()], "nothing left beside it");
}

#[test]
fn an_earlier_kept_copy_is_replaced_by_the_newer_one() {
    let temp = Temp::new("swap-again");
    let binary = temp.0.join(name());
    std::fs::write(&binary, b"1.14.0").expect("writes");
    std::fs::write(previous_path(&binary, &v("1.14.0")), b"stale").expect("writes");
    let staged = temp.0.join("staged");
    std::fs::write(&staged, b"1.15.0").expect("writes");

    let swapped = swap(&binary, &staged, &v("1.14.0")).expect("swaps");

    assert_eq!(std::fs::read(&swapped.previous).expect("kept"), b"1.14.0");
}

#[test]
fn two_installs_on_one_machine_never_run_at_once() {
    let temp = Temp::new("install-lock");
    let settings = Settings {
        switched_off: false,
        source: Source::mirror("http://127.0.0.1:9"),
        dir: temp.0.clone(),
    };

    let first = install_lock(&settings).expect("takes it");
    let second = install_lock(&settings);
    assert!(
        second
            .as_ref()
            .is_err_and(|why| why.contains("another Layover")),
        "{second:?}"
    );
    drop(first);
    assert!(install_lock(&settings).is_ok());
}
