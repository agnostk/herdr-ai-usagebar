//! Finding and running the ai-usagebar binaries.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::plan::FetchError;
use crate::proc;
use crate::redact::redact;
use crate::usage::Report;

pub const USAGE_BIN: &str = "ai-usagebar";
pub const TUI_BIN: &str = "ai-usagebar-tui";

/// Locate `name`. A configured `ai-usagebar` path wins, and the TUI is then
/// expected next to it. Otherwise search PATH and then the usual install
/// directories, which a herdr server started outside a login shell may be
/// missing from its PATH.
pub fn locate(name: &str, configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(configured) = configured {
        let candidate = if name == USAGE_BIN {
            configured.to_path_buf()
        } else {
            configured.with_file_name(name)
        };
        return is_executable(&candidate).then_some(candidate);
    }
    let dirs = search_dirs(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        std::env::var("USER").ok().as_deref(),
    );
    dirs.into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn search_dirs(
    path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    user: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        dirs.extend(
            [".cargo/bin", ".local/bin", ".nix-profile/bin"]
                .into_iter()
                .map(|dir| home.join(dir)),
        );
    }
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/run/current-system/sw/bin",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    if let Some(user) = user {
        dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/bin")));
    }
    dirs
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub fn fetch(binary: Option<&Path>, timeout: Duration) -> Result<Report, FetchError> {
    let binary = binary.ok_or(FetchError::MissingBinary)?;
    let failed = |message: String| FetchError::Failed(redact(&message));
    let output = proc::run(Command::new(binary).args(["usage", "--json"]), timeout)
        .map_err(|error| failed(format!("{error:#}")))?;
    if !output.status.success() {
        return Err(failed(format!(
            "ai-usagebar usage failed: {}",
            output.failure_message()
        )));
    }
    Report::parse(&output.stdout).map_err(|error| failed(format!("{error:#}")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn search_dirs_puts_path_first_then_fallbacks() {
        let dirs = search_dirs(
            Some(std::ffi::OsStr::new("/a:/b")),
            Some(Path::new("/home/u")),
            Some("u"),
        );
        assert_eq!(dirs[0], PathBuf::from("/a"));
        assert_eq!(dirs[1], PathBuf::from("/b"));
        assert!(dirs.contains(&PathBuf::from("/home/u/.cargo/bin")));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert_eq!(
            dirs.last().unwrap(),
            &PathBuf::from("/etc/profiles/per-user/u/bin")
        );
    }

    #[test]
    fn configured_path_is_used_and_the_tui_is_found_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let usage = script(dir.path(), USAGE_BIN, "exit 0");
        let tui = script(dir.path(), TUI_BIN, "exit 0");
        assert_eq!(locate(USAGE_BIN, Some(&usage)), Some(usage.clone()));
        assert_eq!(locate(TUI_BIN, Some(&usage)), Some(tui));
    }

    #[test]
    fn a_configured_path_that_does_not_exist_is_not_replaced_by_a_search() {
        let missing = Path::new("/nonexistent/ai-usagebar");
        assert_eq!(locate(USAGE_BIN, Some(missing)), None);
    }

    #[test]
    fn non_executable_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USAGE_BIN);
        std::fs::write(&path, "not a program").unwrap();
        assert!(!is_executable(&path));
    }

    #[test]
    fn fetch_parses_the_report() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage.json");
        let bin = script(
            dir.path(),
            USAGE_BIN,
            &format!("cat '{}'", fixture.display()),
        );
        let report = fetch(Some(&bin), Duration::from_secs(5)).unwrap();
        assert_eq!(report.entries.len(), 4);
    }

    #[test]
    fn fetch_reports_a_non_zero_exit_with_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let bin = script(
            dir.path(),
            USAGE_BIN,
            "echo 'no vendors enabled' >&2; exit 1",
        );
        let error = fetch(Some(&bin), Duration::from_secs(5)).unwrap_err();
        assert_eq!(
            error,
            FetchError::Failed("ai-usagebar usage failed: no vendors enabled".into())
        );
    }

    #[test]
    fn fetch_without_a_binary_is_missing_binary() {
        assert_eq!(
            fetch(None, Duration::from_secs(1)).unwrap_err(),
            FetchError::MissingBinary
        );
    }

    #[test]
    fn fetch_rejects_output_that_is_not_a_report() {
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), USAGE_BIN, "echo '{\"text\":\"waybar\"}'");
        assert!(matches!(
            fetch(Some(&bin), Duration::from_secs(5)),
            Err(FetchError::Failed(_))
        ));
    }
}
