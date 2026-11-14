//! Bounded execution of the external tools the pipeline shells out to

use std::io;
use std::process::Output;
use std::time::Duration;

use tokio::process::Command;

/// Upper bound for one tool run. Far above what a single page or small PDF
/// needs, so it only fires on a stuck process.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(300);

/// Run `command` to completion, killing it when `limit` passes
///
/// The error is `ErrorKind::TimedOut` on expiry; callers already wrap io
/// errors with the tool name.
pub async fn output_within(command: &mut Command, limit: Duration) -> io::Result<Output> {
    command.kill_on_drop(true);
    match tokio::time::timeout(limit, command.output()).await {
        Ok(result) => result,
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

    #[tokio::test]
    async fn stuck_tool_is_killed_after_the_limit() {
        let started = std::time::Instant::now();
        let mut command = Command::new("sleep");
        command.kill_on_drop(true).arg("30");

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            output_within(&mut command, Duration::from_millis(200)),
        )
        .await
        .expect("must be bounded by the limit, not run to completion");

        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn quick_tool_returns_its_output() {
        let mut command = Command::new("echo");
        command.arg("hi");
        let out = output(&mut command).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }
}
