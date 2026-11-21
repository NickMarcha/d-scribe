//! Discord RPC IPC transport.
//!
//! IPC is the officially supported local RPC transport. Unlike the WebSocket transport it has
//! no Origin check, so the Discord app needs no RPC Origin configured. Windows uses named pipes
//! (`\\.\pipe\discord-ipc-N`); Linux and macOS use Unix sockets with the same name. Each
//! system's backend only opens the connection; framing is shared.
//!
//! Protocol: 8-byte header (opcode u32 LE + length u32 LE) + JSON payload
//! Opcodes: 0=HANDSHAKE, 1=FRAME, 2=CLOSE, 3=PING, 4=PONG

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

#[cfg(not(windows))]
#[path = "unsupported.rs"]
mod backend;

use log::info;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};


const OPCODE_HANDSHAKE: u32 = 0;
pub const OPCODE_FRAME: u32 = 1;
pub const OPCODE_CLOSE: u32 = 2;
pub const OPCODE_PING: u32 = 3;
pub const OPCODE_PONG: u32 = 4;

/// A connection to Discord: a named pipe, a Unix socket, or an in-memory stream in tests.
pub trait IpcStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> IpcStream for T {}

/// Connect to the running Discord app (or the fake set in `DSCRIBE_DISCORD_IPC`) and send
/// the handshake.
pub async fn connect_ipc(client_id: &str) -> Result<(IpcReader, IpcWriter), String> {
    let (name, stream) = match super::fake::ipc_path() {
        Some(path) => (path.clone(), backend::connect_to(&path).await?),
        None => backend::connect().await?,
    };
    info!("[discord-rpc] IPC connected to {}", name);
    handshake(Box::new(stream), client_id).await
}

/// Whether Discord (or the fake) is running, without connecting to it.
pub fn discord_running() -> bool {
    match super::fake::ipc_path() {
        Some(path) => backend::exists(&path),
        None => backend::discord_running(),
    }
}

/// Send the handshake on an open connection and split it into its read and write halves.
pub async fn handshake(stream: Box<dyn IpcStream>, client_id: &str) -> Result<(IpcReader, IpcWriter), String> {
    let (read, write) = tokio::io::split(stream);
    let mut writer = IpcWriter { inner: Box::new(write) };
    let handshake = serde_json::json!({ "v": 1, "client_id": client_id });
    writer.send(OPCODE_HANDSHAKE, &handshake.to_string()).await?;
    Ok((IpcReader { inner: Box::new(read) }, writer))
}

pub struct IpcWriter {
    inner: Box<dyn AsyncWrite + Send + Unpin>,
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
    inner: Box<dyn AsyncRead + Send + Unpin>,
}

impl IpcReader {
    /// Read next frame. Returns (opcode, json_string), or None when the connection is closed.
    /// Not cancel-safe: run it in its own task, not inside `select!`.
    pub async fn recv_frame(&mut self) -> Result<Option<(u32, String)>, String> {
        read_frame(&mut self.inner).await
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, opcode: u32, json: &str) -> Result<(), String> {
    let len = json.len() as u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..8].copy_from_slice(&len.to_le_bytes());
    w.write_all(&header).await.map_err(|e| e.to_string())?;
    w.write_all(json.as_bytes()).await.map_err(|e| e.to_string())?;
    w.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<(u32, String)>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handshake_is_the_first_frame() {
        let (ours, mut discord) = tokio::io::duplex(1024);
        let _conn = handshake(Box::new(ours), "123").await.unwrap();
        let mut header = [0u8; 8];
        discord.read_exact(&mut header).await.unwrap();
        assert_eq!(u32::from_le_bytes(header[0..4].try_into().unwrap()), OPCODE_HANDSHAKE);
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let mut body = vec![0u8; len];
        discord.read_exact(&mut body).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v, serde_json::json!({ "v": 1, "client_id": "123" }));
    }

    #[tokio::test]
    async fn frames_round_trip() {
        let (ours, mut discord) = tokio::io::duplex(1024);
        let (mut reader, mut writer) = handshake(Box::new(ours), "1").await.unwrap();
        read_frame(&mut discord).await.unwrap(); // handshake

        writer.send_json(r#"{"cmd":"X"}"#).await.unwrap();
        writer.send_pong("").await.unwrap();
        assert_eq!(read_frame(&mut discord).await.unwrap(), Some((OPCODE_FRAME, r#"{"cmd":"X"}"#.into())));
        assert_eq!(read_frame(&mut discord).await.unwrap(), Some((OPCODE_PONG, String::new())));

        write_frame(&mut discord, OPCODE_PING, "{}").await.unwrap();
        write_frame(&mut discord, OPCODE_FRAME, "{\"evt\":\"ünïcode\"}").await.unwrap();
        assert_eq!(reader.recv_frame().await.unwrap(), Some((OPCODE_PING, "{}".into())));
        assert_eq!(reader.recv_frame().await.unwrap(), Some((OPCODE_FRAME, "{\"evt\":\"ünïcode\"}".into())));
    }

    /// The `DSCRIBE_DISCORD_IPC` switch, end to end: the fake on a real named pipe.
    #[cfg(windows)]
    #[tokio::test]
    async fn connects_to_the_fake_on_a_named_pipe() {
        use tokio::net::windows::named_pipe::ServerOptions;
        let pipe = format!(r"\\.\pipe\d-scribe-test-{}", uuid::Uuid::new_v4());
        std::env::set_var("DSCRIBE_DISCORD_IPC", &pipe);
        assert!(!discord_running(), "no pipe yet");

        let fake = fake_discord::FakeDiscord::new(Default::default());
        let server = ServerOptions::new().first_pipe_instance(true).create(&pipe).unwrap();
        assert!(discord_running(), "the pipe is listed");
        tokio::spawn(async move {
            server.connect().await.unwrap();
            fake.serve(server).await;
        });

        let (mut reader, _writer) = connect_ipc("42").await.unwrap();
        let (opcode, ready) = reader.recv_frame().await.unwrap().unwrap();
        assert_eq!(opcode, OPCODE_FRAME);
        assert!(ready.contains("\"READY\""), "{}", ready);
        std::env::remove_var("DSCRIBE_DISCORD_IPC");
    }

    #[tokio::test]
    async fn closed_connection_reads_as_none() {
        let (ours, discord) = tokio::io::duplex(1024);
        let (mut reader, _writer) = handshake(Box::new(ours), "1").await.unwrap();
        drop(discord);
        assert_eq!(reader.recv_frame().await.unwrap(), None);
    }
}
