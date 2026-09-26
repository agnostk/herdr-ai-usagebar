//! Runs the plugin binary the way herdr does, against a fake `herdr` CLI that
//! answers with one Claude agent and records every metadata report, and the
//! demo's fake ai-usagebar. Covers the command dispatch and the background
//! refresher's lifecycle end to end.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const PLUGIN_ID: &str = "test.ai-usagebar";
const WAIT: Duration = Duration::from_secs(30);

const FAKE_HERDR: &str = r#"#!/bin/sh
dir="$FAKE_HERDR_DIR"
case "$1 $2" in
  "agent list")
    printf '%s' '{"result":{"agents":[{"agent":"claude","pane_id":"w1:p1","workspace_id":"w1"}]}}' ;;
  "workspace list")
    printf '%s' '{"result":{"workspaces":[{"workspace_id":"w1"}]}}' ;;
  "plugin list")
    enabled=true
    [ -e "$dir/disabled" ] && enabled=false
    printf '{"result":{"plugins":[{"plugin_id":"test.ai-usagebar","enabled":%s}]}}' "$enabled" ;;
  "config check")
    echo "config: ok" ;;
  *)
    if [ "$1" = "--help" ]; then
      echo "Config: $dir/herdr/config.toml"
    else
      echo "$*" >> "$dir/calls.log"
    fi ;;
esac
"#;

struct Plugin {
    dir: tempfile::TempDir,
}

impl Plugin {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["state", "config", "herdr", "bin"] {
            fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let herdr = dir.path().join("bin/herdr");
        fs::write(&herdr, FAKE_HERDR).unwrap();
        fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.path().join("stage"), "1").unwrap();
        let plugin = Self { dir };
        plugin.write_config("");
        plugin
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write_config(&self, extra: &str) {
        let fake_usage = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo/fake-ai-usagebar");
        fs::write(
            self.path("config/config.toml"),
            format!(
                "ai_usagebar = \"{}\"\nscan_secs = 1\n{extra}",
                fake_usage.display()
            ),
        )
        .unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-ai-usagebar"));
        command
            .args(args)
            .env("HERDR_PLUGIN_ID", PLUGIN_ID)
            .env("HERDR_PLUGIN_STATE_DIR", self.path("state"))
            .env("HERDR_PLUGIN_CONFIG_DIR", self.path("config"))
            .env("HERDR_SOCKET_PATH", self.path("herdr.sock"))
            .env("HERDR_BIN_PATH", self.path("bin/herdr"))
            .env("FAKE_HERDR_DIR", self.dir.path())
            .env("DEMO_STAGE_FILE", self.path("stage"))
            .env_remove("HERDR_CONFIG_PATH");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.path("calls.log")).unwrap_or_default()
    }

    fn wait_for_call(&self, needle: &str) {
        wait_until(
            || self.calls().contains(needle),
            || {
                format!(
                    "no herdr call containing {needle:?}; calls:\n{}",
                    self.calls()
                )
            },
        );
    }

    fn wait_until_stopped(&self) {
        wait_until(
            || self.stdout(&["status"]).contains("refresher: not running"),
            || "the refresher never stopped".into(),
        );
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
    }
}

fn wait_until(mut done: impl FnMut() -> bool, message: impl Fn() -> String) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{}", message());
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn refresh_start_status_and_stop_cover_the_refresher_lifecycle() {
    let plugin = Plugin::new();
    let started = plugin.stdout(&["refresh"]);
    assert!(
        started.contains("Started the background refresher"),
        "{started}"
    );

    plugin.wait_for_call("pane report-metadata w1:p1 --source plugin:test.ai-usagebar --token ai_usage=◑ 5h 38% · 7d 54% --token ai_usage_pct=54");
    plugin.wait_for_call(
        "workspace report-metadata w1 --source plugin:test.ai-usagebar --token ai_usage=◑ cld 54%",
    );

    let status = plugin.stdout(&["status"]);
    assert!(status.contains("refresher: running"), "{status}");
    assert!(status.contains("anthropic: ready"), "{status}");
    assert!(status.contains("demo/fake-ai-usagebar"), "{status}");

    let again = plugin.stdout(&["refresh"]);
    assert!(again.contains("Refresh requested"), "{again}");
    plugin.wait_for_call("--token ai_usage=◕ 5h 71% · 7d 58%");

    let stopped = plugin.stdout(&["stop"]);
    assert!(
        stopped.contains("Stopped the background refresher"),
        "{stopped}"
    );
    assert!(
        plugin
            .calls()
            .contains("pane report-metadata w1:p1 --source plugin:test.ai-usagebar --clear-token ai_usage --clear-token ai_usage_pct"),
        "{}",
        plugin.calls()
    );
    assert!(
        plugin
            .stdout(&["status"])
            .contains("refresher: not running")
    );
}

