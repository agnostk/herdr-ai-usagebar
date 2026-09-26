//! `setup-sidebar`: add a `$ai_usage` row to herdr's sidebar layouts.
//!
//! herdr only renders custom tokens that appear in `[ui.sidebar.*].rows`, and
//! a plugin cannot change the layout at runtime, so this edits the user's
//! config.toml once, preserving its formatting and comments.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::config::{Config, WorkspaceRows};
use crate::herdr::Herdr;
use crate::log;

pub const USAGE_TOKEN: &str = "$ai_usage";

/// Colored by the leading pie glyph, the only part of the value herdr's
/// style rules can match on.
const USAGE_ROW: &str = r##"[{ token = "$ai_usage", fg = "#a6e3a1", rules = [{ starts_with = "●", fg = "#f38ba8", bold = true }, { starts_with = "◕", fg = "#fab387" }, { starts_with = "◑", fg = "#f9e2af" }, { starts_with = "⚠", fg = "#f38ba8" }] }]"##;

const AGENT_DEFAULT_ROWS: &[&[&str]] =
    &[&["state_icon", "machine", "workspace", "tab"], &["agent"]];
const SPACE_DEFAULT_ROWS: &[&[&str]] = &[&["state_icon", "workspace"], &["branch", "git_status"]];
const ROW_INDENT: &str = "\n  ";

/// Add the usage row where it is missing; returns the config keys changed.
pub fn install(doc: &mut DocumentMut, agents: bool, spaces: bool) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    if agents {
        let table = panel_table(doc, "agents")?;
        if add_usage_row(table, AGENT_DEFAULT_ROWS)? {
            changed.push("ui.sidebar.agents.rows".to_string());
        }
        // A per-agent override replaces `rows` entirely, so the usage row has
        // to be added to each override too or it disappears for that agent.
        if let Some(overrides) = table
            .get_mut("rows_by_agent")
            .and_then(Item::as_table_like_mut)
        {
            for (agent, rows) in overrides.iter_mut() {
                if rows.as_array_mut().is_some_and(append_if_missing) {
                    changed.push(format!("ui.sidebar.agents.rows_by_agent.{agent}"));
                }
            }
        }
    }
    if spaces && add_usage_row(panel_table(doc, "spaces")?, SPACE_DEFAULT_ROWS)? {
        changed.push("ui.sidebar.spaces.rows".to_string());
    }
    Ok(changed)
}

fn panel_table<'a>(doc: &'a mut DocumentMut, panel: &str) -> Result<&'a mut dyn TableLike> {
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for key in ["ui", "sidebar", panel] {
        let mut fresh = Table::new();
        fresh.set_implicit(key != panel);
        table = table
            .entry(key)
            .or_insert(Item::Table(fresh))
            .as_table_like_mut()
            .with_context(|| format!("`{key}` in the herdr config is not a table"))?;
    }
    Ok(table)
}

fn add_usage_row(table: &mut dyn TableLike, defaults: &[&[&str]]) -> Result<bool> {
    match table.get_mut("rows") {
        Some(item) => {
            let rows = item
                .as_array_mut()
                .context("sidebar `rows` in the herdr config is not an array")?;
            Ok(append_if_missing(rows))
        }
        None => {
            let mut rows = Array::new();
            for row in defaults {
                let mut value = Value::Array(row.iter().copied().collect());
                value.decor_mut().set_prefix(ROW_INDENT);
                rows.push_formatted(value);
            }
            let mut usage = usage_row();
            usage.decor_mut().set_prefix(ROW_INDENT);
            rows.push_formatted(usage);
            rows.set_trailing_comma(true);
            rows.set_trailing("\n");
            table.insert("rows", Item::Value(Value::Array(rows)));
            Ok(true)
        }
    }
}

fn append_if_missing(rows: &mut Array) -> bool {
    if has_usage_token(rows) {
        return false;
    }
    let prefix = rows
        .iter()
        .last()
        .and_then(|row| row.decor().prefix())
        .and_then(|prefix| prefix.as_str())
        .filter(|prefix| prefix.contains('\n'))
        .map_or_else(
            || if rows.is_empty() { "" } else { " " }.to_string(),
            str::to_string,
        );
    let mut usage = usage_row();
    usage.decor_mut().set_prefix(prefix);
    usage.decor_mut().set_suffix("");
    rows.push_formatted(usage);
    true
}

