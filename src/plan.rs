//! Decides which sidebar tokens each herdr pane and workspace should carry,
//! from the latest usage snapshot and the current agent layout. Pure: no I/O.

use std::collections::BTreeMap;

use crate::config::{Config, WorkspaceRows};
use crate::format;
use crate::usage::{Entry, Report};

pub const TOKEN_USAGE: &str = "ai_usage";
pub const TOKEN_PERCENT: &str = "ai_usage_pct";
pub const TOKENS: [&str; 2] = [TOKEN_USAGE, TOKEN_PERCENT];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Pane(String),
    Workspace(String),
}

pub type Tokens = BTreeMap<&'static str, String>;
pub type Plan = BTreeMap<Target, Tokens>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub agents: Vec<Agent>,
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Snapshot {
    /// No fetch has finished yet.
    Pending,
    Ready(Report),
    Unavailable {
        missing_binary: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    MissingBinary,
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBinary => f.write_str(
                "ai-usagebar not found; install it or set `ai_usagebar` in the plugin config.toml",
            ),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Fold a fetch result into the snapshot. A failed refresh keeps showing the
/// last good report, marked stale, rather than blanking the sidebar.
pub fn next_snapshot(previous: Snapshot, result: Result<Report, FetchError>) -> Snapshot {
    match (result, previous) {
        (Ok(report), _) => Snapshot::Ready(report),
        (Err(FetchError::MissingBinary), _) => Snapshot::Unavailable {
            missing_binary: true,
        },
        (Err(FetchError::Failed(_)), Snapshot::Ready(report)) => {
            Snapshot::Ready(report.into_stale())
        }
        (Err(FetchError::Failed(_)), _) => Snapshot::Unavailable {
            missing_binary: false,
        },
    }
}

pub fn plan(snapshot: &Snapshot, layout: &Layout, config: &Config) -> Plan {
    match snapshot {
        Snapshot::Pending => Plan::new(),
        Snapshot::Ready(report) => plan_report(report, layout, config),
        Snapshot::Unavailable { missing_binary } => {
            let text = if *missing_binary {
                format::MISSING_BINARY
            } else {
                format::UNAVAILABLE
            };
            plan_unavailable(text, layout, config)
        }
    }
}

/// The entry an agent's usage comes from: the first of its configured ids
/// that is reporting, else the first that exists so its error is visible.
pub fn resolve<'a>(report: &'a Report, providers: &[String]) -> Option<&'a Entry> {
    let mut candidates = providers.iter().flat_map(|provider| {
        report
            .entries
            .iter()
            .filter(move |entry| matches_provider(&entry.id, provider))
    });
    let first = candidates.clone().next();
    candidates.find(|entry| entry.is_ready()).or(first)
}

/// `anthropic` also matches its named accounts such as `anthropic@work`.
fn matches_provider(entry_id: &str, provider: &str) -> bool {
    entry_id == provider
        || entry_id
            .strip_prefix(provider)
            .is_some_and(|rest| rest.starts_with('@'))
}

fn plan_report(report: &Report, layout: &Layout, config: &Config) -> Plan {
    let mut plan = Plan::new();
    let resolved: Vec<(&Agent, &Entry)> = layout
        .agents
        .iter()
        .filter_map(|agent| {
            resolve(report, config.providers_for(&agent.agent)).map(|entry| (agent, entry))
        })
        .collect();

    if config.agent_rows {
        for (agent, entry) in &resolved {
            if let Some(line) = format::entry_line(entry, config.max_windows) {
                plan.insert(
                    Target::Pane(agent.pane_id.clone()),
                    tokens(line, entry.max_percent()),
                );
            }
        }
    }

    for workspace_id in &layout.workspace_ids {
        let entries: Vec<&Entry> = match config.workspace_rows {
            WorkspaceRows::Off => continue,
            WorkspaceRows::All => report.entries.iter().collect(),
            WorkspaceRows::Agents => report
                .entries
                .iter()
                .filter(|entry| {
                    resolved.iter().any(|(agent, used)| {
                        agent.workspace_id == *workspace_id && used.id == entry.id
                    })
                })
                .collect(),
        };
        let percent = entries.iter().filter_map(|entry| entry.max_percent()).max();
        if let Some(line) = format::summary_line(entries) {
            plan.insert(
                Target::Workspace(workspace_id.clone()),
                tokens(line, percent),
            );
        }
    }
    plan
}

