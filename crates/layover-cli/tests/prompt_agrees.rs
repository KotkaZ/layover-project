//! `layover prompt` is the preview of what a run is told. It is only worth anything if a real run
//! is told the same thing.
//!
//! Driven through the binary on both sides, because the defect was exactly that the two paths
//! resolved flags differently: the preview honoured `--flag`, the run used the defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_store::Journal;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-agree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("prompts")).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn layover(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("the binary runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    text
}

#[test]
fn a_flag_set_at_trigger_time_composes_what_layover_prompt_previews() {
    let temp = Temp::new("flag");
    let site = &temp.0;

    std::fs::write(
        site.join("prompts").join("a.md"),
        "Do the work.\n@include(flag_x) on.md\n@include(!flag_x) off.md\n",
    )
    .expect("writes");
    std::fs::write(site.join("prompts").join("on.md"), "THE FLAG IS ON").expect("writes");
    std::fs::write(site.join("prompts").join("off.md"), "THE FLAG IS OFF").expect("writes");

    let echo = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };
    std::fs::write(
        site.join("layover.toml"),
        format!(
            r#"
[layover]
prompt_dir = "prompts"

[defaults]
runner = "shell"
timeout_sec = 60

[runners.shell]
command = {echo}

[agents.a]
prompt_file = "a.md"

[pipelines.p]
entry = "a"

[pipelines.p.flags]
flag_x = {{ default = false }}
"#
        ),
    )
    .expect("writes the factory");

    // What `POST /flights {"pipeline":"p","flags":{"flag_x":true}}` stores.
    Journal::open(site.join(".layover").join("journal"))
        .expect("opens")
        .queue(Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("a"),
                "hello",
                8,
            ),
            Some(PipelineName::new("p")),
            BTreeMap::from([("flag_x".to_owned(), true)]),
        ))
        .expect("queues");

    let preview = layover(
        site,
        &["prompt", "a", "--pipeline", "p", "--flag", "flag_x=true"],
    );
    let ran = layover(site, &["run"]);
    assert!(ran.contains("a succeeded"), "{ran}");

    let payload = std::fs::read_dir(site.join(".layover").join("hangars").join("a"))
        .expect("a Hangar")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("prompt.md"))
        .find(|path| path.exists())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .expect("a payload");

    assert!(preview.contains("THE FLAG IS ON"), "{preview}");
    assert!(
        payload.contains(preview.trim()),
        "the run was told something other than the preview.\npreview:\n{preview}\npayload:\n{payload}"
    );
}