fn has_usage_token(rows: &Array) -> bool {
    rows.iter()
        .filter_map(Value::as_array)
        .flat_map(Array::iter)
        .any(|token| match token {
            Value::String(name) => name.value() == USAGE_TOKEN,
            Value::InlineTable(styled) => {
                styled.get("token").and_then(Value::as_str) == Some(USAGE_TOKEN)
            }
            _ => false,
        })
}

fn usage_row() -> Value {
    let doc: DocumentMut = format!("row = {USAGE_ROW}")
        .parse()
        .expect("USAGE_ROW is valid TOML");
    doc["row"].as_value().expect("USAGE_ROW is a value").clone()
}

#[derive(Debug)]
pub struct Outcome {
    pub path: PathBuf,
    pub changed: Vec<String>,
    pub backup: Option<PathBuf>,
    /// The config is correct on disk but herdr did not reload it.
    pub reload_error: Option<String>,
}

/// Edit the herdr config in place: validate the result with `herdr config
/// check` before it replaces anything, keep a backup, then reload herdr.
/// The reload also runs when nothing changed, so a rerun recovers from an
/// earlier reload that failed after the file was written.
pub fn apply(herdr: &Herdr, config: &Config) -> Result<Outcome> {
    let path = herdr
        .config_path()
        .or_else(fallback_config_path)
        .context("cannot find the herdr config file")?;
    // Write through a symlink (dotfile managers) instead of replacing it.
    let path = fs::canonicalize(&path).unwrap_or(path);

    let original = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", path.display()));
        }
    };
    let mut doc: DocumentMut = original
        .as_deref()
        .unwrap_or_default()
        .parse()
        .with_context(|| format!("cannot parse {}", path.display()))?;

    let changed = install(
        &mut doc,
        config.agent_rows,
        config.workspace_rows != WorkspaceRows::Off,
    )?;
    if changed.is_empty() {
        return Ok(Outcome {
            path,
            changed,
            backup: None,
            reload_error: reload(herdr),
        });
    }

    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    // A randomly named file created with O_EXCL, in the same directory so the
    // final rename is atomic; dropping it on any early return deletes it.
    let mut candidate = tempfile::Builder::new()
        .prefix(".config.toml.ai-usagebar-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    candidate.write_all(doc.to_string().as_bytes())?;
    candidate.as_file().sync_all()?;
    if let Ok(meta) = fs::metadata(&path) {
        candidate.as_file().set_permissions(meta.permissions())?;
    }
    if let Err(error) = herdr.check_config(candidate.path()) {
        bail!("herdr rejected the updated config, nothing was changed:\n{error:#}");
    }

    let backup = match &original {
        Some(text) => Some(write_backup(dir, &path, text)?),
        None => None,
    };
    candidate
        .persist(&path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(Outcome {
        path,
        changed,
        backup,
        reload_error: reload(herdr),
    })
}

fn reload(herdr: &Herdr) -> Option<String> {
    herdr
        .reload_config()
        .err()
        .map(|error| format!("{error:#}"))
}

/// `create_new` refuses to reuse an existing path, including a planted
/// symlink, so the backup never writes through someone else's link.
fn write_backup(dir: &Path, path: &Path, text: &str) -> Result<PathBuf> {
    let stem = format!("config.toml.bak-ai-usagebar-{}", log::now_secs());
    let (backup, mut file) = (0..100)
        .map(|n| match n {
            0 => dir.join(&stem),
            n => dir.join(format!("{stem}-{n}")),
        })
        .find_map(|backup| {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup);
            file.ok().map(|file| (backup, file))
        })
        .with_context(|| {
            format!(
                "cannot create a backup of {} in {}",
                path.display(),
                dir.display()
            )
        })?;
    file.write_all(text.as_bytes())?;
    if let Ok(meta) = fs::metadata(path) {
        file.set_permissions(meta.permissions())?;
    }
    Ok(backup)
}

