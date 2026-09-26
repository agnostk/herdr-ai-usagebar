//! herdr plugin that shows ai-usagebar plan usage in the herdr sidebar.

mod ai_usagebar;
mod config;
mod context;
mod daemon;
mod format;
mod herdr;
mod log;
mod plan;
mod proc;
mod publisher;
mod redact;
mod session;
mod sidebar;
mod usage;

use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::ai_usagebar::{TUI_BIN, USAGE_BIN};
use crate::config::{Config, WorkspaceRows};
use crate::context::Context;
use crate::plan::{TOKENS, Target, Tokens};
use crate::publisher::Sink;
use crate::session::Marker;

const USAGE: &str = "\
herdr-ai-usagebar: ai-usagebar plan usage in the herdr sidebar

Commands (normally run by herdr from herdr-plugin.toml):
  start          Start the background refresher for this herdr session
  refresh        Refresh usage now, starting the refresher if needed
  stop           Stop the refresher and clear the sidebar tokens
  setup-sidebar  Add the $ai_usage row to the herdr sidebar layout
  status         Show refresher state and the last ai-usagebar result
  dashboard      Open the ai-usagebar-tui popup pane
  tui            Run ai-usagebar-tui in this terminal (the pane entrypoint)
  daemon         Run the refresher in the foreground (internal)

Set HERDR_AI_USAGEBAR_DEBUG=1 for verbose logs.";

fn main() -> ExitCode {
    log::set_verbose(std::env::var_os("HERDR_AI_USAGEBAR_DEBUG").is_some());
    let command = std::env::args().nth(1).unwrap_or_default();
    let result = match command.as_str() {
        "start" => start(),
        "refresh" => refresh(),
        "stop" => stop(),
        "setup-sidebar" => setup_sidebar(),
        "status" => status(),
        "dashboard" => Context::from_env()
            .and_then(|ctx| ctx.herdr().open_plugin_pane(&ctx.plugin_id, "dashboard")),
        "tui" => tui(),
        "daemon" => Context::from_env().and_then(|ctx| daemon::run(&ctx)),
        "--version" | "-V" => {
            println!("herdr-ai-usagebar {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "" | "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("{USAGE}");
            Err(anyhow::anyhow!("unknown command `{other}`"))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("herdr-ai-usagebar: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Startup and event hooks: make sure the refresher runs.
fn start() -> Result<()> {
    let ctx = Context::from_env()?;
    if daemon::ensure_running(&ctx)? {
        log::info("started the background refresher");
    }
    Ok(())
}

fn refresh() -> Result<()> {
    let ctx = Context::from_env()?;
    if daemon::ensure_running(&ctx)? {
        println!("Started the background refresher; usage appears in a few seconds.");
    } else {
        ctx.session().signal(Marker::Refresh)?;
        println!("Refresh requested.");
    }
    Ok(())
}

fn stop() -> Result<()> {
    let ctx = Context::from_env()?;
    if daemon::stop(&ctx)? {
        println!("Stopped the background refresher and cleared the sidebar tokens.");
        return Ok(());
    }
    // Nothing running: clear any tokens a previous run left before expiry.
    let mut herdr = ctx.herdr();
    let layout = herdr.layout()?;
    let targets = layout
        .agents
        .iter()
        .map(|agent| Target::Pane(agent.pane_id.clone()))
        .chain(layout.workspace_ids.iter().cloned().map(Target::Workspace));
    for target in targets {
        let ttl = std::time::Duration::ZERO;
        if let Err(error) = herdr.report(&target, &Tokens::new(), &TOKENS, ttl) {
            log::debug(&format!("clear {target:?}: {error:#}"));
        }
    }
    println!("The background refresher was not running; cleared the sidebar tokens.");
    Ok(())
}

fn setup_sidebar() -> Result<()> {
    let ctx = Context::from_env()?;
    let config = ctx.load_config()?;
    let outcome = sidebar::apply(&ctx.herdr(), &config)?;
    if !outcome.changed.is_empty() {
        println!("Added the $ai_usage row to {}:", outcome.path.display());
        for key in &outcome.changed {
            println!("  {key}");
        }
        if let Some(backup) = &outcome.backup {
            println!("Backup: {}", backup.display());
        }
    } else if config.agent_rows || config.workspace_rows != WorkspaceRows::Off {
        println!("{} already shows $ai_usage.", outcome.path.display());
    } else {
        println!("agent_rows and workspace_rows are off in the plugin config; nothing to add.");
    }
    daemon::ensure_running(&ctx)?;
    if let Some(error) = outcome.reload_error {
        bail!("herdr did not reload its config ({error}); run `herdr server reload-config`");
    }
    println!("Reloaded the herdr config.");
    Ok(())
}

fn status() -> Result<()> {
    let ctx = Context::from_env()?;
    let session = ctx.session();
    println!("herdr-ai-usagebar {}", env!("CARGO_PKG_VERSION"));

    let config = match ctx.load_config() {
        Ok(config) => {
            println!("config: {}", ctx.config_path().display());
            config
        }
        Err(error) => {
            println!("config: {error:#}");
            Config::default()
        }
    };
    match ai_usagebar::locate(USAGE_BIN, config.ai_usagebar.as_deref()) {
        Some(path) => println!("ai-usagebar: {}", path.display()),
        None => println!("ai-usagebar: not found"),
    }

    let running = session.is_running()?;
    println!(
        "refresher: {} (log: {})",
        if running { "running" } else { "not running" },
        session.log_path().display()
    );
    if let Some(status) = session.read_status() {
        println!(
            "  pid {} v{}, started {}",
            status.pid,
            status.version,
            log::utc_timestamp(status.started_at)
        );
        if let Some(at) = status.last_fetch_at {
            println!("  last refresh {}", log::utc_timestamp(at));
        }
        if let Some(error) = &status.last_error {
            println!("  last error: {error}");
        }
        for entry in &status.entries {
            println!("  {entry}");
        }
    }
    Ok(())
}

/// Pane entrypoint: replace this process with ai-usagebar-tui.
fn tui() -> Result<()> {
    let config = Context::from_env()
        .and_then(|ctx| ctx.load_config())
        .unwrap_or_default();
    let Some(path) = ai_usagebar::locate(TUI_BIN, config.ai_usagebar.as_deref()) else {
        eprintln!(
            "{TUI_BIN} was not found. Install ai-usagebar \
             (https://github.com/akitaonrails/ai-usagebar) or set `ai_usagebar` \
             in the plugin config.toml.\n\nPress Enter to close."
        );
        let _ = std::io::stdin().read_line(&mut String::new());
        bail!("{TUI_BIN} not found");
    };
    exec(&path)
}

#[cfg(unix)]
fn exec(path: &std::path::Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    // exec only returns on failure.
    let error = proc::retry_while_busy(|| Err::<(), _>(std::process::Command::new(path).exec()))
        .unwrap_err();
    Err(anyhow::Error::new(error).context(format!("cannot run {}", path.display())))
}

#[cfg(not(unix))]
fn exec(path: &std::path::Path) -> Result<()> {
    let status = std::process::Command::new(path).status()?;
    if !status.success() {
        bail!("{} exited with {status}", path.display());
    }
    Ok(())
}