#[test]
fn start_is_idempotent_and_writes_the_example_config_once() {
    let plugin = Plugin::new();
    fs::remove_file(plugin.path("config/config.toml")).unwrap();
    assert!(plugin.run(&["start"]).status.success());
    assert!(plugin.run(&["start"]).status.success());
    wait_until(
        || plugin.path("config/config.toml").exists(),
        || "the refresher did not write the example config".into(),
    );
    let example = fs::read_to_string(plugin.path("config/config.toml")).unwrap();
    assert!(example.contains("# refresh_secs = 60"));
}

#[test]
fn disabling_the_plugin_clears_tokens_and_stops_the_refresher() {
    let plugin = Plugin::new();
    plugin.stdout(&["refresh"]);
    plugin.wait_for_call("--token ai_usage=");

    fs::write(plugin.path("disabled"), "").unwrap();
    plugin.stdout(&["refresh"]);
    plugin.wait_until_stopped();
    assert!(plugin.calls().contains("--clear-token ai_usage"));
}

#[test]
fn a_missing_ai_usagebar_is_shown_in_the_sidebar() {
    let plugin = Plugin::new();
    fs::write(
        plugin.path("config/config.toml"),
        "ai_usagebar = \"/nonexistent/ai-usagebar\"\nscan_secs = 1\n",
    )
    .unwrap();
    plugin.stdout(&["refresh"]);
    plugin.wait_for_call("--token ai_usage=⚠ ai-usagebar not found");
    let status = plugin.stdout(&["status"]);
    assert!(status.contains("ai-usagebar: not found"), "{status}");
}

#[test]
fn stop_without_a_refresher_clears_tokens_directly() {
    let plugin = Plugin::new();
    let output = plugin.stdout(&["stop"]);
    assert!(output.contains("was not running"), "{output}");
    assert!(plugin.calls().contains("pane report-metadata w1:p1"));
    assert!(plugin.calls().contains("workspace report-metadata w1"));
}

#[test]
fn setup_sidebar_edits_the_config_and_reloads_herdr() {
    let plugin = Plugin::new();
    fs::write(plugin.path("herdr/config.toml"), "onboarding = false\n").unwrap();

    let output = plugin.stdout(&["setup-sidebar"]);
    assert!(output.contains("Added the $ai_usage row"), "{output}");
    assert!(output.contains("Backup:"), "{output}");
    let config = fs::read_to_string(plugin.path("herdr/config.toml")).unwrap();
    assert!(config.contains("token = \"$ai_usage\""), "{config}");
    assert!(plugin.calls().contains("server reload-config"));

    let rerun = plugin.stdout(&["setup-sidebar"]);
    assert!(rerun.contains("already shows $ai_usage"), "{rerun}");
}

#[test]
fn setup_sidebar_with_both_row_kinds_off_adds_nothing() {
    let plugin = Plugin::new();
    plugin.write_config("agent_rows = false\nworkspace_rows = \"off\"\n");
    let output = plugin.stdout(&["setup-sidebar"]);
    assert!(output.contains("nothing to add"), "{output}");
    assert!(!plugin.path("herdr/config.toml").exists());
}

#[test]
fn dashboard_opens_the_plugin_pane() {
    let plugin = Plugin::new();
    plugin.stdout(&["dashboard"]);
    assert!(
        plugin
            .calls()
            .contains("plugin pane open --plugin test.ai-usagebar --entrypoint dashboard"),
        "{}",
        plugin.calls()
    );
}

#[test]
fn tui_runs_the_ai_usagebar_tui_beside_the_configured_binary() {
    let plugin = Plugin::new();
    let bin = plugin.path("bin");
    for (name, body) in [
        ("ai-usagebar", "exit 0"),
        ("ai-usagebar-tui", "echo tui ran"),
    ] {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        plugin.path("config/config.toml"),
        format!("ai_usagebar = \"{}\"\n", bin.join("ai-usagebar").display()),
    )
    .unwrap();
    assert_eq!(plugin.stdout(&["tui"]), "tui ran\n");
}

#[test]
fn tui_explains_a_missing_binary() {
    let plugin = Plugin::new();
    fs::write(
        plugin.path("config/config.toml"),
        "ai_usagebar = \"/nonexistent/ai-usagebar\"\n",
    )
    .unwrap();
    let output = plugin.run(&["tui"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ai-usagebar-tui was not found"), "{stderr}");
}

#[test]
fn commands_explain_a_missing_herdr_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-ai-usagebar"))
        .arg("refresh")
        .env_remove("HERDR_PLUGIN_ID")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("HERDR_PLUGIN_ID is not set"), "{stderr}");
}

#[test]
fn help_version_and_unknown_commands() {
    let binary = env!("CARGO_BIN_EXE_herdr-ai-usagebar");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("setup-sidebar"));

    let version = Command::new(binary).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("herdr-ai-usagebar {}\n", env!("CARGO_PKG_VERSION"))
    );

    let unknown = Command::new(binary).arg("frobnicate").output().unwrap();
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown command `frobnicate`"));
}
