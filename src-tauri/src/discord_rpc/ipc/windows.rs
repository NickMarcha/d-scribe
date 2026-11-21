//! Discord IPC on Windows: named pipes `\\.\pipe\discord-ipc-0` to `-9`.

use log::debug;
use std::io;
use std::time::Duration;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

/// ERROR_PIPE_BUSY
const ERROR_PIPE_BUSY: i32 = 231;

async fn open_pipe(pipe_name: &str) -> io::Result<NamedPipeClient> {
    match ClientOptions::new().open(pipe_name) {
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ClientOptions::new().open(pipe_name)
        }
        other => other,
    }
}

/// Opens the first Discord pipe that answers. Returns its name and the connection.
pub async fn connect() -> Result<(String, NamedPipeClient), String> {
    for i in 0..10 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        match open_pipe(&pipe_name).await {
            Ok(client) => return Ok((pipe_name, client)),
            Err(e) => debug!("[discord-rpc] IPC pipe {} failed: {}", pipe_name, e),
        }
    }
    Err("Could not connect to Discord. Is the Discord desktop app running?".into())
}

/// Opens a given pipe, e.g. `\\.\pipe\d-scribe-fake-discord`.
pub async fn connect_to(pipe_name: &str) -> Result<NamedPipeClient, String> {
    open_pipe(pipe_name)
        .await
        .map_err(|e| format!("Could not connect to {}: {}", pipe_name, e))
}

/// Whether a pipe exists, by listing the pipe namespace (opening it would use up a connection).
pub fn exists(pipe_name: &str) -> bool {
    let Some(name) = pipe_name.strip_prefix(r"\\.\pipe\") else { return false };
    std::fs::read_dir(r"\\.\pipe\")
        .map(|entries| entries.flatten().any(|e| e.file_name().to_str() == Some(name)))
        .unwrap_or(false)
}

/// Discord desktop is running if one of its IPC pipes (discord-ipc-0..9) exists.
/// Lists the pipe namespace instead of opening pipes, so no connection is made.
pub fn discord_running() -> bool {
    std::fs::read_dir(r"\\.\pipe\")
        .map(|entries| entries.flatten().any(|e| e.file_name().to_str().is_some_and(is_ipc_name)))
        .unwrap_or(false)
}

fn is_ipc_name(name: &str) -> bool {
    name.strip_prefix("discord-ipc-")
        .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    #[test]
    fn ipc_names() {
        assert!(super::is_ipc_name("discord-ipc-0"));
        assert!(super::is_ipc_name("discord-ipc-9"));
        assert!(!super::is_ipc_name("discord-ipc-10"));
        assert!(!super::is_ipc_name("discord-ipc-"));
        assert!(!super::is_ipc_name("other-pipe"));
    }
}
