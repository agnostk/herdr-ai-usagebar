//! Plugin settings, read from `config.toml` in the plugin's herdr config
//! directory (`herdr plugin config-dir agnostk.ai-usagebar`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

pub const FILE_NAME: &str = "config.toml";
pub const EXAMPLE: &str = include_str!("../config.example.toml");

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRows {
    /// Providers used by the agents running in that workspace.
    #[default]
    Agents,
    /// Every provider ai-usagebar reports, on every workspace.
    All,
    Off,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub ai_usagebar: Option<PathBuf>,
    pub refresh_secs: u64,
    pub scan_secs: u64,
    pub fetch_timeout_secs: u64,
    pub max_windows: usize,
    pub agent_rows: bool,
    pub workspace_rows: WorkspaceRows,
    /// herdr agent id -> ai-usagebar entry ids, tried in order.
    pub agents: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    ai_usagebar: Option<PathBuf>,
    refresh_secs: Option<u64>,
    scan_secs: Option<u64>,
    fetch_timeout_secs: Option<u64>,
    max_windows: Option<usize>,
    agent_rows: Option<bool>,
    workspace_rows: Option<WorkspaceRows>,
    agents: BTreeMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self::from_raw(RawConfig::default())
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawConfig = toml::from_str(text)?;
        Ok(Self::from_raw(raw))
    }

    /// A missing file means defaults; an unreadable or invalid one is an
    /// error so the caller can report it instead of silently ignoring it.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    fn from_raw(raw: RawConfig) -> Self {
        let mut agents = default_agents();
        agents.extend(raw.agents);
        Self {
            ai_usagebar: raw.ai_usagebar,
            refresh_secs: raw.refresh_secs.unwrap_or(60).clamp(15, 3600),
            scan_secs: raw.scan_secs.unwrap_or(5).clamp(1, 300),
            fetch_timeout_secs: raw.fetch_timeout_secs.unwrap_or(90).clamp(5, 600),
            max_windows: raw.max_windows.unwrap_or(2).clamp(1, 6),
            agent_rows: raw.agent_rows.unwrap_or(true),
            workspace_rows: raw.workspace_rows.unwrap_or_default(),
            agents,
        }
    }

    /// How long reported tokens live without being renewed. One pass of the
    /// refresher can take a fetch timeout plus a scan interval, and tokens are
    /// renewed after a third of their TTL, so this outlasts the slowest pass
    /// while still clearing values soon after the refresher dies.
    pub fn token_ttl(&self) -> Duration {
        Duration::from_secs(3 * (self.refresh_secs + self.fetch_timeout_secs + self.scan_secs))
    }

    /// Entry ids to try for a herdr agent; empty when the agent has no
    /// provider ai-usagebar can report on.
    pub fn providers_for(&self, agent: &str) -> &[String] {
        self.agents
            .get(agent)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

/// Write the commented example config if the user has none yet, so the
/// options are discoverable from `herdr plugin config-dir`.
pub fn ensure_example(dir: &Path) -> Result<()> {
    let path = dir.join(FILE_NAME);
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, EXAMPLE).with_context(|| format!("cannot write {}", path.display()))
}

fn default_agents() -> BTreeMap<String, Vec<String>> {
    [
        ("claude", &["anthropic", "anthropic_api"][..]),
        ("codex", &["openai"]),
        ("copilot", &["copilot"]),
        ("cursor", &["cursor"]),
        ("kimi", &["kimi", "moonshot"]),
        ("kilo", &["kilo"]),
        ("kiro", &["kiro"]),
        ("grok", &["grok", "supergrok"]),
        ("agy", &["antigravity"]),
        ("opencode", &["opencode-go"]),
    ]
    .into_iter()
    .map(|(agent, ids)| {
        (
            agent.to_string(),
            ids.iter().map(|id| id.to_string()).collect(),
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.refresh_secs, 60);
        assert_eq!(config.scan_secs, 5);
        assert_eq!(config.max_windows, 2);
        assert!(config.agent_rows);
        assert_eq!(config.workspace_rows, WorkspaceRows::Agents);
        assert_eq!(
            config.providers_for("claude"),
            ["anthropic", "anthropic_api"]
        );
        assert_eq!(config.providers_for("codex"), ["openai"]);
    }

    #[test]
    fn unknown_agents_have_no_providers() {
        assert!(Config::default().providers_for("droid").is_empty());
    }

    #[test]
    fn agent_overrides_replace_only_their_key() {
        let config = Config::parse(
            r#"
            [agents]
            claude = ["anthropic@work"]
            amp = ["openrouter"]
            codex = []
            "#,
        )
        .unwrap();
        assert_eq!(config.providers_for("claude"), ["anthropic@work"]);
        assert_eq!(config.providers_for("amp"), ["openrouter"]);
        assert!(config.providers_for("codex").is_empty());
        assert_eq!(config.providers_for("copilot"), ["copilot"]);
    }

    #[test]
    fn numbers_are_clamped_to_safe_ranges() {
        let config = Config::parse(
            "refresh_secs = 1\nscan_secs = 0\nmax_windows = 50\nfetch_timeout_secs = 100000",
        )
        .unwrap();
        assert_eq!(config.refresh_secs, 15);
        assert_eq!(config.scan_secs, 1);
        assert_eq!(config.max_windows, 6);
        assert_eq!(config.fetch_timeout_secs, 600);
    }

    #[test]
    fn token_ttl_outlasts_the_slowest_refresher_pass() {
        let extremes = [
            "refresh_secs = 15\nfetch_timeout_secs = 600\nscan_secs = 300",
            "refresh_secs = 3600\nfetch_timeout_secs = 5\nscan_secs = 1",
            "",
        ];
        for text in extremes {
            let config = Config::parse(text).unwrap();
            let renew_after = config.token_ttl() / 3;
            let slowest_pass = Duration::from_secs(config.fetch_timeout_secs + config.scan_secs);
            assert!(renew_after + slowest_pass < config.token_ttl(), "{text:?}");
        }
    }

    #[test]
    fn workspace_rows_accepts_each_mode() {
        for (text, mode) in [
            ("agents", WorkspaceRows::Agents),
            ("all", WorkspaceRows::All),
            ("off", WorkspaceRows::Off),
        ] {
            let config = Config::parse(&format!("workspace_rows = \"{text}\"")).unwrap();
            assert_eq!(config.workspace_rows, mode);
        }
    }

    #[test]
    fn typos_are_rejected() {
        let err = Config::parse("refresh_sec = 30").unwrap_err();
        assert!(format!("{err:#}").contains("refresh_sec"));
    }

    #[test]
    fn the_shipped_example_parses_to_the_defaults() {
        assert_eq!(Config::parse(EXAMPLE).unwrap(), Config::default());
    }

    #[test]
    fn load_treats_a_missing_file_as_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn ensure_example_writes_once_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        ensure_example(dir.path()).unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);

        std::fs::write(&path, "max_windows = 3\n").unwrap();
        ensure_example(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "max_windows = 3\n");
    }
}
