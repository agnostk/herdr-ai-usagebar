//! herdr access through its CLI (`HERDR_BIN_PATH`), which herdr documents as
//! the portable plugin API.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::plan::{Agent, Layout, Target, Tokens};
use crate::proc;
use crate::publisher::Sink;

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Enabled,
    Disabled,
    Missing,
}

pub struct Herdr {
    bin: PathBuf,
    source: String,
    last_seq: AtomicU64,
}

impl Herdr {
    pub fn new(bin: PathBuf, plugin_id: &str) -> Self {
        Self {
            bin,
            source: format!("plugin:{plugin_id}"),
            last_seq: AtomicU64::new(0),
        }
    }

    fn run(&self, args: &[String]) -> Result<String> {
        let output = proc::run(Command::new(&self.bin).args(args), CALL_TIMEOUT)?;
        if !output.status.success() {
            let message = if output.stderr.trim().is_empty() {
                output.stdout.trim().to_string()
            } else {
                output.stderr.trim().to_string()
            };
            bail!(
                "herdr {}: {message}",
                args.first().map_or("", String::as_str)
            );
        }
        Ok(output.stdout)
    }

    fn run_json(&self, args: &[&str]) -> Result<Value> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let stdout = self.run(&args)?;
        serde_json::from_str(&stdout).context("herdr returned invalid JSON")
    }

    pub fn layout(&self) -> Result<Layout> {
        let agents = parse_agents(&self.run_json(&["agent", "list"])?)?;
        let workspace_ids = parse_workspace_ids(&self.run_json(&["workspace", "list"])?)?;
        Ok(Layout {
            agents,
            workspace_ids,
        })
    }

    pub fn plugin_state(&self, plugin_id: &str) -> Result<PluginState> {
        let list = self.run_json(&["plugin", "list", "--plugin", plugin_id, "--json"])?;
        Ok(parse_plugin_state(&list, plugin_id))
    }

    pub fn reload_config(&self) -> Result<()> {
        self.run(&["server".into(), "reload-config".into()])
            .map(drop)
    }

    pub fn open_plugin_pane(&self, plugin_id: &str, entrypoint: &str) -> Result<()> {
        let args = [
            "plugin",
            "pane",
            "open",
            "--plugin",
            plugin_id,
            "--entrypoint",
            entrypoint,
        ];
        self.run(&args.map(str::to_string)).map(drop)
    }

    /// Validate a config file with herdr's own parser before it goes live.
    pub fn check_config(&self, path: &Path) -> Result<()> {
        let output = proc::run(
            Command::new(&self.bin)
                .args(["config", "check"])
                .env("HERDR_CONFIG_PATH", path),
            CALL_TIMEOUT,
        )?;
        if !output.status.success() {
            bail!("{}", output.stdout.trim());
        }
        Ok(())
    }

    /// The config file this herdr resolves, from the `Config:` line of
    /// `herdr --help`, which accounts for HERDR_CONFIG_PATH and XDG.
    pub fn config_path(&self) -> Option<PathBuf> {
        let output = proc::run(Command::new(&self.bin).arg("--help"), CALL_TIMEOUT).ok()?;
        parse_config_path(&output.stdout)
    }

    /// Strictly increasing per process, so herdr never drops a report as
    /// older than the one before it.
    fn next_seq(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        let mut last = self.last_seq.load(Ordering::Relaxed);
        loop {
            let next = now.max(last + 1);
            match self
                .last_seq
                .compare_exchange(last, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return next,
                Err(actual) => last = actual,
            }
        }
    }
}

impl Sink for Herdr {
    fn report(
        &mut self,
        target: &Target,
        tokens: &Tokens,
        clear: &[&str],
        ttl: Duration,
    ) -> Result<()> {
        let args = report_args(&self.source, target, tokens, clear, ttl, self.next_seq());
        self.run(&args).map(drop)
    }
}

pub fn report_args(
    source: &str,
    target: &Target,
    tokens: &Tokens,
    clear: &[&str],
    ttl: Duration,
    seq: u64,
) -> Vec<String> {
    let (noun, id) = match target {
        Target::Pane(id) => ("pane", id),
        Target::Workspace(id) => ("workspace", id),
    };
    let mut args = vec![
        noun.to_string(),
        "report-metadata".into(),
        id.clone(),
        "--source".into(),
        source.to_string(),
    ];
    for (name, value) in tokens {
        args.push("--token".into());
        args.push(format!("{name}={value}"));
    }
    for name in clear {
        args.push("--clear-token".into());
        args.push(name.to_string());
    }
    args.push("--seq".into());
    args.push(seq.to_string());
    if !tokens.is_empty() {
        args.push("--ttl-ms".into());
        args.push(ttl.as_millis().to_string());
    }
    args
}

fn result<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value["result"][key]
        .as_array()
        .with_context(|| format!("herdr response has no result.{key}"))
}

