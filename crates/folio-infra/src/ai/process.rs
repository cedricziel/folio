//! Bounded execution of the external tools the pipeline shells out to

use std::io;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::process::Command;

/// Upper bound for one tool run. Far above what a single page or small PDF
/// needs, so it only fires on a stuck process.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(300);

/// Upper bound for a quick metadata call such as `pdfinfo`
pub const INFO_TIMEOUT: Duration = Duration::from_secs(30);

/// Kills the child's whole process group unless disarmed
///
/// `kill_on_drop` only reaches the direct child; tools such as `ocrmypdf`
/// start `tesseract` and Ghostscript that would otherwise outlive a timeout
/// or a cancelled job.
struct GroupKill(Option<u32>);

impl GroupKill {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: the child leads its own group (`process_group(0)`), so
            // its pid is the group id
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

/// Run `command` to completion in its own process group, killing the whole
/// group when `limit` passes or the future is dropped
///
/// The error is `ErrorKind::TimedOut` on expiry; callers already wrap io
/// errors with the tool name.
pub async fn output_within(command: &mut Command, limit: Duration) -> io::Result<Output> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);

    let child = command.spawn()?;
    let mut guard = GroupKill(child.id());
    let result = tokio::time::timeout(limit, child.wait_with_output()).await;
    match result {
        Ok(Ok(output)) => {
            guard.disarm();
            Ok(output)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("timed out after {}s", limit.as_secs_f32()),
        )),
    }
}

pub async fn output(command: &mut Command) -> io::Result<Output> {
    output_within(command, TOOL_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn gone(pid: i32) -> bool {
        for _ in 0..40 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn stuck_tool_and_its_children_are_killed_after_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let child_pid = dir.path().join("child");
        let grandchild_pid = dir.path().join("grandchild");
        let script = format!(
            "sleep 30 & echo $! > {}; echo $$ > {}; wait",
            grandchild_pid.display(),
            child_pid.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            output_within(&mut command, Duration::from_secs(2)),
        )
        .await
        .expect("must be bounded by the limit, not run to completion");

        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let read = |p: &std::path::Path| {
            std::fs::read_to_string(p)
                .unwrap()
                .trim()
                .parse::<i32>()
                .unwrap()
        };
        assert!(gone(read(&child_pid)).await, "child survived the timeout");
        assert!(
            gone(read(&grandchild_pid)).await,
            "grandchild survived the timeout"
        );
    }

    #[tokio::test]
    async fn quick_tool_returns_its_output() {
        let mut command = Command::new("echo");
        command.arg("hi");
        let out = output(&mut command).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }
}
