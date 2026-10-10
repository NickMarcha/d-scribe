//! Discord RPC IPC transport (Windows named pipes).
//!
//! IPC is the officially supported local RPC transport. Unlike the WebSocket transport it has
//! no Origin check, so the Discord app needs no RPC Origin configured.
//!
//! Protocol: 8-byte header (opcode u32 LE + length u32 LE) + JSON payload
//! Opcodes: 0=HANDSHAKE, 1=FRAME, 2=CLOSE, 3=PING, 4=PONG

#![cfg(windows)]

use log::{debug, info};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

const OPCODE_HANDSHAKE: u32 = 0;
pub const OPCODE_FRAME: u32 = 1;
pub const OPCODE_CLOSE: u32 = 2;
pub const OPCODE_PING: u32 = 3;
const OPCODE_PONG: u32 = 4;

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

/// Connect to Discord via IPC (named pipes) and send the handshake.
/// Tries discord-ipc-0 through discord-ipc-9.
pub async fn connect_ipc(client_id: &str) -> Result<(IpcReader, IpcWriter), String> {
    for i in 0..10 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        match open_pipe(&pipe_name).await {
            Ok(client) => {
                info!("[discord-rpc] IPC connected to {}", pipe_name);
                let (read, write) = tokio::io::split(client);
                let mut writer = IpcWriter { inner: write };
                let handshake = serde_json::json!({ "v": 1, "client_id": client_id });
                writer.send(OPCODE_HANDSHAKE, &handshake.to_string()).await?;
                return Ok((IpcReader { inner: read }, writer));
            }
            Err(e) => debug!("[discord-rpc] IPC pipe {} failed: {}", pipe_name, e),
        }
    }
    Err("Could not connect to Discord. Is the Discord desktop app running?".into())
}

pub struct IpcWriter {
    inner: WriteHalf<NamedPipeClient>,
}

impl IpcWriter {
    async fn send(&mut self, opcode: u32, json: &str) -> Result<(), String> {
        write_frame(&mut self.inner, opcode, json).await
    }

    /// Send a FRAME (opcode 1) with JSON payload.
    pub async fn send_json(&mut self, json: &str) -> Result<(), String> {
        self.send(OPCODE_FRAME, json).await
    }

    /// Send PONG (opcode 4) in response to PING.
    pub async fn send_pong(&mut self, payload: &str) -> Result<(), String> {
        self.send(OPCODE_PONG, payload).await
    }
}

pub struct IpcReader {
    inner: ReadHalf<NamedPipeClient>,
}

impl IpcReader {
    /// Read next frame. Returns (opcode, json_string), or None when the pipe is closed.
    /// Not cancel-safe: run it in its own task, not inside `select!`.
    pub async fn recv_frame(&mut self) -> Result<Option<(u32, String)>, String> {
        read_frame(&mut self.inner).await
    }
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, opcode: u32, json: &str) -> Result<(), String> {
    let len = json.len() as u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..8].copy_from_slice(&len.to_le_bytes());
    w.write_all(&header).await.map_err(|e| e.to_string())?;
    w.write_all(json.as_bytes()).await.map_err(|e| e.to_string())?;
    w.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<(u32, String)>, String> {
    let mut header = [0u8; 8];
    if let Err(e) = r.read_exact(&mut header).await {
        if e.kind() == io::ErrorKind::UnexpectedEof || e.kind() == io::ErrorKind::BrokenPipe {
            return Ok(None);
        }
        return Err(e.to_string());
    }
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    if len == 0 {
        return Ok(Some((opcode, String::new())));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    let json = String::from_utf8(buf).map_err(|e| e.to_string())?;
    Ok(Some((opcode, json)))
}