pub fn parse_agents(value: &Value) -> Result<Vec<Agent>> {
    Ok(result(value, "agents")?
        .iter()
        .filter_map(|agent| {
            Some(Agent {
                pane_id: agent["pane_id"].as_str()?.to_string(),
                workspace_id: agent["workspace_id"].as_str()?.to_string(),
                agent: agent["agent"].as_str()?.to_string(),
            })
        })
        .collect())
}

pub fn parse_workspace_ids(value: &Value) -> Result<Vec<String>> {
    Ok(result(value, "workspaces")?
        .iter()
        .filter_map(|workspace| workspace["workspace_id"].as_str().map(str::to_string))
        .collect())
}

pub fn parse_plugin_state(value: &Value, plugin_id: &str) -> PluginState {
    let plugin = value["result"]["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|plugin| plugin["plugin_id"].as_str() == Some(plugin_id));
    match plugin {
        None => PluginState::Missing,
        Some(plugin) if plugin["enabled"].as_bool() == Some(false) => PluginState::Disabled,
        Some(_) => PluginState::Enabled,
    }
}

pub fn parse_config_path(help: &str) -> Option<PathBuf> {
    help.lines()
        .find_map(|line| line.trim().strip_prefix("Config:"))
        .map(|path| PathBuf::from(path.trim()))
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{TOKEN_PERCENT, TOKEN_USAGE};
    use serde_json::json;

    #[test]
    fn parses_agents_skipping_incomplete_rows() {
        let value = json!({"result": {"agents": [
            {"agent": "claude", "pane_id": "w1:p1", "workspace_id": "w1", "agent_status": "working"},
            {"pane_id": "w1:p2", "workspace_id": "w1"},
        ], "type": "agent_list"}});
        assert_eq!(
            parse_agents(&value).unwrap(),
            [Agent {
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                agent: "claude".into(),
            }]
        );
    }

    #[test]
    fn parses_workspace_ids() {
        let value = json!({"result": {"workspaces": [
            {"workspace_id": "w1", "label": "api"},
            {"workspace_id": "w2"},
        ]}});
        assert_eq!(parse_workspace_ids(&value).unwrap(), ["w1", "w2"]);
    }

    #[test]
    fn a_response_without_the_expected_list_is_an_error() {
        let value = json!({"error": {"code": "server_unavailable"}});
        assert!(parse_agents(&value).is_err());
        assert!(parse_workspace_ids(&value).is_err());
    }

    #[test]
    fn plugin_state_reflects_the_enabled_flag() {
        let list = |enabled: bool| json!({"result": {"plugins": [{"plugin_id": "agnostk.ai-usagebar", "enabled": enabled}]}});
        assert_eq!(
            parse_plugin_state(&list(true), "agnostk.ai-usagebar"),
            PluginState::Enabled
        );
        assert_eq!(
            parse_plugin_state(&list(false), "agnostk.ai-usagebar"),
            PluginState::Disabled
        );
        assert_eq!(
            parse_plugin_state(&list(true), "someone.else"),
            PluginState::Missing
        );
    }

    #[test]
    fn report_args_set_clear_sequence_and_ttl() {
        let tokens = Tokens::from([
            (TOKEN_USAGE, "◔ 5h 0% · 7d 13%".to_string()),
            (TOKEN_PERCENT, "13".to_string()),
        ]);
        let args = report_args(
            "plugin:x",
            &Target::Pane("w1:p1".into()),
            &tokens,
            &[],
            Duration::from_secs(270),
            42,
        );
        assert_eq!(
            args,
            [
                "pane",
                "report-metadata",
                "w1:p1",
                "--source",
                "plugin:x",
                "--token",
                "ai_usage=◔ 5h 0% · 7d 13%",
                "--token",
                "ai_usage_pct=13",
                "--seq",
                "42",
                "--ttl-ms",
                "270000",
            ]
        );
    }

    #[test]
    fn clearing_a_workspace_omits_the_ttl() {
        let args = report_args(
            "plugin:x",
            &Target::Workspace("w2".into()),
            &Tokens::new(),
            &[TOKEN_USAGE, TOKEN_PERCENT],
            Duration::from_secs(270),
            7,
        );
        assert_eq!(
            args,
            [
                "workspace",
                "report-metadata",
                "w2",
                "--source",
                "plugin:x",
                "--clear-token",
                "ai_usage",
                "--clear-token",
                "ai_usage_pct",
                "--seq",
                "7",
            ]
        );
    }

    #[test]
    fn sequence_numbers_strictly_increase() {
        let herdr = Herdr::new("herdr".into(), "x");
        let first = herdr.next_seq();
        let second = herdr.next_seq();
        let third = herdr.next_seq();
        assert!(first < second && second < third);
    }

    #[test]
    fn config_path_comes_from_the_help_text() {
        let help = "Usage: herdr\n\nConfig: /home/u/.config/herdr/config.toml\nLogs:   /x\n";
        assert_eq!(
            parse_config_path(help),
            Some(PathBuf::from("/home/u/.config/herdr/config.toml"))
        );
        assert_eq!(parse_config_path("no such line"), None);
    }
}
