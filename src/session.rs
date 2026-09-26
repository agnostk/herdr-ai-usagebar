//! Per-herdr-session files under the plugin state directory. Plugins are
//! global to the user but each herdr session (server socket) runs its own
//! startup hook, so each gets its own background process, lock and status.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const PAUSE_TICK: Duration = Duration::from_millis(250);

pub struct Session {
    dir: PathBuf,
}

impl Session {
    pub fn new(state_dir: &Path, socket: &str) -> Self {
        Self {
            dir: state_dir
                .join("sessions")
                .join(format!("{:016x}", fnv1a(socket.as_bytes()))),
        }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.join("daemon.log")
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join("daemon.lock")
    }

    fn marker_path(&self, marker: Marker) -> PathBuf {
        self.dir.join(match marker {
            Marker::Stop => "stop",
            Marker::Refresh => "refresh",
        })
    }

    fn status_path(&self) -> PathBuf {
        self.dir.join("status.json")
    }

    /// The exclusive lock the background process holds for its lifetime.
    /// `None` means another process holds it. The OS releases it when the
    /// holder exits, so a crash never leaves a stale lock behind.
    pub fn try_lock(&self) -> Result<Option<File>> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create {}", self.dir.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path())?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }

    pub fn is_running(&self) -> Result<bool> {
        Ok(self.try_lock()?.is_none())
    }

    pub fn signal(&self, marker: Marker) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.marker_path(marker), b"").context("cannot signal the background process")
    }

    /// Consume a marker, returning whether it was set.
    pub fn take(&self, marker: Marker) -> bool {
        fs::remove_file(self.marker_path(marker)).is_ok()
    }

    pub fn is_signalled(&self, marker: Marker) -> bool {
        self.marker_path(marker).exists()
    }

    /// Sleep up to `duration`, returning early once any marker is set so
    /// `stop` and `refresh` take effect within a fraction of a second.
    pub fn pause(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let now = Instant::now();
            if now >= deadline
                || self.is_signalled(Marker::Stop)
                || self.is_signalled(Marker::Refresh)
            {
                return;
            }
            thread::sleep(PAUSE_TICK.min(deadline - now));
        }
    }

    pub fn write_status(&self, status: &Status) -> Result<()> {
        let tmp = self.dir.join("status.json.tmp");
        open_private(&tmp, false)?.write_all(&serde_json::to_vec_pretty(status)?)?;
        fs::rename(&tmp, self.status_path())?;
        Ok(())
    }

    pub fn read_status(&self) -> Option<Status> {
        let bytes = fs::read(self.status_path()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Marker {
    Stop,
    Refresh,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub version: String,
    pub socket: String,
    pub started_at: u64,
    pub ai_usagebar: Option<String>,
    pub last_fetch_at: Option<u64>,
    pub last_error: Option<String>,
    /// `id: ready` / `id: error` for each entry of the last good report.
    pub entries: Vec<String>,
}

/// Owner-only (0600) on Unix: the log and status file can carry provider
/// error text.
pub fn open_private(path: &Path, append: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`, so a rebuilt
/// binary finds the same session directory as the running one.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn sessions_are_separated_by_socket() {
        let state = tempfile::tempdir().unwrap();
        let one = Session::new(state.path(), "/run/herdr/default.sock");
        let two = Session::new(state.path(), "/run/herdr/work.sock");
        assert_ne!(one.dir(), two.dir());
        assert_eq!(
            one.dir(),
            Session::new(state.path(), "/run/herdr/default.sock").dir()
        );
    }

    #[test]
    fn the_lock_is_exclusive_until_released() {
        let state = tempfile::tempdir().unwrap();
        let session = Session::new(state.path(), "sock");
        let held = session.try_lock().unwrap().expect("first lock succeeds");
        assert!(session.try_lock().unwrap().is_none());
        assert!(session.is_running().unwrap());
        drop(held);
        // Tests running in parallel spawn processes, and a child between
        // fork and exec briefly shares the lock's file description.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while session.is_running().unwrap() {
            assert!(std::time::Instant::now() < deadline, "lock never released");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn markers_are_consumed_once() {
        let state = tempfile::tempdir().unwrap();
        let session = Session::new(state.path(), "sock");
        assert!(!session.take(Marker::Refresh));
        session.signal(Marker::Refresh).unwrap();
        assert!(session.take(Marker::Refresh));
        assert!(!session.take(Marker::Refresh));
        assert!(!session.take(Marker::Stop));
    }

    #[test]
    fn pause_returns_early_when_signalled() {
        let state = tempfile::tempdir().unwrap();
        let session = Session::new(state.path(), "sock");
        let start = Instant::now();
        session.pause(Duration::from_millis(100));
        assert!(start.elapsed() >= Duration::from_millis(100));

        for marker in [Marker::Stop, Marker::Refresh] {
            session.signal(marker).unwrap();
            let start = Instant::now();
            session.pause(Duration::from_secs(30));
            assert!(start.elapsed() < Duration::from_secs(1));
            assert!(session.take(marker));
        }
    }

    #[test]
    fn status_round_trips() {
        let state = tempfile::tempdir().unwrap();
        let session = Session::new(state.path(), "sock");
        assert_eq!(session.read_status(), None);
        fs::create_dir_all(session.dir()).unwrap();
        let status = Status {
            pid: 42,
            entries: vec!["anthropic: ready".into()],
            ..Status::default()
        };
        session.write_status(&status).unwrap();
        assert_eq!(session.read_status(), Some(status));
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only_even_if_they_existed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        fs::write(&path, "old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        open_private(&path, true)
            .unwrap()
            .write_all(b"new\n")
            .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "old\nnew\n");

        let session = Session::new(dir.path(), "sock");
        fs::create_dir_all(session.dir()).unwrap();
        session.write_status(&Status::default()).unwrap();
        let status = session.dir().join("status.json");
        assert_eq!(
            fs::metadata(status).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
