//! Discord RPC client over IPC (Windows named pipes).
//!
//! Auth uses the OAuth2 public client flow with PKCE, so the user only needs a Client ID
//! (no Client Secret, no RPC Origin). Channel changes are detected by polling
//! GET_SELECTED_VOICE_CHANNEL, which works with `rpc.voice.read` (the VOICE_CHANNEL_SELECT
//! event would need the full `rpc` scope).

use crate::discord_rpc::events::SpeakingEvent;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// OAuth2 scopes requested in AUTHORIZE.
///
/// - `identify`: user id from AUTHENTICATE
/// - `rpc.voice.read`: GET_SELECTED_VOICE_CHANNEL and SPEAKING_START/SPEAKING_STOP
/// - `guilds`: guild names via the REST endpoint `GET /users/@me/guilds`
///
/// If Discord rejects these, switch back to full RPC with `&["rpc", "identify"]`.
/// Guild names then come from the RPC GET_GUILD command instead (see `has_scope("guilds")`).
pub const OAUTH_SCOPES: &[&str] = &["identify", "rpc.voice.read", "guilds"];

/// Sent as `redirect_uri` in the code exchange. Must be listed under OAuth2 Redirects
/// in the Discord app.
pub const OAUTH_REDIRECT_URI: &str = "https://localhost";

const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";

fn has_scope(scope: &str) -> bool {
    OAUTH_SCOPES.contains(&scope)
}

struct TokenSet {
    access_token: String,
    refresh_token: Option<String>,
}

/// PKCE verifier and S256 challenge, generated fresh for each authorization.
struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

async fn post_token(form: &[(&str, &str)]) -> Result<TokenSet, String> {
    let response = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Could not reach Discord: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let mut msg = format!("Discord token request failed ({}): {}", status, body);
        if body.contains("invalid_client") {
            msg.push_str(". Turn on Public Client in the OAuth2 page of your Discord app.");
        } else if body.contains("redirect_uri") {
            msg.push_str(&format!(
                ". Add {} under Redirects in the OAuth2 page of your Discord app.",
                OAUTH_REDIRECT_URI
            ));
        }
        return Err(msg);
    }

    let data: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("No access_token in Discord token response")?
        .to_string();
    let refresh_token = data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(TokenSet {
        access_token,
        refresh_token,
    })
}

/// Exchange an RPC authorization code for tokens (public client: no secret, PKCE verifier).
async fn exchange_code(client_id: &str, code: &str, code_verifier: &str) -> Result<TokenSet, String> {
    post_token(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
        ("redirect_uri", OAUTH_REDIRECT_URI),
    ])
    .await
}

/// Exchange a refresh token for a new access token (public client: no secret).
async fn refresh_access_token(client_id: &str, refresh_token: &str) -> Result<TokenSet, String> {
    post_token(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])
    .await
}

pub struct DiscordRpcClient {
    client_id: String,
}

impl DiscordRpcClient {
    pub fn new(client_id: String) -> Self {
        Self { client_id }
    }

    /// Connect and authorize (Discord shows an approval popup).
    /// Returns the refresh token to persist.
    pub async fn connect(
        &self,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
    ) -> Result<Option<String>, String> {
        self.start(tx, None).await
    }

    /// Connect using a stored refresh token (no popup). Returns the refresh token to persist.
    pub async fn connect_with_refresh_token(
        &self,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
        refresh_token: String,
    ) -> Result<Option<String>, String> {
        let tokens = refresh_access_token(&self.client_id, &refresh_token).await?;
        let refresh_to_save = tokens.refresh_token.unwrap_or(refresh_token);
        self.start(tx, Some(tokens.access_token)).await?;
        Ok(Some(refresh_to_save))
    }

    #[cfg(windows)]
    async fn start(
        &self,
        tx: mpsc::UnboundedSender<SpeakingEvent>,
        access_token: Option<String>,
    ) -> Result<Option<String>, String> {
        let (reader, writer) = crate::discord_rpc::ipc::connect_ipc(&self.client_id).await?;
        let conn = connection::Conn::new(reader, writer, tx);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(connection::run(
            conn,
            self.client_id.clone(),
            access_token,
            ready_tx,
        ));
        ready_rx
            .await
            .map_err(|_| "Discord connection task stopped".to_string())?
    }