fn fallback_config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("herdr").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(input: &str, agents: bool, spaces: bool) -> (String, Vec<String>) {
        let mut doc: DocumentMut = input.parse().unwrap();
        let changed = install(&mut doc, agents, spaces).unwrap();
        (doc.to_string(), changed)
    }

    fn rows<'a>(doc: &'a DocumentMut, panel: &str) -> &'a Array {
        doc["ui"]["sidebar"][panel]["rows"].as_array().unwrap()
    }

    #[test]
    fn an_empty_config_gets_the_default_rows_plus_usage() {
        let (output, changed) = installed("onboarding = false\n", true, true);
        assert_eq!(
            changed,
            ["ui.sidebar.agents.rows", "ui.sidebar.spaces.rows"]
        );

        let doc: DocumentMut = output.parse().unwrap();
        let agents = rows(&doc, "agents");
        assert_eq!(agents.len(), 3);
        assert_eq!(
            agents
                .get(1)
                .unwrap()
                .as_array()
                .unwrap()
                .get(0)
                .unwrap()
                .as_str(),
            Some("agent")
        );
        assert!(has_usage_token(agents));
        assert!(has_usage_token(rows(&doc, "spaces")));

        assert!(output.starts_with("onboarding = false\n"));
        assert!(output.contains("[ui.sidebar.agents]\nrows = [\n  [\"state_icon\", \"machine\", \"workspace\", \"tab\"],\n  [\"agent\"],\n  [{ token = \"$ai_usage\""));
        assert!(
            !output.contains("[ui]\n"),
            "intermediate tables stay implicit:\n{output}"
        );
    }

    #[test]
    fn existing_rows_are_kept_and_extended() {
        let input =
            "[ui.sidebar.agents]\nrows = [\n  [\"state_icon\", \"agent\"],\n  [\"$model\"],\n]\n";
        let (output, changed) = installed(input, true, false);
        assert_eq!(changed, ["ui.sidebar.agents.rows"]);
        assert!(output.starts_with(
            "[ui.sidebar.agents]\nrows = [\n  [\"state_icon\", \"agent\"],\n  [\"$model\"],\n  [{ token = \"$ai_usage\""
        ), "{output}");
        assert!(output.ends_with("}] }],\n]\n"), "{output}");
    }

    #[test]
    fn single_line_rows_stay_on_one_line() {
        let input = "[ui.sidebar.spaces]\nrows = [[\"workspace\"]]\n";
        let (output, _) = installed(input, false, true);
        assert!(
            output.starts_with(
                "[ui.sidebar.spaces]\nrows = [[\"workspace\"], [{ token = \"$ai_usage\""
            ),
            "{output}"
        );
    }

    #[test]
    fn installing_twice_changes_nothing() {
        let (once, _) = installed("", true, true);
        let (twice, changed) = installed(&once, true, true);
        assert!(changed.is_empty());
        assert_eq!(once, twice);
    }

    #[test]
    fn a_plain_token_counts_as_already_installed() {
        let input = "[ui.sidebar.agents]\nrows = [[\"agent\", \"$ai_usage\"]]\n";
        let (output, changed) = installed(input, true, false);
        assert!(changed.is_empty());
        assert_eq!(output, input);
    }

    #[test]
    fn per_agent_overrides_get_the_row_too() {
        let input = "[ui.sidebar.agents]\nrows = [[\"agent\"]]\n\n[ui.sidebar.agents.rows_by_agent]\nclaude = [[\"agent\"], [\"terminal_title_stripped\"]]\n";
        let (output, changed) = installed(input, true, false);
        assert_eq!(
            changed,
            [
                "ui.sidebar.agents.rows",
                "ui.sidebar.agents.rows_by_agent.claude"
            ]
        );
        let doc: DocumentMut = output.parse().unwrap();
        let claude = doc["ui"]["sidebar"]["agents"]["rows_by_agent"]["claude"]
            .as_array()
            .unwrap();
        assert_eq!(claude.len(), 3);
        assert!(has_usage_token(claude));
    }

    #[test]
    fn panels_can_be_skipped() {
        let (output, changed) = installed("", false, false);
        assert!(changed.is_empty());
        assert_eq!(output, "");
    }

    #[test]
    fn comments_and_unrelated_settings_survive() {
        let input =
            "# my herdr config\n[ui]\nsidebar_width = 32 # wider\n\n[keys]\nprefix = \"ctrl+a\"\n";
        let (output, _) = installed(input, true, true);
        assert!(
            output.starts_with("# my herdr config\n[ui]\nsidebar_width = 32 # wider\n"),
            "{output}"
        );
        assert!(
            output.ends_with("[keys]\nprefix = \"ctrl+a\"\n"),
            "{output}"
        );
    }

    #[test]
    fn a_non_table_ui_key_is_an_error() {
        let mut doc: DocumentMut = "ui = 3\n".parse().unwrap();
        let error = install(&mut doc, true, true).unwrap_err();
        assert!(error.to_string().contains("`ui`"));
    }

    #[test]
    fn a_non_array_rows_key_is_an_error() {
        let mut doc: DocumentMut = "[ui.sidebar.agents]\nrows = \"agent\"\n".parse().unwrap();
        assert!(install(&mut doc, true, false).is_err());
    }

    #[cfg(unix)]
    mod apply {
        use super::super::*;
        use std::os::unix::fs::PermissionsExt;

        /// A stand-in `herdr` that reports `config` as its config path,
        /// accepts or rejects `config check`, and records reloads, which
        /// fail while the `reload-fails` file exists.
        struct FakeHerdr {
            dir: tempfile::TempDir,
            config: PathBuf,
        }

        impl FakeHerdr {
            fn new(check_passes: bool) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let config = dir.path().join("herdr").join("config.toml");
                fs::create_dir_all(config.parent().unwrap()).unwrap();
                let check_exit = if check_passes { 0 } else { 1 };
                let script = format!(
                    "#!/bin/sh\n\
                     case \"$1\" in\n\
                       --help) echo 'Config: {config}' ;;\n\
                       config) echo 'config: issues found'; exit {check_exit} ;;\n\
                       server) [ -e '{fail}' ] && {{ echo 'server unavailable' >&2; exit 1; }}\n\
                               echo reload >> '{reloads}' ;;\n\
                     esac\n",
                    config = config.display(),
                    reloads = dir.path().join("reloads").display(),
                    fail = dir.path().join("reload-fails").display(),
                );
                let bin = dir.path().join("herdr-bin");
                fs::write(&bin, script).unwrap();
                fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
                Self { dir, config }
            }

            fn herdr(&self) -> Herdr {
                Herdr::new(self.dir.path().join("herdr-bin"), "test.plugin")
            }

            fn fail_reloads(&self, fail: bool) {
                let flag = self.dir.path().join("reload-fails");
                if fail {
                    fs::write(flag, "").unwrap();
                } else {
                    let _ = fs::remove_file(flag);
                }
            }

            fn reloads(&self) -> usize {
                fs::read_to_string(self.dir.path().join("reloads"))
                    .map_or(0, |text| text.lines().count())
            }

            fn backups(&self) -> Vec<PathBuf> {
                fs::read_dir(self.config.parent().unwrap())
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.to_string_lossy().contains(".bak-ai-usagebar-"))
                    .collect()
            }
        }

        #[test]
        fn backs_up_writes_and_reloads() {
            let fake = FakeHerdr::new(true);
            fs::write(&fake.config, "onboarding = false\n").unwrap();

            let outcome = apply(&fake.herdr(), &Config::default()).unwrap();
            assert_eq!(outcome.changed.len(), 2);
            assert!(
                fs::read_to_string(&fake.config)
                    .unwrap()
                    .contains("$ai_usage")
            );
            let backup = outcome.backup.expect("existing config is backed up");
            assert_eq!(fs::read_to_string(backup).unwrap(), "onboarding = false\n");
            assert_eq!(fake.reloads(), 1);

            assert!(outcome.reload_error.is_none());

            let again = apply(&fake.herdr(), &Config::default()).unwrap();
            assert!(again.changed.is_empty());
            assert!(again.backup.is_none());
            assert_eq!(fake.backups().len(), 1);
        }

        #[test]
        fn a_failed_reload_is_reported_and_retried_by_a_rerun() {
            let fake = FakeHerdr::new(true);
            fs::write(&fake.config, "onboarding = false\n").unwrap();
            fake.fail_reloads(true);

            let outcome = apply(&fake.herdr(), &Config::default()).unwrap();
            assert_eq!(outcome.changed.len(), 2);
            assert!(outcome.reload_error.unwrap().contains("server unavailable"));
            assert!(
                fs::read_to_string(&fake.config)
                    .unwrap()
                    .contains("$ai_usage")
            );
            assert_eq!(fake.reloads(), 0);

            fake.fail_reloads(false);
            let rerun = apply(&fake.herdr(), &Config::default()).unwrap();
            assert!(rerun.changed.is_empty());
            assert!(rerun.reload_error.is_none());
            assert_eq!(fake.reloads(), 1);
        }

        #[test]
        fn backups_in_the_same_second_get_distinct_names() {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("config.toml");
            fs::write(&config, "a = 1\n").unwrap();
            let first = write_backup(dir.path(), &config, "a = 1\n").unwrap();
            let second = write_backup(dir.path(), &config, "a = 2\n").unwrap();
            assert_ne!(first, second);
            assert_eq!(fs::read_to_string(first).unwrap(), "a = 1\n");
            assert_eq!(fs::read_to_string(second).unwrap(), "a = 2\n");
        }

        #[test]
        fn the_config_and_backup_keep_the_original_permissions() {
            let fake = FakeHerdr::new(true);
            fs::write(&fake.config, "onboarding = false\n").unwrap();
            fs::set_permissions(&fake.config, fs::Permissions::from_mode(0o640)).unwrap();

            let outcome = apply(&fake.herdr(), &Config::default()).unwrap();
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&fake.config), 0o640);
            assert_eq!(mode(&outcome.backup.unwrap()), 0o640);
        }

        #[test]
        fn a_missing_config_is_created_without_a_backup() {
            let fake = FakeHerdr::new(true);
            let outcome = apply(&fake.herdr(), &Config::default()).unwrap();
            assert!(outcome.backup.is_none());
            assert!(
                fs::read_to_string(&fake.config)
                    .unwrap()
                    .contains("$ai_usage")
            );
        }

        #[test]
        fn a_rejected_config_leaves_the_file_untouched() {
            let fake = FakeHerdr::new(false);
            fs::write(&fake.config, "onboarding = false\n").unwrap();

            let error = apply(&fake.herdr(), &Config::default()).unwrap_err();
            assert!(error.to_string().contains("nothing was changed"));
            assert_eq!(
                fs::read_to_string(&fake.config).unwrap(),
                "onboarding = false\n"
            );
            assert!(fake.backups().is_empty());
            assert_eq!(fake.reloads(), 0);
            let leftovers = fs::read_dir(fake.config.parent().unwrap()).unwrap().count();
            assert_eq!(leftovers, 1, "the candidate file is removed");
        }

        #[test]
        fn a_symlinked_config_is_edited_through_the_link() {
            let fake = FakeHerdr::new(true);
            let real = fake.dir.path().join("dotfiles-config.toml");
            fs::write(&real, "onboarding = false\n").unwrap();
            std::os::unix::fs::symlink(&real, &fake.config).unwrap();

            apply(&fake.herdr(), &Config::default()).unwrap();
            assert!(
                fs::symlink_metadata(&fake.config)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(fs::read_to_string(&real).unwrap().contains("$ai_usage"));
        }

        #[test]
        fn disabled_rows_are_not_added() {
            let fake = FakeHerdr::new(true);
            let config = Config {
                agent_rows: false,
                workspace_rows: WorkspaceRows::Off,
                ..Config::default()
            };
            let outcome = apply(&fake.herdr(), &config).unwrap();
            assert!(outcome.changed.is_empty());
            assert!(!fake.config.exists());
        }
    }
}
