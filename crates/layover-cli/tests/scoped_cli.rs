//! What `explain` and `graph` say about a factory that scopes its routes.
//!
//! Driven through the binary, because the question is what an operator reads.

use std::path::PathBuf;
use std::process::Command;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-scoped-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        std::fs::write(path.join("layover.toml"), FACTORY).expect("writes the factory");
        Self(path)
    }

    fn layover(&self, args: &[&str]) -> (bool, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_layover"))
            .args(args)
            .arg("--config")
            .arg(self.0.join("layover.toml"))
            .output()
            .expect("the binary runs");
        (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Two workflows sharing `eagle`: only `devforge` may hand `bob` work from it.
const FACTORY: &str = r#"
[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "a"

[agents.azurix]
prompt = "z"

[agents.eagle]
prompt = "e"

[agents.bob]
prompt = "b"

[pipelines.devforge]
entry = "analyst"

[pipelines.eagle-eye]
entry = "azurix"

[[routes]]
from = "analyst"
to = "eagle"
pipelines = "devforge"

[[routes]]
from = "eagle"
to = "bob"
pipelines = "devforge"

[[routes]]
from = "azurix"
to = "eagle"
mode = "spawn"
pipelines = "eagle-eye"
"#;

#[test]
fn explain_names_each_routes_scope_and_what_each_pipeline_reaches() {
    let temp = Temp::new("explain");
    let (ok, out) = temp.layover(&["explain"]);

    assert!(ok, "{out}");
    assert!(
        out.contains("  eagle -> bob  [pipelines = devforge]"),
        "{out}"
    );
    assert!(
        out.contains("      reaches: azurix, eagle"),
        "a pipeline's reach, over its own routes: {out}"
    );
    assert!(out.contains("      reaches: analyst, bob, eagle"), "{out}");
}

#[test]
fn graph_can_draw_one_workflow() {
    let temp = Temp::new("graph");

    let (ok, mermaid) = temp.layover(&["graph", "--pipeline", "eagle-eye"]);
    assert!(ok, "{mermaid}");
    assert!(
        mermaid.contains("a_azurix -- spawn --> a_eagle"),
        "{mermaid}"
    );
    assert!(!mermaid.contains("a_bob"), "{mermaid}");

    let (ok, svg) = temp.layover(&["graph", "--svg", "--pipeline", "eagle-eye"]);
    assert!(ok, "{svg}");
    assert!(svg.contains(r#"id="a_eagle""#), "{svg}");
    assert!(!svg.contains(r#"id="a_bob""#), "{svg}");
}

#[test]
fn graph_of_the_whole_factory_labels_scoped_edges() {
    let temp = Temp::new("graph-all");
    let (ok, mermaid) = temp.layover(&["graph"]);

    assert!(ok, "{mermaid}");
    assert!(
        mermaid.contains(r#"a_eagle -->|"devforge"| a_bob"#),
        "{mermaid}"
    );
}

#[test]
fn graph_of_an_unknown_pipeline_says_which_exist() {
    let temp = Temp::new("graph-unknown");
    let (ok, out) = temp.layover(&["graph", "--pipeline", "nightly"]);

    assert!(!ok);
    assert!(out.contains("`nightly`"), "{out}");
    assert!(
        out.contains("devforge") && out.contains("eagle-eye"),
        "{out}"
    );
}
