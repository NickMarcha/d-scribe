//! Discord RPC client. Uses IPC (named pipes) on Windows (officially supported);
//! falls back to WebSocket on other platforms or if IPC fails.

use crate::discord_rpc::events::{ChannelInfo, SpeakingEvent, VoiceChannel};
use crate::discord_rpc::set_channel_info;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        protocol::Message,
    },
};
use uuid::Uuid;

const RPC_PORTS: std::ops::Range<u16> = 6463..6473; // 6463 to 6472 inclusive
const RPC_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum RpcConnectionState {
    Disconnected,
    Connecting,
    AwaitingAuth,
    Authenticated,
    Subscribed,
    Error(String),
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RpcPayload {
    cmd: Option<String>,
    evt: Option<String>,
    nonce: Option<String>,
    data: Option<serde_json::Value>,
    #[serde(rename = "args")]
    _args: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorData {
    code: Option<i64>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthorizeData {
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthenticateData {
    user: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SpeakingData {
    #[serde(rename = "user_id")]
    user_id: Option<String>,
}

pub struct DiscordRpcClient {
    client_id: String,
    client_secret: String,
    rpc_origin: String,
    state: Arc<RpcLock>,
}

struct RpcLock {
    connection_state: RwLock<RpcConnectionState>,
    pending: RwLock<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>,
}

impl DiscordRpcClient {
    fn enhance_error(err: &str) -> String {
        if err.contains("Invalid Origin") {
            format!(
                "{}. Add https://localhost to your app's RPC Origins in the Discord Developer Portal. \
                RPC Origin is separate from OAuth2 Redirects. If you don't see an RPC Origin field, \
                your app may not have RPC access (RPC is in private beta).",
                err
            )
        } else {
            err.to_string()
        }
    }

    pub fn new(client_id: String, client_secret: String, rpc_origin: String) -> Self {
        Self {
            client_id,
            client_secret,
            rpc_origin,
            state: Arc::new(RpcLock {
                connection_state: RwLock::new(RpcConnectionState::Disconnected),
                pending: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn connect(
        &self,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
    ) -> Result<(), String> {
        *self.state.connection_state.write().await = RpcConnectionState::Connecting;

        // On Windows: try IPC first (officially supported, no Origin validation)
        #[cfg(windows)]
        {
            if let Ok(ipc) = crate::discord_rpc::ipc::connect_ipc(&self.client_id).await {
                let state = self.state.clone();
                let client_id = self.client_id.clone();
                let client_secret = self.client_secret.clone();
                let tx = tx.clone();
                let rpc_origin = self.rpc_origin.clone();

                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    match Self::run_connection_ipc(
                        ipc,
                        &state,
                        &client_id,
                        &client_secret,
                        &rpc_origin,
                        tx,
                        ready_tx,
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(e) => {
                            error!("[discord-rpc] IPC connection error: {}", e);
                            *state.connection_state.write().await =
                                RpcConnectionState::Error(e.clone());
                        }
                    }
                });

                match ready_rx.await {
                    Ok(Ok(())) => {
                        info!("[discord-rpc] Auth flow complete (IPC), channel info set");
                        return Ok(());
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_) => return Err("Connection task dropped".into()),
                }
            }
            info!("[discord-rpc] IPC failed, falling back to WebSocket");
        }

        let mut last_error = None;
        for port in RPC_PORTS {
            let url = format!(
                "ws://127.0.0.1:{}/?v={}&client_id={}&encoding=json",
                port, RPC_VERSION, self.client_id
            );

            let mut request = url
                .as_str()
                .into_client_request()
                .map_err(|e| e.to_string())?;
            request.headers_mut().insert(
                "Origin",
                http::header::HeaderValue::from_str(&self.rpc_origin).map_err(|e| e.to_string())?,
            );

            match connect_async(request).await {
                Ok((ws_stream, _)) => {
                    info!("[discord-rpc] WebSocket connected on port {}", port);
                    let (write, read) = ws_stream.split();
                    let state = self.state.clone();
                    let client_id = self.client_id.clone();
                    let client_secret = self.client_secret.clone();
                    let tx = tx.clone();
                    let rpc_origin = self.rpc_origin.clone();

                    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        match Self::run_connection(
                            write,
                            read,
                            &state,
                            &client_id,
                            &client_secret,
                            &rpc_origin,
                            tx,
                            ready_tx,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(e) => {
                                error!("[discord-rpc] Connection error: {}", e);
                                *state.connection_state.write().await =
                                    RpcConnectionState::Error(e.clone());
                            }
                        }
                    });

                    match ready_rx.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => return Err(e),
                        Err(_) => return Err("Connection task dropped".into()),
                    }
                    info!("[discord-rpc] Auth flow complete, channel info set");
                    return Ok(());
                }
                Err(e) => {
                    debug!("[discord-rpc] Port {} failed: {}", port, e);
                    last_error = Some(e.to_string());
                    continue;
                }
            }
        }

        *self.state.connection_state.write().await =
            RpcConnectionState::Error(last_error.unwrap_or_else(|| "No RPC port available".into()));
        Err("Could not connect to Discord. Is Discord running?".into())
    }

    async fn run_connection<W, R, E>(
        mut write: W,
        mut read: R,
        state: &RpcLock,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
        ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    ) -> Result<(), String>
    where
        W: SinkExt<Message> + Unpin,
        W::Error: std::fmt::Display,
        R: StreamExt<Item = Result<Message, E>> + Unpin,
        E: std::fmt::Display,
    {
        // Wait for READY
        info!("[discord-rpc] Waiting for READY...");
        let mut got_ready = false;
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| e.to_string())?;
            match msg {
                Message::Text(text) => {
                    if let Ok(payload) = serde_json::from_str::<RpcPayload>(&text) {
                        if payload.evt.as_deref() == Some("READY") {
                            info!("[discord-rpc] READY received");
                            got_ready = true;
                            break;
                        }
                        if payload.evt.as_deref() == Some("ERROR") {
                            let err_msg = payload
                                .data
                                .and_then(|d| serde_json::from_value::<ErrorData>(d).ok())
                                .and_then(|d| d.message)
                                .unwrap_or_else(|| "Unknown error".into());
                            let _ = ready_tx.send(Err(Self::enhance_error(&err_msg)));
                            return Err(Self::enhance_error(&err_msg));
                        }
                    }
                }
                Message::Close(frame) => {
                    let reason = frame
                        .as_ref()
                        .map(|f| f.reason.to_string())
                        .unwrap_or_else(|| "Connection closed by Discord".into());
                    let err = Self::enhance_error(&reason);
                    let _ = ready_tx.send(Err(err.clone()));
                    return Err(err);
                }
                _ => {} // Ping, Pong, Binary - ignore
            }
        }
        if !got_ready {
            let err = "Connection closed before READY".to_string();
            let _ = ready_tx.send(Err(err.clone()));
            return Err(err);
        }

        *state.connection_state.write().await = RpcConnectionState::AwaitingAuth;
        info!("[discord-rpc] Sending AUTHORIZE (approve in Discord popup)...");

        // AUTHORIZE
        let nonce = Uuid::new_v4().to_string();
        let auth_cmd = serde_json::json!({
            "cmd": "AUTHORIZE",
            "nonce": nonce,
            "args": {
                "client_id": client_id,
                "scopes": ["rpc", "identify"]
            }
        });

        let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel();
        state.pending.write().await.insert(nonce.clone(), tx_oneshot);
        write
            .send(Message::Text(auth_cmd.to_string()))
            .await
            .map_err(|e| e.to_string())?;

        let auth_response = rx_oneshot.await.map_err(|_| "Auth response channel closed")?;
        let code = match auth_response.get("code").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => {
                let err = "No authorization code. Did you approve in the Discord popup? If no popup appeared, check RPC Origin and Redirect URI.".to_string();
                let _ = ready_tx.send(Err(err.clone()));
                return Err(err);
            }
        };
        debug!("[discord-rpc] Got auth code, exchanging for token...");

        // Exchange code for access token
        let client = reqwest::Client::new();
        let token_response = client
            .post("https://discord.com/api/oauth2/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !token_response.status().is_success() {
            let status = token_response.status();
            let body = token_response.text().await.unwrap_or_default();
            let err = format!(
                "Token exchange failed ({}): {}. Ensure OAuth2 Redirect URI is exactly {} in your Discord app.",
                status, body, redirect_uri
            );
            let _ = ready_tx.send(Err(err.clone()));
            return Err(err);
        }

        let token_data: serde_json::Value = token_response.json().await.map_err(|e| e.to_string())?;
        let access_token = match token_data.get("access_token").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                let err = "No access_token in response. Check that Redirect URI in OAuth2 matches exactly (e.g. https://localhost).".to_string();
                let _ = ready_tx.send(Err(err.clone()));
                return Err(err);
            }
        };

        // AUTHENTICATE
        let nonce = Uuid::new_v4().to_string();
        let auth_cmd = serde_json::json!({
            "cmd": "AUTHENTICATE",
            "nonce": nonce,
            "args": {
                "access_token": access_token.as_str()
            }
        });

        let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel();
        state.pending.write().await.insert(nonce.clone(), tx_oneshot);
        write
            .send(Message::Text(auth_cmd.to_string()))
            .await
            .map_err(|e| e.to_string())?;

        let auth_response = rx_oneshot.await.map_err(|_| "Auth response closed")?;
        let self_user_id = auth_response
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        *state.connection_state.write().await = RpcConnectionState::Authenticated;

        // GET_SELECTED_VOICE_CHANNEL
        let nonce = Uuid::new_v4().to_string();
        let get_channel_cmd = serde_json::json!({
            "cmd": "GET_SELECTED_VOICE_CHANNEL",
            "nonce": nonce,
            "args": {}
        });

        let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel();
        state.pending.write().await.insert(nonce.clone(), tx_oneshot);
        write
            .send(Message::Text(get_channel_cmd.to_string()))
            .await
            .map_err(|e| e.to_string())?;

        info!("[discord-rpc] Getting voice channel...");
        let channel_response = rx_oneshot.await.map_err(|_| "Channel response closed")?;
        let channel_id = match channel_response.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                let err = "Not in a voice channel. Join a voice channel in Discord first, then click Connect.".to_string();
                let _ = ready_tx.send(Err(err.clone()));
                return Err(err);
            }
        };
        let channel_name = channel_response
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from);
        let guild_id = channel_response
            .get("guild_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut user_labels = std::collections::HashMap::new();
        if let Some(states) = channel_response.get("voice_states").and_then(|v| v.as_array()) {
            for vs in states {
                let user = vs.get("user");
                let user_id = user
                    .and_then(|u| u.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let username = user
                    .and_then(|u| u.get("username"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let nick = vs.get("nick").and_then(|v| v.as_str()).map(String::from);
                if let Some(uid) = user_id {
                    let label = nick.or(username).unwrap_or_else(|| uid.clone());
                    user_labels.insert(uid, label);
                }
            }
        }

        set_channel_info(ChannelInfo {
            channel_id: channel_id.clone(),
            channel_name: channel_name.clone(),
            guild_id,
            guild_name: None, // Would need GET_GUILD to fetch
            self_user_id,
            user_labels,
        });
        info!(
            "[discord-rpc] Channel info set: {} ({})",
            channel_name.as_deref().unwrap_or("?"),
            channel_id
        );

        // Signal ready BEFORE message loop - connect() is waiting
        if ready_tx.send(Ok(())).is_err() {
            warn!("[discord-rpc] ready_tx already dropped");
        }

        // SUBSCRIBE to SPEAKING_START and SPEAKING_STOP
        for evt in ["SPEAKING_START", "SPEAKING_STOP"] {
            let nonce = Uuid::new_v4().to_string();
            let sub_cmd = serde_json::json!({
                "cmd": "SUBSCRIBE",
                "nonce": nonce,
                "evt": evt,
                "args": { "channel_id": channel_id.clone() }
            });

            let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel();
            state.pending.write().await.insert(nonce.clone(), tx_oneshot);
            write
                .send(Message::Text(sub_cmd.to_string()))
                .await
                .map_err(|e| e.to_string())?;

            let _ = rx_oneshot.await;
        }

        *state.connection_state.write().await = RpcConnectionState::Subscribed;

        // Process incoming messages
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| e.to_string())?;
            if let Message::Text(text) = msg {
                if let Ok(payload) = serde_json::from_str::<RpcPayload>(&text) {
                    let evt = payload.evt.as_deref();
                    let data = payload.data.clone();

                    if let Some(nonce) = &payload.nonce {
                        if let Some(tx) = state.pending.write().await.remove(nonce) {
                            if let Some(ref d) = data {
                                let _ = tx.send(d.clone());
                            }
                        }
                    }
                    if evt == Some("SPEAKING_START") || evt == Some("SPEAKING_STOP") {
                        if let Some(ref d) = data {
                            if let Ok(speaking) = serde_json::from_value::<SpeakingData>(d.clone()) {
                                if let Some(user_id) = speaking.user_id {
                                    debug!("[discord-rpc] {:?} user_id={}", evt, user_id);
                                    let event = if evt == Some("SPEAKING_START") {
                                        SpeakingEvent::Start { user_id }
                                    } else {
                                        SpeakingEvent::Stop { user_id }
                                    };
                                    let _ = tx.send(event);
                                }
                            }
                        }
                    }
                    if evt == Some("ERROR") {
                        let err_msg = data
                            .and_then(|d| serde_json::from_value::<ErrorData>(d).ok())
                            .and_then(|d| d.message)
                            .unwrap_or_else(|| "Unknown error".into());
                        *state.connection_state.write().await =
                            RpcConnectionState::Error(err_msg.clone());
                        return Err(err_msg);
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(windows)]
    async fn ipc_read_response(
        ipc: &mut crate::discord_rpc::ipc::IpcConnection,
        expected_nonce: &str,
    ) -> Result<serde_json::Value, String> {
        loop {
            match ipc.recv_frame().await? {
                Some((1, text)) => {
                    if let Ok(payload) = serde_json::from_str::<RpcPayload>(&text) {
                        if payload.evt.as_deref() == Some("ERROR") {
                            let err_msg = payload
                                .data
                                .and_then(|d| serde_json::from_value::<ErrorData>(d).ok())
                                .and_then(|d| d.message)
                                .unwrap_or_else(|| "Unknown error".into());
                            return Err(Self::enhance_error(&err_msg));
                        }
                        if payload.nonce.as_deref() == Some(expected_nonce) {
                            return Ok(payload.data.unwrap_or(serde_json::Value::Null));
                        }
                    }
                }
                Some((2, _)) => return Err("Connection closed by Discord".into()),
                Some((3, ping_data)) => {
                    crate::discord_rpc::ipc::send_pong(ipc, &ping_data).await?;
                }
                Some((_, _)) | None => {}
            }
        }
    }

    #[cfg(windows)]
    async fn run_connection_ipc(
        mut ipc: crate::discord_rpc::ipc::IpcConnection,
        state: &RpcLock,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
        ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    ) -> Result<(), String> {
        let mut ready_tx = Some(ready_tx);
        let mut send_err = |e: String| {
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(Err(e.clone()));
            }
            e
        };

        // Wait for READY (first frame after handshake)
        info!("[discord-rpc] Waiting for READY (IPC)...");
        let mut got_ready = false;
        loop {
            match ipc.recv_frame().await? {
                Some((1, text)) => {
                    if let Ok(payload) = serde_json::from_str::<RpcPayload>(&text) {
                        if payload.evt.as_deref() == Some("READY") {
                            info!("[discord-rpc] READY received");
                            got_ready = true;
                            break;
                        }
                        if payload.evt.as_deref() == Some("ERROR") {
                            let err_msg = payload
                                .data
                                .and_then(|d| serde_json::from_value::<ErrorData>(d).ok())
                                .and_then(|d| d.message)
                                .unwrap_or_else(|| "Unknown error".into());
                            return Err(send_err(Self::enhance_error(&err_msg)));
                        }
                    }
                }
                Some((3, ping_data)) => {
                    crate::discord_rpc::ipc::send_pong(&mut ipc, &ping_data).await?;
                }
                Some((2, _)) => {
                    return Err(send_err("Connection closed by Discord".into()));
                }
                Some((_, _)) => {}
                None => break,
            }
        }
        if !got_ready {
            return Err(send_err("Connection closed before READY".into()));
        }

        *state.connection_state.write().await = RpcConnectionState::AwaitingAuth;
        info!("[discord-rpc] Sending AUTHORIZE (approve in Discord popup)...");

        // AUTHORIZE
        let nonce = Uuid::new_v4().to_string();
        let auth_cmd = serde_json::json!({
            "cmd": "AUTHORIZE",
            "nonce": nonce,
            "args": {
                "client_id": client_id,
                "scopes": ["rpc", "identify"]
            }
        });

        ipc.send_json(&auth_cmd.to_string()).await?;
        let auth_response = Self::ipc_read_response(&mut ipc, &nonce).await
            .map_err(&mut send_err)?;
        let code = match auth_response.get("code").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => {
                return Err(send_err("No authorization code. Did you approve in the Discord popup? If no popup appeared, check OAuth2 Redirect URI.".into()));
            }
        };
        debug!("[discord-rpc] Got auth code, exchanging for token...");

        // Exchange code for access token
        let client = reqwest::Client::new();
        let token_response = client
            .post("https://discord.com/api/oauth2/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !token_response.status().is_success() {
            let status = token_response.status();
            let body = token_response.text().await.unwrap_or_default();
            return Err(send_err(format!(
                "Token exchange failed ({}): {}. Ensure OAuth2 Redirect URI is exactly {} in your Discord app.",
                status, body, redirect_uri
            )));
        }

        let token_data: serde_json::Value = token_response.json().await.map_err(|e| send_err(e.to_string()))?;
        let access_token = match token_data.get("access_token").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return Err(send_err("No access_token in response. Check that Redirect URI in OAuth2 matches exactly (e.g. https://localhost).".into()));
            }
        };

        // AUTHENTICATE
        let nonce = Uuid::new_v4().to_string();
        let auth_cmd = serde_json::json!({
            "cmd": "AUTHENTICATE",
            "nonce": nonce,
            "args": {
                "access_token": access_token.as_str()
            }
        });

        ipc.send_json(&auth_cmd.to_string()).await?;
        let auth_response = Self::ipc_read_response(&mut ipc, &nonce).await
            .map_err(&mut send_err)?;
        let self_user_id = auth_response
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        *state.connection_state.write().await = RpcConnectionState::Authenticated;

        // GET_SELECTED_VOICE_CHANNEL
        let nonce = Uuid::new_v4().to_string();
        let get_channel_cmd = serde_json::json!({
            "cmd": "GET_SELECTED_VOICE_CHANNEL",
            "nonce": nonce,
            "args": {}
        });

        ipc.send_json(&get_channel_cmd.to_string()).await?;
        info!("[discord-rpc] Getting voice channel...");
        let channel_response = Self::ipc_read_response(&mut ipc, &nonce).await
            .map_err(&mut send_err)?;
        let channel_id = match channel_response.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                return Err(send_err("Not in a voice channel. Join a voice channel in Discord first, then click Connect.".into()));
            }
        };
        let channel_name = channel_response
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from);
        let guild_id = channel_response
            .get("guild_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut user_labels = std::collections::HashMap::new();
        if let Some(states) = channel_response.get("voice_states").and_then(|v| v.as_array()) {
            for vs in states {
                let user = vs.get("user");
                let user_id = user
                    .and_then(|u| u.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let username = user
                    .and_then(|u| u.get("username"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let nick = vs.get("nick").and_then(|v| v.as_str()).map(String::from);
                if let Some(uid) = user_id {
                    let label = nick.or(username).unwrap_or_else(|| uid.clone());
                    user_labels.insert(uid, label);
                }
            }
        }

        set_channel_info(ChannelInfo {
            channel_id: channel_id.clone(),
            channel_name: channel_name.clone(),
            guild_id,
            guild_name: None,
            self_user_id,
            user_labels,
        });
        info!(
            "[discord-rpc] Channel info set: {} ({})",
            channel_name.as_deref().unwrap_or("?"),
            channel_id
        );

        if let Some(tx) = ready_tx.take() {
            if tx.send(Ok(())).is_err() {
                warn!("[discord-rpc] ready_tx already dropped");
            }
        }

        // SUBSCRIBE to SPEAKING_START and SPEAKING_STOP
        for evt in ["SPEAKING_START", "SPEAKING_STOP"] {
            let nonce = Uuid::new_v4().to_string();
            let sub_cmd = serde_json::json!({
                "cmd": "SUBSCRIBE",
                "nonce": nonce,
                "evt": evt,
                "args": { "channel_id": channel_id.clone() }
            });

            ipc.send_json(&sub_cmd.to_string()).await?;
            let _ = Self::ipc_read_response(&mut ipc, &nonce).await?;
        }

        *state.connection_state.write().await = RpcConnectionState::Subscribed;

        // Process incoming messages - we need to handle the pending responses from SUBSCRIBE
        // and then the SPEAKING_START/STOP events. The SUBSCRIBE responses will complete the
        // rx_oneshot. We need a message loop that processes frames and dispatches to pending
        // and to the tx channel for speaking events.
        loop {
            match ipc.recv_frame().await? {
                Some((1, text)) => {
                    if let Ok(payload) = serde_json::from_str::<RpcPayload>(&text) {
                        let evt = payload.evt.as_deref();
                        let data = payload.data.clone();

                        if let Some(nonce) = &payload.nonce {
                            if let Some(tx) = state.pending.write().await.remove(nonce) {
                                if let Some(ref d) = data {
                                    let _ = tx.send(d.clone());
                                }
                            }
                        }
                        if evt == Some("SPEAKING_START") || evt == Some("SPEAKING_STOP") {
                            if let Some(ref d) = data {
                                if let Ok(speaking) = serde_json::from_value::<SpeakingData>(d.clone()) {
                                    if let Some(user_id) = speaking.user_id {
                                        debug!("[discord-rpc] {:?} user_id={}", evt, user_id);
                                        let event = if evt == Some("SPEAKING_START") {
                                            SpeakingEvent::Start { user_id }
                                        } else {
                                            SpeakingEvent::Stop { user_id }
                                        };
                                        let _ = tx.send(event);
                                    }
                                }
                            }
                        }
                        if evt == Some("ERROR") {
                            let err_msg = data
                                .and_then(|d| serde_json::from_value::<ErrorData>(d).ok())
                                .and_then(|d| d.message)
                                .unwrap_or_else(|| "Unknown error".into());
                            *state.connection_state.write().await =
                                RpcConnectionState::Error(err_msg.clone());
                            return Err(err_msg);
                        }
                    }
                }
                Some((2, _)) => break,
                Some((3, _)) => {}
                Some((_, _)) => {}
                None => break,
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn connection_state(&self) -> RpcConnectionState {
        self.state.connection_state.read().await.clone()
    }

    #[allow(dead_code)]
    pub async fn get_selected_voice_channel(&self) -> Result<Option<VoiceChannel>, String> {
        // This would need an active connection - for now we'll get it during connect
        Ok(None)
    }
}
