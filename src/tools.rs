//! Tool implementations used by the Jarvis assistant.
//!
//! The Python prototype supported two tools: a `shell_task` for running
//! arbitrary commands in the local shell and a `codex_cli_task` for
//! scaffolding code via the `codex` CLI. This module provides Rust
//! equivalents of those utilities. They return the stdout/stderr of the
//! executed program and attempt to provide useful error messages on
//! failure.

use anyhow::{Context, Result};
use std::process::Command;
use wait_timeout::ChildExt;

/// Execute a raw shell command and return its output. The command is
/// executed using the default system shell (`sh` on Unix and `cmd.exe`
/// on Windows). Stdout and stderr are captured and concatenated. If
/// the process exits with a non‑zero status the exit code and stderr
/// are returned instead of stdout.
pub fn run_shell_task(command: &str) -> Result<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Ok("No command provided.".to_string());
    }
    // Handle directory changes specially: update persistent working directory.
    let jarvis_io = crate::jarvis_io::JarvisIO::new();
    if let Some(arg) = trimmed.strip_prefix("cd ") {
        // Determine new path relative to current working directory if needed.
        let target = if std::path::Path::new(arg).is_absolute() {
            std::path::PathBuf::from(arg)
        } else if let Some(cwd) = jarvis_io.read_working_directory() {
            std::path::PathBuf::from(cwd).join(arg)
        } else {
            std::env::current_dir()?.join(arg)
        };
        let new_dir = std::fs::canonicalize(&target)
            .with_context(|| format!("failed to change directory to '{:?}'", target))?;
        if new_dir.is_dir() {
            jarvis_io.write_working_directory(new_dir.to_string_lossy().as_ref());
            return Ok(format!("Changed directory to {}", new_dir.display()));
        } else {
            return Ok(format!("Directory not found: {}", new_dir.display()));
        }
    }
    // On Windows use `cmd /C`, on other platforms use `sh -c` and set current_dir if configured.
    #[cfg(target_os = "windows")]
    let mut cmd = Command::new("cmd");
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("sh");
    #[cfg(target_os = "windows")]
    cmd.args(["/C", trimmed]);
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-c", trimmed]);
    if let Some(cwd) = jarvis_io.read_working_directory() {
        cmd.current_dir(cwd.trim());
    }
    let output = cmd.output().context("failed to execute shell command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        if !stderr.is_empty() {
            return Ok(format!("Command exited with {code}: {stderr}"));
        } else {
            return Ok(format!(
                "Command exited with {code} and produced no output."
            ));
        }
    }
    if !stdout.is_empty() {
        Ok(stdout)
    } else if !stderr.is_empty() {
        Ok(stderr)
    } else {
        Ok("Command ran successfully with no output.".to_string())
    }
}

/// Run the `codex` CLI in `--full-auto` mode with the provided natural
/// language instruction. This function assumes that the `codex` binary
/// is available on the system `PATH`. Execution is limited to a
/// reasonable duration; if the process times out an error message is
/// returned. As with [`run_shell_task`], stdout and stderr are
/// captured and formatted into a single string.
pub fn run_codex_cli(instruction: &str) -> Result<String> {
    let trimmed = instruction.trim();
    if trimmed.is_empty() {
        return Ok("No Codex instruction provided.".to_string());
    }
    // Quote the instruction so that spaces and special characters are
    // passed correctly to the codex binary. We rely on the shell to
    // perform argument parsing so we wrap the entire instruction in
    // double quotes and escape any existing quotes.
    let escaped = trimmed.replace('"', "\\\"");
    let full_cmd = format!(
        "codex --dangerously-bypass-approvals-and-sandbox \"{}\"",
        escaped
    );

    // Use the system shell to execute the command. This allows users to
    // set up aliases or wrappers for codex as desired. To prevent the
    // assistant from hanging indefinitely when Codex runs a long task or
    // encounters an unknown instruction, we spawn the process and
    // enforce a timeout.
    use std::time::Duration;
    // Spawn the Codex CLI process with piped stdout/stderr
    // Spawn the Codex CLI process, using persistent working directory if set.
    let jarvis_io = crate::jarvis_io::JarvisIO::new();
    #[cfg(target_os = "windows")]
    let mut child = {
        let mut c = Command::new("cmd");
        c.args(["/C", &full_cmd])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(cwd) = jarvis_io.read_working_directory() {
            c.current_dir(cwd.trim());
        }
        c.spawn().context("failed to spawn codex CLI")?
    };
    #[cfg(not(target_os = "windows"))]
    let mut child = {
        let mut c = Command::new("sh");
        c.args(["-c", &full_cmd])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(cwd) = jarvis_io.read_working_directory() {
            c.current_dir(cwd.trim());
        }
        c.spawn().context("failed to spawn codex CLI")?
    };
    // Use wait_timeout to wait for the process with a timeout
    let timeout = Duration::from_secs(60);
    match child
        .wait_timeout(timeout)
        .context("failed to wait on codex process")?
    {
        Some(status) => {
            // Process exited within timeout; capture output
            let output = child
                .wait_with_output()
                .context("failed to capture codex output")?;
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !status.success() {
                let code = status.code().unwrap_or(-1);
                if !stderr.is_empty() {
                    return Ok(format!("Codex CLI exited with {code}: {stderr}"));
                } else {
                    return Ok(format!(
                        "Codex CLI exited with {code} and produced no output."
                    ));
                }
            }
            if !stdout.is_empty() {
                Ok(stdout)
            } else if !stderr.is_empty() {
                Ok(stderr)
            } else {
                Ok("Codex ran successfully with no output.".to_string())
            }
        }
        None => {
            // Timeout expired; kill the process and return message
            let _ = child.kill();
            // Wait for the process to exit and clean up resources
            let _ = child.wait();
            Ok("Codex CLI timed out. Please try again with a simpler or more specific instruction.".to_string())
        }
    }
}
