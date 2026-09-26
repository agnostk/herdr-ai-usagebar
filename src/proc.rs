//! Run a child process with a deadline, capturing its output.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const POLL: Duration = Duration::from_millis(20);
const BUSY_RETRIES: u32 = 20;

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// stderr, or the exit status when the program printed nothing.
    pub fn failure_message(&self) -> String {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            self.status.to_string()
        } else {
            stderr.to_string()
        }
    }
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own group, so a timeout can kill everything it started.
        command.process_group(0);
    }
    let mut child =
        retry_while_busy(|| command.spawn()).with_context(|| format!("cannot run {program}"))?;

    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            kill_tree(&mut child);
            // Killing the group closes the pipes, so the reader threads end.
            // They are not joined in case something escaped the group.
            bail!("{program} timed out after {}s", timeout.as_secs());
        }
        thread::sleep(POLL);
    };

    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Retry an exec that failed with ETXTBSY. Linux refuses to execute a file
/// that some process still holds open for writing, which happens briefly
/// while a binary is being replaced (an ai-usagebar upgrade) or when another
/// thread forks right after the file was written.
pub fn retry_while_busy<T>(mut attempt: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match attempt() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && tries < BUSY_RETRIES =>
            {
                tries += 1;
                thread::sleep(POLL);
            }
            result => return result,
        }
    }
}

fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_status() {
        let output = run(
            Command::new("sh").args(["-c", "echo out; echo err >&2; exit 3"]),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output.stdout, "out\n");
        assert_eq!(output.stderr, "err\n");
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(output.failure_message(), "err");
    }

    #[test]
    fn kills_a_process_that_overruns_its_deadline() {
        let start = Instant::now();
        let error = run(Command::new("sleep").arg("10"), Duration::from_millis(200)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_timeout_also_kills_grandchildren_holding_the_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        let start = Instant::now();
        run(
            Command::new("sh").args(["-c", &script]),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5));

        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let alive = Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            if !alive {
                break;
            }
            assert!(Instant::now() < deadline, "grandchild {pid} survived");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn retry_while_busy_retries_only_busy_errors() {
        use std::io::{Error, ErrorKind};
        let mut calls = 0;
        let result = retry_while_busy(|| {
            calls += 1;
            if calls < 3 {
                Err(Error::from(ErrorKind::ExecutableFileBusy))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(result.unwrap(), 3);

        let mut calls = 0;
        let result: std::io::Result<()> = retry_while_busy(|| {
            calls += 1;
            Err(Error::from(ErrorKind::NotFound))
        });
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
        assert_eq!(calls, 1);

        let mut calls = 0;
        let result: std::io::Result<()> = retry_while_busy(|| {
            calls += 1;
            Err(Error::from(ErrorKind::ExecutableFileBusy))
        });
        assert!(result.is_err());
        assert_eq!(calls, BUSY_RETRIES + 1);
    }

    /// Linux reports ETXTBSY while a writer holds the file; macOS does not.
    #[cfg(target_os = "linux")]
    #[test]
    fn runs_a_script_once_its_writer_closes() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("busy.sh");
        let mut writer = std::fs::File::create(&path).unwrap();
        writer.write_all(b"#!/bin/sh\necho ran\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let closer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            drop(writer);
        });
        let output = run(&mut Command::new(&path), Duration::from_secs(5)).unwrap();
        closer.join().unwrap();
        assert_eq!(output.stdout, "ran\n");
    }

    #[test]
    fn reports_a_missing_program() {
        let error = run(
            &mut Command::new("/nonexistent/herdr-ai-usagebar-test"),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot run"));
    }
}