fn plan_unavailable(text: &str, layout: &Layout, config: &Config) -> Plan {
    let mut plan = Plan::new();
    let tracked: Vec<&Agent> = layout
        .agents
        .iter()
        .filter(|agent| !config.providers_for(&agent.agent).is_empty())
        .collect();

    if config.agent_rows {
        for agent in &tracked {
            plan.insert(
                Target::Pane(agent.pane_id.clone()),
                tokens(text.into(), None),
            );
        }
    }
    for workspace_id in &layout.workspace_ids {
        let show = match config.workspace_rows {
            WorkspaceRows::Off => false,
            WorkspaceRows::All => true,
            WorkspaceRows::Agents => tracked
                .iter()
                .any(|agent| agent.workspace_id == *workspace_id),
        };
        if show {
            plan.insert(
                Target::Workspace(workspace_id.clone()),
                tokens(text.into(), None),
            );
        }
    }
    plan
}

fn tokens(line: String, percent: Option<u32>) -> Tokens {
    let mut tokens = Tokens::from([(TOKEN_USAGE, line)]);
    if let Some(percent) = percent {
        tokens.insert(TOKEN_PERCENT, percent.to_string());
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report::parse(include_str!("../tests/fixtures/usage.json")).unwrap()
    }

    fn agent(pane: &str, workspace: &str, kind: &str) -> Agent {
        Agent {
            pane_id: pane.into(),
            workspace_id: workspace.into(),
            agent: kind.into(),
        }
    }

    fn layout() -> Layout {
        Layout {
            agents: vec![
                agent("w1:p1", "w1", "claude"),
                agent("w1:p2", "w1", "codex"),
                agent("w2:p1", "w2", "droid"),
            ],
            workspace_ids: vec!["w1".into(), "w2".into(), "w3".into()],
        }
    }

    fn usage(plan: &Plan, target: Target) -> Option<&str> {
        plan.get(&target)
            .and_then(|tokens| tokens.get(TOKEN_USAGE))
            .map(String::as_str)
    }

    fn pane(id: &str) -> Target {
        Target::Pane(id.into())
    }

    fn workspace(id: &str) -> Target {
        Target::Workspace(id.into())
    }

    #[test]
    fn agents_get_their_own_provider_usage() {
        let plan = plan(&Snapshot::Ready(report()), &layout(), &Config::default());
        assert_eq!(usage(&plan, pane("w1:p1")), Some("◔ 5h 0% · 7d 13%"));
        assert_eq!(usage(&plan, pane("w1:p2")), Some("○ 5h 0% · 7d 0%"));
        assert_eq!(plan[&pane("w1:p1")][TOKEN_PERCENT], "13");
    }

    #[test]
    fn agents_without_a_provider_get_nothing() {
        let plan = plan(&Snapshot::Ready(report()), &layout(), &Config::default());
        assert!(!plan.contains_key(&pane("w2:p1")));
    }

    #[test]
    fn workspaces_summarise_the_providers_of_their_agents() {
        let plan = plan(&Snapshot::Ready(report()), &layout(), &Config::default());
        assert_eq!(usage(&plan, workspace("w1")), Some("◔ cld 13% · gpt 0%"));
        assert_eq!(plan[&workspace("w1")][TOKEN_PERCENT], "13");
        assert!(!plan.contains_key(&workspace("w2")));
        assert!(!plan.contains_key(&workspace("w3")));
    }

    #[test]
    fn workspace_rows_all_shows_every_provider_everywhere() {
        let config = Config {
            workspace_rows: WorkspaceRows::All,
            ..Config::default()
        };
        let plan = plan(&Snapshot::Ready(report()), &layout(), &config);
        for id in ["w1", "w2", "w3"] {
            assert_eq!(
                usage(&plan, workspace(id)),
                Some("◔ cld 13% · gpt 0% · ghc 0% · zai ⚠")
            );
        }
    }

    #[test]
    fn rows_can_be_switched_off() {
        let config = Config {
            agent_rows: false,
            workspace_rows: WorkspaceRows::Off,
            ..Config::default()
        };
        assert!(plan(&Snapshot::Ready(report()), &layout(), &config).is_empty());
    }

    #[test]
    fn a_shared_provider_is_listed_once_per_workspace() {
        let layout = Layout {
            agents: vec![
                agent("w1:p1", "w1", "claude"),
                agent("w1:p2", "w1", "claude"),
            ],
            workspace_ids: vec!["w1".into()],
        };
        let plan = plan(&Snapshot::Ready(report()), &layout, &Config::default());
        assert_eq!(usage(&plan, workspace("w1")), Some("◔ cld 13%"));
    }

    #[test]
    fn pending_snapshot_plans_nothing() {
        assert!(plan(&Snapshot::Pending, &layout(), &Config::default()).is_empty());
    }

    #[test]
    fn missing_binary_is_shown_on_tracked_agents_and_their_workspaces() {
        let snapshot = Snapshot::Unavailable {
            missing_binary: true,
        };
        let plan = plan(&snapshot, &layout(), &Config::default());
        assert_eq!(usage(&plan, pane("w1:p1")), Some(format::MISSING_BINARY));
        assert_eq!(usage(&plan, workspace("w1")), Some(format::MISSING_BINARY));
        assert!(!plan.contains_key(&pane("w2:p1")));
        assert!(!plan.contains_key(&workspace("w2")));
        assert!(!plan[&pane("w1:p1")].contains_key(TOKEN_PERCENT));
    }

    #[test]
    fn resolve_prefers_a_ready_entry_over_an_earlier_failing_one() {
        let report = report();
        let providers = vec!["zai".to_string(), "openai".to_string()];
        assert_eq!(resolve(&report, &providers).unwrap().id, "openai");
    }

    #[test]
    fn resolve_falls_back_to_a_failing_entry_to_surface_its_error() {
        let report = report();
        let providers = vec!["zai".to_string(), "not-configured".to_string()];
        assert_eq!(resolve(&report, &providers).unwrap().id, "zai");
    }

    #[test]
    fn resolve_matches_named_accounts_but_not_other_prefixes() {
        let mut report = report();
        report.entries[0].id = "anthropic@work".into();
        let anthropic = vec!["anthropic".to_string()];
        assert_eq!(resolve(&report, &anthropic).unwrap().id, "anthropic@work");

        let api = vec!["anthropic_api".to_string()];
        assert!(resolve(&report, &api).is_none());
        let partial = vec!["anthrop".to_string()];
        assert!(resolve(&report, &partial).is_none());
    }

    #[test]
    fn failed_refresh_keeps_the_last_report_as_stale() {
        let ready = Snapshot::Ready(report());
        let next = next_snapshot(ready, Err(FetchError::Failed("timeout".into())));
        let Snapshot::Ready(report) = next else {
            panic!("expected the previous report to be kept");
        };
        assert!(report.entries.iter().all(|entry| entry.stale));
    }

    #[test]
    fn failed_first_refresh_is_unavailable() {
        let next = next_snapshot(Snapshot::Pending, Err(FetchError::Failed("boom".into())));
        assert_eq!(
            next,
            Snapshot::Unavailable {
                missing_binary: false
            }
        );
    }

    #[test]
    fn missing_binary_replaces_any_previous_report() {
        let next = next_snapshot(Snapshot::Ready(report()), Err(FetchError::MissingBinary));
        assert_eq!(
            next,
            Snapshot::Unavailable {
                missing_binary: true
            }
        );
    }

    #[test]
    fn successful_refresh_replaces_a_stale_report() {
        let stale = Snapshot::Ready(report().into_stale());
        assert_eq!(
            next_snapshot(stale, Ok(report())),
            Snapshot::Ready(report())
        );
    }
}
