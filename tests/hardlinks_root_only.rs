#![cfg(unix)]
#![cfg(feature = "cli")]

pub mod _utils;
pub use _utils::*;

use command_extra::CommandExtra;
use pipe_trait::Pipe;
use pretty_assertions::assert_eq;
use std::{
    path::Path,
    process::{Command, Stdio},
};

fn root_size(workspace: &Path, quantity: &str, extra: &[&str]) -> u64 {
    Command::new(PDU)
        .with_current_dir(workspace)
        .with_arg(format!("--quantity={quantity}"))
        .with_arg("--deduplicate-hardlinks")
        .with_arg("--json-output")
        .with_args(extra)
        .with_arg(".")
        .with_stdin(Stdio::null())
        .with_stdout(Stdio::piped())
        .with_stderr(Stdio::piped())
        .output()
        .expect("spawn command")
        .pipe(stdout_text)
        .pipe_as_ref(serde_json::from_str::<serde_json::Value>)
        .expect("parse stdout as JSON")["tree"]["size"]
        .as_u64()
        .expect("root size")
}

#[test]
fn root_only_counting_matches_path_based_deduplication() {
    let workspaces = [
        SampleWorkspace::simple_tree_with_some_hardlinks([
            200_000, 220_000, 310_000, 110_000, 210_000,
        ]),
        SampleWorkspace::multiple_hardlinks_to_a_single_file(100_000, 10),
        SampleWorkspace::complex_tree_with_shared_and_unique_files(15, 100_000),
    ];
    let root_only = [
        "--max-depth=1",
        "--omit-json-shared-details",
        "--omit-json-shared-summary",
    ];
    for workspace in &workspaces {
        for quantity in ["apparent-size", "block-size"] {
            assert_eq!(
                root_size(workspace, quantity, &root_only),
                root_size(workspace, quantity, &[]),
                "{quantity}",
            );
        }
    }
}
