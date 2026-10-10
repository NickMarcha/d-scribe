//! Runs a user-configured command-line AI tool (e.g. `claude -p`, `ollama run llama3`).
//! The prompt and transcript go in on stdin and the tool's stdout is the result, so
//! d-scribe has no provider-specific code.

use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub async fn run_command(command: &str, input: &str, work_dir: &Path) -> Result<String, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("No AI command set. Add one in Settings.".into());
    }
    std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;

    // Run through the shell so PATH lookup finds .cmd shims (npm-installed CLIs) too.
    #[cfg(windows)]
    let mut cmd = {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = Command::new("cmd");
        c.arg("/C").raw_arg(command).creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    // An empty working directory keeps agent-style CLIs away from unrelated files.
    let mut child = cmd
        .current_dir(work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start `{}`: {}", command, e))?;

    let mut stdin = child.stdin.take().ok_or("No stdin")?;
    stdin.write_all(input.as_bytes()).await.map_err(|e| e.to_string())?;
    drop(stdin);

    let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!("`{}` exited with {}: {}", command, output.status, detail));
    }
    if stdout.is_empty() {
        return Err(format!("`{}` returned no output.", command));
    }
    Ok(stdout)
}
