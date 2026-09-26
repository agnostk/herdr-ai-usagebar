//! Run a child process with a deadline, capturing its output.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const POLL: Duration = Duration::from_millis(20);

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
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot run {program}"))?;

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
    fn reports_a_missing_program() {
        let error = run(
            &mut Command::new("/nonexistent/herdr-ai-usagebar-test"),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot run"));
    }
}
