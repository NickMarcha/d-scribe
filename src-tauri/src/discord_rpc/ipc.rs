//! Discord RPC IPC transport (Windows named pipes).
//!
//! Discord officially supports IPC; WebSocket RPC is not publicly supported and requires
//! RPC Origin configuration. IPC uses named pipes and has no Origin validation.
//!
//! Protocol: 8-byte header (opcode u32 LE + length u32 LE) + JSON payload
//! Opcodes: 0=HANDSHAKE, 1=FRAME, 2=CLOSE, 3=PING, 4=PONG

#![cfg(windows)]

use log::{debug, info};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;

const OPCODE_HANDSHAKE: u32 = 0;
const OPCODE_FRAME: u32 = 1;
#[allow(dead_code)]
const OPCODE_CLOSE: u32 = 2;
#[allow(dead_code)]
const OPCODE_PING: u32 = 3;
const OPCODE_PONG: u32 = 4;

/// Connect to Discord via IPC (named pipes). Tries discord-ipc-0 through discord-ipc-9.
pub async fn connect_ipc(client_id: &str) -> Result<IpcConnection, String> {
    for i in 0..10 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        match ClientOptions::new().open(&pipe_name) {
            Ok(mut client) => {
                info!("[discord-rpc] IPC connected to {}", pipe_name);

                // Send HANDSHAKE
                let handshake = serde_json::json!({
                    "v": 1,
                    "client_id": client_id
                });
                let handshake_str = handshake.to_string();
                send_frame(&mut client, OPCODE_HANDSHAKE, &handshake_str).await?;

                return Ok(IpcConnection { pipe: client });
            }
            Err(e) => {
                let raw = e.raw_os_error();
                // ERROR_PIPE_BUSY = 231
                if raw == Some(231) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    // Retry same pipe
                    if let Ok(mut client) = ClientOptions::new().open(&pipe_name) {
                        let handshake = serde_json::json!({
                            "v": 1,
                            "client_id": client_id
                        });
                        let handshake_str = handshake.to_string();
                        send_frame(&mut client, OPCODE_HANDSHAKE, &handshake_str).await?;
                        return Ok(IpcConnection { pipe: client });
                    }
                }
                debug!("[discord-rpc] IPC pipe {} failed: {}", pipe_name, e);
            }
        }
    }
    Err("Could not connect to Discord via IPC. Is Discord running?".into())
}

/// Send PONG (opcode 4) in response to PING.
pub async fn send_pong(pipe: &mut IpcConnection, payload: &str) -> Result<(), String> {
    send_frame(&mut pipe.pipe, OPCODE_PONG, payload).await
}

async fn send_frame(
    pipe: &mut tokio::net::windows::named_pipe::NamedPipeClient,
    opcode: u32,
    json: &str,
) -> Result<(), String> {
    let len = json.len() as u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..8].copy_from_slice(&len.to_le_bytes());
    pipe.write_all(&header).await.map_err(|e| e.to_string())?;
    pipe.write_all(json.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    pipe.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub struct IpcConnection {
    pipe: tokio::net::windows::named_pipe::NamedPipeClient,
}

impl IpcConnection {
    /// Send a FRAME (opcode 1) with JSON payload.
    pub async fn send_json(&mut self, json: &str) -> Result<(), String> {
        send_frame(&mut self.pipe, OPCODE_FRAME, json).await
    }

    /// Read next frame. Returns (opcode, json_string). Blocks until a full frame is received.
    pub async fn recv_frame(&mut self) -> Result<Option<(u32, String)>, String> {
        // Read 8-byte header
        let mut header = [0u8; 8];
        if let Err(e) = self.pipe.read_exact(&mut header).await {
            if e.kind() == io::ErrorKind::UnexpectedEof {
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
        self.pipe
            .read_exact(&mut buf)
            .await
            .map_err(|e| e.to_string())?;
        let json = String::from_utf8(buf).map_err(|e| e.to_string())?;
        Ok(Some((opcode, json)))
    }
}
