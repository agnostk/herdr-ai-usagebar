//! The background process that keeps sidebar tokens current.
//!
//! herdr v1 has no timers or supervised plugin processes: startup hooks are
//! one-shot. So the hook spawns this detached process, which holds a
//! per-session lock (one instance per herdr server) and stops on its own when
//! the plugin is disabled, the server goes away, or `stop` is invoked.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result};

use crate::ai_usagebar::{self, USAGE_BIN};
use crate::config::{self, Config};
use crate::context::Context;
use crate::herdr::PluginState;
use crate::log;
use crate::plan::{self, Snapshot};
use crate::publisher::Publisher;
use crate::session::{self, Marker, Session, Status};

/// How long herdr may be unreachable before the process assumes the server
/// is gone and exits. Covers a live handoff between servers.
const UNREACHABLE_LIMIT: Duration = Duration::from_secs(60);
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Beyond an in-flight ai-usagebar run, stopping only waits for one scan
/// and the clearing calls.
const STOP_GRACE: Duration = Duration::from_secs(20);

/// Start the background process unless this session already has one.
/// Returns whether a new process was spawned.
pub fn ensure_running(ctx: &Context) -> Result<bool> {
    let session = ctx.session();
    if session.is_running()? {
        return Ok(false);
    }
    // Hooks firing together can both get here and both spawn; the loser
    // fails its own try_lock in `run` and exits.
    session.take(Marker::Stop);
    spawn_detached(&session)?;
    Ok(true)
}

fn spawn_detached(session: &Session) -> Result<()> {
    let exe = std::env::current_exe().context("cannot locate the plugin binary")?;
    let log_path = session.log_path();
    if fs::metadata(&log_path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&log_path, log_path.with_extension("log.1"));
    }
    let log_file = session::open_private(&log_path, true)
        .with_context(|| format!("cannot open {}", log_path.display()))?;

    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so signals aimed at the short-lived hook
        // that spawned it do not reach it.
        command.process_group(0);
    }
    command
        .spawn()
        .context("cannot start the background process")?;
    Ok(())
}

/// Ask the running process to clear its tokens and exit, waiting for it.
/// Returns false when no process was running.
pub fn stop(ctx: &Context) -> Result<bool> {
    let session = ctx.session();
    if !session.is_running()? {
        return Ok(false);
    }
    session.signal(Marker::Stop)?;
    let fetch_timeout = ctx.load_config().unwrap_or_default().fetch_timeout_secs;
    let wait = Duration::from_secs(fetch_timeout) + STOP_GRACE;
    let deadline = Instant::now() + wait;
    while session.is_running()? {
        if Instant::now() >= deadline {
            anyhow::bail!(
                "the background process did not stop within {}s",
                wait.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(true)
}

pub fn run(ctx: &Context) -> Result<()> {
    let session = ctx.session();
    let Some(lock) = session.try_lock()? else {
        log::info("another instance is already running for this session");
        return Ok(());
    };
    let exe = std::env::current_exe()?;
    let exe_stamp = stamp(&exe);
    log::info(&format!(
        "started pid {} (v{}) for {}",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        ctx.socket
    ));
    // Leave the commented example where `herdr plugin config-dir` points.
    if let Err(error) = config::ensure_example(&ctx.config_dir) {
        log::info(&format!("{error:#}"));
    }

    let mut herdr = ctx.herdr();
    let mut config = ctx.load_config().unwrap_or_else(|error| {
        log::info(&format!("{error:#}; using defaults"));
        Config::default()
    });
    let mut publisher = Publisher::new(config.token_ttl());
    let mut snapshot = Snapshot::Pending;
    let mut status = Status {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").into(),
        socket: ctx.socket.clone(),
        started_at: log::now_secs(),
        ..Status::default()
    };
    let mut next_fetch = Instant::now();
    let mut unreachable_since: Option<Instant> = None;

    loop {
        if session.take(Marker::Stop) {
            log::info("stop requested");
            publisher.clear_all(&mut herdr);
            return Ok(());
        }
        if session.take(Marker::Refresh) {
            next_fetch = Instant::now();
        }

        if Instant::now() >= next_fetch {
            match ctx.load_config() {
                Ok(fresh) => {
                    if fresh.token_ttl() != config.token_ttl() {
                        publisher.clear_all(&mut herdr);
                        publisher = Publisher::new(fresh.token_ttl());
                    }
                    config = fresh;
                }
                Err(error) => log::info(&format!("{error:#}; keeping the previous settings")),
            }
            match herdr.plugin_state(&ctx.plugin_id) {
                Ok(PluginState::Enabled) => {}
                Ok(state) => {
                    log::info(&format!("plugin is {state:?}; clearing tokens and exiting"));
                    publisher.clear_all(&mut herdr);
                    return Ok(());
                }
                Err(error) => log::debug(&format!("plugin state: {error:#}")),
            }
            if stamp(&exe) != exe_stamp {
                log::info("plugin binary changed; restarting");
                drop(lock);
                return spawn_detached(&session);
            }

            let binary = ai_usagebar::locate(USAGE_BIN, config.ai_usagebar.as_deref());
            let result = ai_usagebar::fetch(
                binary.as_deref(),
                Duration::from_secs(config.fetch_timeout_secs),
            );
            status.ai_usagebar = binary.map(|path| path.display().to_string());
            status.last_fetch_at = Some(log::now_secs());
            match &result {
                Ok(report) => {
                    status.last_error = None;
                    status.entries = report
                        .entries
                        .iter()
                        .map(|entry| {
                            let state = if entry.is_ready() { "ready" } else { "error" };
                            format!("{}: {state}", entry.id)
                        })
                        .collect();
                }
                Err(error) => {
                    log::info(&format!("refresh failed: {error}"));
                    status.last_error = Some(error.to_string());
                }
            }
            if let Err(error) = session.write_status(&status) {
                log::debug(&format!("status: {error:#}"));
            }
            snapshot = plan::next_snapshot(snapshot, result);
            next_fetch = Instant::now() + Duration::from_secs(config.refresh_secs);
        }

        match herdr.layout() {
            Ok(layout) => {
                if unreachable_since.take().is_some() {
                    log::info("herdr is reachable again");
                }
                let plan = plan::plan(&snapshot, &layout, &config);
                publisher.apply(plan, Instant::now(), &mut herdr);
            }
            Err(error) => {
                let since = *unreachable_since.get_or_insert_with(|| {
                    log::info(&format!("herdr unreachable: {error:#}"));
                    Instant::now()
                });
                if since.elapsed() >= UNREACHABLE_LIMIT {
                    log::info("herdr server is gone; exiting");
                    return Ok(());
                }
            }
        }

        session.pause(Duration::from_secs(config.scan_secs));
    }
}

/// Identifies a build of the binary so a rebuild or reinstall restarts the
/// process onto the new code.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}