    #[cfg(not(windows))]
    async fn start(
        &self,
        _tx: mpsc::UnboundedSender<SpeakingEvent>,
        _access_token: Option<String>,
    ) -> Result<Option<String>, String> {
        Err("Discord connection is only supported on Windows.".into())
    }
}

#[cfg(windows)]
mod connection {
    use super::*;
    use crate::discord_rpc::events::ChannelInfo;
    use crate::discord_rpc::ipc::{IpcReader, IpcWriter, OPCODE_CLOSE, OPCODE_FRAME, OPCODE_PING};
    use crate::discord_rpc::{clear_channel_info, set_channel_info, set_rpc_connected};
    use log::{debug, error, info, warn};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::time::MissedTickBehavior;
    use uuid::Uuid;

    const CHANNEL_POLL_INTERVAL: Duration = Duration::from_secs(2);
    const SPEAKING_EVENTS: [&str; 2] = ["SPEAKING_START", "SPEAKING_STOP"];
    const RPC_ERROR_OAUTH2: i64 = 5000;
    const USER_GUILDS_URL: &str = "https://discord.com/api/v10/users/@me/guilds";

    type Frame = Result<(u32, String), String>;

    fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
        v.get(key).and_then(|v| v.as_str())
    }

    struct RpcError {
        code: Option<i64>,
        message: String,
    }

    impl RpcError {
        fn from_data(data: &Value) -> Self {
            Self {
                code: data.get("code").and_then(|v| v.as_i64()),
                message: str_field(data, "message")
                    .unwrap_or("Unknown error")
                    .to_string(),
            }
        }

        fn other(message: String) -> Self {
            Self {
                code: None,
                message,
            }
        }
    }

    impl From<RpcError> for String {
        fn from(e: RpcError) -> String {
            match e.code {
                Some(code) => format!("{} (code {})", e.message, code),
                None => e.message,
            }
        }
    }

    /// Map an AUTHORIZE error to a user-facing message.
    fn authorize_error(err: RpcError) -> String {
        let lower = err.message.to_lowercase();
        let denied = lower.contains("denied") || lower.contains("cancel");
        let scope_rejected = lower.contains("invalid_scope")
            || lower.contains("scope")
            || (err.code == Some(RPC_ERROR_OAUTH2) && !denied);
        if scope_rejected {
            format!(
                "This Discord app is not allowed to use the permissions d-scribe asks for ({}). \
                 Only the app owner and App Testers can use them. In the Discord Developer Portal, \
                 add your Discord account as an App Tester for this app, or ask the app owner to add you. \
                 Discord said: {}",
                OAUTH_SCOPES.join(", "),
                err.message
            )
        } else if denied {
            "Authorization was cancelled. Click Connect and approve the request in the Discord popup."
                .to_string()
        } else {
            format!("Discord authorization failed: {}", String::from(err))
        }
    }

    fn close_message(text: &str) -> String {
        let reason = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| str_field(&v, "message").map(String::from))
            .unwrap_or_else(|| "no reason given".into());
        format!("Discord closed the connection: {}", reason)
    }

    /// One IPC connection. A reader task forwards frames into a channel so the main loop can
    /// wait on frames and the poll timer at the same time (pipe reads are not cancel-safe).
    pub(super) struct Conn {
        writer: IpcWriter,
        frames: mpsc::UnboundedReceiver<Frame>,
        reader_task: tokio::task::JoinHandle<()>,
        speaking_tx: mpsc::UnboundedSender<SpeakingEvent>,
    }

    impl Drop for Conn {
        fn drop(&mut self) {
            // Dropping the read half (with the writer) closes the pipe.
            self.reader_task.abort();
        }
    }

    impl Conn {
        pub(super) fn new(
            mut reader: IpcReader,
            writer: IpcWriter,
            speaking_tx: mpsc::UnboundedSender<SpeakingEvent>,
        ) -> Self {
            let (frame_tx, frames) = mpsc::unbounded_channel();
            let reader_task = tokio::spawn(async move {
                loop {
                    match reader.recv_frame().await {
                        Ok(Some(frame)) => {
                            if frame_tx.send(Ok(frame)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = frame_tx.send(Err(e));
                            break;
                        }
                    }
                }
            });
            Self {
                writer,
                frames,
                reader_task,
                speaking_tx,
            }
        }

        /// Handle a raw frame (answers PING). Returns the JSON payload for FRAME opcodes.
        async fn process_frame(&mut self, frame: Frame) -> Result<Option<Value>, String> {
            let (opcode, text) = frame?;
            match opcode {
                OPCODE_FRAME => match serde_json::from_str::<Value>(&text) {
                    Ok(v) => Ok(Some(v)),
                    Err(e) => {
                        debug!("[discord-rpc] Ignoring unparsable frame: {}", e);
                        Ok(None)
                    }
                },
                OPCODE_CLOSE => Err(close_message(&text)),
                OPCODE_PING => {
                    self.writer.send_pong(&text).await?;
                    Ok(None)
                }
                _ => Ok(None),
            }
        }

        async fn next_payload(&mut self) -> Result<Value, String> {
            loop {
                let frame = self
                    .frames
                    .recv()
                    .await
                    .ok_or_else(|| "Discord closed the connection".to_string())?;
                if let Some(payload) = self.process_frame(frame).await? {
                    return Ok(payload);
                }
            }
        }

        /// Send a command and wait for its response. Events that arrive meanwhile are dispatched.
        async fn request(
            &mut self,
            cmd: &str,
            args: Value,
            evt: Option<&str>,
        ) -> Result<Value, RpcError> {
            let nonce = Uuid::new_v4().to_string();
            let mut payload = json!({ "cmd": cmd, "nonce": nonce, "args": args });
            if let Some(evt) = evt {
                payload["evt"] = json!(evt);
            }
            self.writer
                .send_json(&payload.to_string())
                .await
                .map_err(RpcError::other)?;
            loop {
                let p = self.next_payload().await.map_err(RpcError::other)?;
                if str_field(&p, "nonce") == Some(nonce.as_str()) {
                    let data = p.get("data").cloned().unwrap_or(Value::Null);
                    if str_field(&p, "evt") == Some("ERROR") {
                        return Err(RpcError::from_data(&data));
                    }
                    return Ok(data);
                }
                self.dispatch_event(&p).map_err(RpcError::other)?;
            }
        }

        /// Handle an unsolicited payload: forward speaking events, fail on connection errors.
        fn dispatch_event(&self, p: &Value) -> Result<(), String> {
            let data = p.get("data").unwrap_or(&Value::Null);
            match str_field(p, "evt") {
                Some(evt @ ("SPEAKING_START" | "SPEAKING_STOP")) => {
                    if let Some(user_id) = str_field(data, "user_id") {
                        debug!("[discord-rpc] {} user_id={}", evt, user_id);
                        let user_id = user_id.to_string();
                        let event = if evt == "SPEAKING_START" {
                            SpeakingEvent::Start { user_id }
                        } else {
                            SpeakingEvent::Stop { user_id }
                        };
                        let _ = self.speaking_tx.send(event);
                    }
                    Ok(())
                }
                Some("ERROR") => {
                    let err = RpcError::from_data(data);
                    if str_field(p, "nonce").is_some() {
                        warn!(
                            "[discord-rpc] Error for an earlier command: {}",
                            String::from(err)
                        );
                        Ok(())
                    } else {
                        Err(err.into())
                    }
                }
                _ => Ok(()),
            }
        }

        async fn wait_ready(&mut self) -> Result<(), String> {
            loop {
                let p = self.next_payload().await?;
                match str_field(&p, "evt") {
                    Some("READY") => return Ok(()),
                    Some("ERROR") => {
                        return Err(RpcError::from_data(p.get("data").unwrap_or(&Value::Null)).into())
                    }
                    _ => {}
                }
            }
        }

        async fn set_speaking_subscription(
            &mut self,
            cmd: &str,
            channel_id: &str,
        ) -> Result<(), String> {
            for evt in SPEAKING_EVENTS {
                self.request(cmd, json!({ "channel_id": channel_id }), Some(evt))
                    .await
                    .map_err(|e| format!("{} {} failed: {}", cmd, evt, String::from(e)))?;
            }
            Ok(())
        }
    }

    async fn fetch_user_guilds(access_token: &str) -> Result<HashMap<String, String>, String> {
        let response = reqwest::Client::new()
            .get(USER_GUILDS_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("({}) {}", status, body));
        }
        let guilds: Vec<Value> = response.json().await.map_err(|e| e.to_string())?;
        Ok(guilds
            .iter()
            .filter_map(|g| Some((str_field(g, "id")?.to_string(), str_field(g, "name")?.to_string())))
            .collect())
    }

    fn user_labels(channel: &Value, self_user_id: Option<&str>) -> HashMap<String, String> {
        let mut labels = HashMap::new();
        if let Some(states) = channel.get("voice_states").and_then(|v| v.as_array()) {
            for vs in states {
                let user = vs.get("user").unwrap_or(&Value::Null);
                if let Some(uid) = str_field(user, "id") {
                    let label = str_field(vs, "nick")
                        .or_else(|| str_field(user, "username"))
                        .unwrap_or(uid);
                    labels.insert(uid.to_string(), label.to_string());
                }
            }
        }
        if let Some(uid) = self_user_id {
            labels
                .entry(uid.to_string())
                .or_insert_with(|| uid.to_string());
        }
        labels
    }

    /// Tracks the selected voice channel and keeps speaking subscriptions and ChannelInfo in sync.
    struct ChannelTracker {
        self_user_id: Option<String>,
        access_token: String,
        current_channel_id: Option<String>,
        /// Guild id -> name from `GET /users/@me/guilds` (only with the `guilds` scope).
        guild_names: HashMap<String, String>,
        /// Last resolved guild, so polling does not repeat lookups.
        last_guild: Option<(String, Option<String>)>,
    }

    impl ChannelTracker {
        async fn guild_name(&mut self, conn: &mut Conn, guild_id: &str) -> Option<String> {
            if let Some((id, name)) = &self.last_guild {
                if id == guild_id {
                    return name.clone();
                }
            }
            let name = if has_scope("guilds") {
                if !self.guild_names.contains_key(guild_id) {
                    match fetch_user_guilds(&self.access_token).await {
                        Ok(map) => self.guild_names = map,
                        Err(e) => warn!("[discord-rpc] Could not load guild names: {}", e),
                    }
                }
                self.guild_names.get(guild_id).cloned()
            } else {
                conn.request("GET_GUILD", json!({ "guild_id": guild_id }), None)
                    .await
                    .ok()
                    .and_then(|d| str_field(&d, "name").map(String::from))
            };
            self.last_guild = Some((guild_id.to_string(), name.clone()));
            name
        }

        /// Apply a GET_SELECTED_VOICE_CHANNEL response (channel object or null).
        async fn apply(&mut self, conn: &mut Conn, channel: &Value) -> Result<(), String> {
            let new_id = str_field(channel, "id").map(String::from);
            let old_id = self.current_channel_id.clone();

            let Some(new_id) = new_id else {
                if let Some(old) = old_id {
                    if let Err(e) = conn.set_speaking_subscription("UNSUBSCRIBE", &old).await {
                        warn!("[discord-rpc] {}", e);
                    }
                    clear_channel_info();
                    self.current_channel_id = None;
                    info!("[discord-rpc] User left voice channel");
                }
                return Ok(());
            };

            let changed = old_id.as_deref() != Some(new_id.as_str());
            if changed {
                if let Some(old) = &old_id {
                    if let Err(e) = conn.set_speaking_subscription("UNSUBSCRIBE", old).await {
                        warn!("[discord-rpc] {}", e);
                    }
                }
                conn.set_speaking_subscription("SUBSCRIBE", &new_id).await?;
                self.current_channel_id = Some(new_id.clone());
            }

            let guild_id = str_field(channel, "guild_id").map(String::from);
            let guild_name = match &guild_id {
                Some(gid) => self.guild_name(conn, gid).await,
                None => None,
            };
            let channel_name = str_field(channel, "name").map(String::from);
            if changed {
                info!(
                    "[discord-rpc] Voice channel: {} / {} ({})",
                    guild_name.as_deref().unwrap_or("?"),
                    channel_name.as_deref().unwrap_or("?"),
                    new_id
                );
            }
            set_channel_info(ChannelInfo {
                channel_id: new_id,
                channel_name,
                channel_type: channel.get("type").and_then(|v| v.as_u64()).map(|n| n as u8),
                guild_id,
                guild_name,
                self_user_id: self.self_user_id.clone(),
                user_labels: user_labels(channel, self.self_user_id.as_deref()),
            });
            Ok(())
        }

        async fn poll_selected_channel(&mut self, conn: &mut Conn) -> Result<(), String> {
            let channel = conn
                .request("GET_SELECTED_VOICE_CHANNEL", json!({}), None)
                .await
                .map_err(|e| format!("GET_SELECTED_VOICE_CHANNEL failed: {}", String::from(e)))?;
            self.apply(conn, &channel).await
        }

        async fn run_loop(&mut self, conn: &mut Conn) -> Result<(), String> {
            let mut interval = tokio::time::interval(CHANNEL_POLL_INTERVAL);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            interval.tick().await; // first tick fires immediately
            loop {
                tokio::select! {
                    frame = conn.frames.recv() => {
                        let Some(frame) = frame else { return Ok(()) };
                        if let Some(p) = conn.process_frame(frame).await? {
                            conn.dispatch_event(&p)?;
                        }
                    }
                    _ = interval.tick() => {
                        self.poll_selected_channel(conn).await?;
                    }
                }
            }
        }
    }

    async fn authorize(conn: &mut Conn, client_id: &str) -> Result<TokenSet, String> {
        info!("[discord-rpc] Sending AUTHORIZE (approve in Discord popup)...");
        let pkce = Pkce::generate();
        let data = conn
            .request(
                "AUTHORIZE",
                json!({
                    "client_id": client_id,
                    "scopes": OAUTH_SCOPES,
                    "code_challenge": pkce.challenge,
                    "code_challenge_method": "S256",
                }),
                None,
            )
            .await
            .map_err(authorize_error)?;
        let code = str_field(&data, "code").ok_or_else(|| {
            "Discord did not return an authorization code. Approve the request in the Discord popup."
                .to_string()
        })?;
        debug!("[discord-rpc] Got auth code, exchanging for token...");
        exchange_code(client_id, code, &pkce.verifier).await
    }

    /// READY, authorize (unless an access token is given), AUTHENTICATE, read the selected
    /// voice channel and subscribe to speaking events.
    async fn setup(
        conn: &mut Conn,
        client_id: &str,
        access_token: Option<String>,
    ) -> Result<(ChannelTracker, Option<String>), String> {
        info!("[discord-rpc] Waiting for READY...");
        conn.wait_ready().await?;

        let (access_token, refresh_token) = match access_token {
            Some(token) => (token, None),
            None => {
                let tokens = authorize(conn, client_id).await?;
                (tokens.access_token, tokens.refresh_token)
            }
        };

        let auth = conn
            .request(
                "AUTHENTICATE",
                json!({ "access_token": access_token.as_str() }),
                None,
            )
            .await
            .map_err(|e| format!("Discord rejected the access token: {}", String::from(e)))?;
        let self_user_id = auth
            .get("user")
            .and_then(|u| str_field(u, "id"))
            .map(String::from);

        info!("[discord-rpc] Getting voice channel...");
        let channel = conn
            .request("GET_SELECTED_VOICE_CHANNEL", json!({}), None)
            .await
            .map_err(|e| format!("Could not read the voice channel: {}", String::from(e)))?;
        if str_field(&channel, "id").is_none() {
            return Err(
                "Not in a voice channel. Join a voice channel in Discord first, then click Connect."
                    .into(),
            );
        }

        let mut tracker = ChannelTracker {
            self_user_id,
            access_token,
            current_channel_id: None,
            guild_names: HashMap::new(),
            last_guild: None,
        };
        tracker.apply(conn, &channel).await?;
        Ok((tracker, refresh_token))
    }

    pub(super) async fn run(
        mut conn: Conn,
        client_id: String,
        access_token: Option<String>,
        ready_tx: tokio::sync::oneshot::Sender<Result<Option<String>, String>>,
    ) {
        let mut tracker = match setup(&mut conn, &client_id, access_token).await {
            Ok((tracker, refresh_token)) => {
                info!("[discord-rpc] Connected");
                set_rpc_connected(true);
                let _ = ready_tx.send(Ok(refresh_token));
                tracker
            }
            Err(e) => {
                error!("[discord-rpc] Connect failed: {}", e);
                let _ = ready_tx.send(Err(e));
                return;
            }
        };

        match tracker.run_loop(&mut conn).await {
            Ok(()) => info!("[discord-rpc] Discord closed the connection"),
            Err(e) => error!("[discord-rpc] Connection error: {}", e),
        }
        set_rpc_connected(false);
        clear_channel_info();
    }
}
