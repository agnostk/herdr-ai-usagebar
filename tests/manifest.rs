//! Guards herdr-plugin.toml, which herdr only validates on link or install:
//! ids, versions, event names, and that every command runs a subcommand the
//! binary actually has. scripts/e2e.sh also links it into a real herdr.

use std::collections::HashSet;
use std::process::Command;

use toml::{Table, Value};

const BINARY: &str = "target/release/herdr-ai-usagebar";

/// herdr's lifecycle events (`herdr api schema --json`).
const KNOWN_EVENTS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.updated",
    "pane.closed",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "pane.output_matched",
    "pane.agent_status_changed",
    "pane.scroll_changed",
    "layout.updated",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
];

fn manifest() -> Table {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
        .expect("herdr-plugin.toml exists");
    text.parse().expect("herdr-plugin.toml is valid TOML")
}

fn entries<'a>(manifest: &'a Table, kind: &str) -> Vec<&'a Table> {
    manifest
        .get(kind)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_table).collect())
        .unwrap_or_default()
}

fn str_field<'a>(table: &'a Table, key: &str) -> &'a str {
    table
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("`{key}` is missing or not a string in {table:?}"))
}

fn command(table: &Table) -> Vec<&str> {
    table["command"]
        .as_array()
        .expect("command is an argv array")
        .iter()
        .map(|arg| arg.as_str().expect("argv entries are strings"))
        .collect()
}

fn binary_subcommands() -> HashSet<String> {
    let help = Command::new(env!("CARGO_BIN_EXE_herdr-ai-usagebar"))
        .arg("--help")
        .output()
        .unwrap();
    String::from_utf8(help.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

fn is_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".:_-".contains(c))
}

fn is_local_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":_-".contains(c))
}

#[test]
fn metadata_is_complete_and_matches_the_crate() {
    let manifest = manifest();
    assert!(is_plugin_id(str_field(&manifest, "id")));
    assert!(!str_field(&manifest, "name").is_empty());
    assert!(!str_field(&manifest, "description").is_empty());
    assert_eq!(str_field(&manifest, "version"), env!("CARGO_PKG_VERSION"));

    let min = str_field(&manifest, "min_herdr_version");
    let parts: Vec<&str> = min.split('.').collect();
    assert_eq!(parts.len(), 3, "min_herdr_version {min} is not x.y.z");
    assert!(parts.iter().all(|part| part.parse::<u32>().is_ok()));

    let platforms: Vec<&str> = manifest["platforms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|platform| platform.as_str().unwrap())
        .collect();
    assert!(!platforms.is_empty());
    assert!(
        platforms
            .iter()
            .all(|p| ["linux", "macos", "windows"].contains(p))
    );
}

#[test]
fn local_ids_are_valid_and_unique_per_kind() {
    let manifest = manifest();
    for kind in ["actions", "panes", "link_handlers"] {
        let mut seen = HashSet::new();
        for entry in entries(&manifest, kind) {
            let id = str_field(entry, "id");
            assert!(
                is_local_id(id),
                "{kind} id {id:?} has characters herdr rejects"
            );
            assert!(seen.insert(id), "duplicate {kind} id {id:?}");
        }
    }
}

#[test]
fn every_command_runs_a_subcommand_the_binary_has() {
    let manifest = manifest();
    let subcommands = binary_subcommands();
    assert!(subcommands.contains("setup-sidebar"), "{subcommands:?}");
    for kind in ["startup", "actions", "events", "panes"] {
        for entry in entries(&manifest, kind) {
            let argv = command(entry);
            assert_eq!(argv[0], BINARY, "{kind} runs {argv:?}");
            assert_eq!(argv.len(), 2, "{kind} runs {argv:?}");
            assert!(
                subcommands.contains(argv[1]),
                "{kind} runs unknown subcommand {:?}",
                argv[1]
            );
        }
    }
}

#[test]
fn actions_have_titles_descriptions_and_global_context() {
    for action in entries(&manifest(), "actions") {
        assert!(!str_field(action, "title").is_empty());
        assert!(!str_field(action, "description").is_empty());
        let contexts = action["contexts"].as_array().unwrap();
        assert_eq!(contexts, &vec![Value::from("global")]);
    }
}

#[test]
fn event_hooks_use_known_event_names() {
    for event in entries(&manifest(), "events") {
        let name = str_field(event, "on");
        assert!(KNOWN_EVENTS.contains(&name), "unknown herdr event {name:?}");
    }
}

#[test]
fn the_install_build_is_locked_and_release() {
    let manifest = manifest();
    let builds = entries(&manifest, "build");
    assert_eq!(builds.len(), 1);
    assert_eq!(
        command(builds[0]),
        ["cargo", "build", "--release", "--locked"]
    );
}

#[test]
fn panes_use_a_known_placement() {
    for pane in entries(&manifest(), "panes") {
        let placement = str_field(pane, "placement");
        assert!(
            ["overlay", "popup", "split", "tab", "zoomed"].contains(&placement),
            "unknown placement {placement:?}"
        );
    }
}
