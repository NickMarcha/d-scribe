//! A stand-in for the Discord desktop app, for testing d-scribe without Discord.
//!
//! It speaks Discord's local RPC protocol (the IPC framing, READY, AUTHORIZE, AUTHENTICATE,
//! GET_SELECTED_VOICE_CHANNEL, GET_GUILD, SUBSCRIBE/UNSUBSCRIBE to SPEAKING_START/STOP, PING)
//! and the two HTTP endpoints d-scribe calls (`/oauth2/token` and `/v10/users/@me/guilds`), and
//! plays a scenario: a call with people joining, leaving and talking.
//!
//! The framing is written here again rather than shared with d-scribe on purpose, so a bug in
//! d-scribe's version can't hide in both.
//!
//! Used by d-scribe's Rust tests (on an in-memory stream) and by the `fake-discord` tool (on a
//! named pipe or Unix socket, for running the real app against it).

pub mod scenario;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use scenario::{Action, Channel, Member, Scenario};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// What d-scribe must send as `redirect_uri` in the code exchange.
pub const REDIRECT_URI: &str = "https://localhost";

// Discord's RPC error codes for the cases we fake.
const ERR_UNKNOWN_COMMAND: i64 = 4000;
const ERR_INVALID_GUILD: i64 = 4003;
const ERR_NOT_AUTHENTICATED: i64 = 4006;
const ERR_INVALID_TOKEN: i64 = 4009;
const ERR_OAUTH2: i64 = 5000;

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, opcode: u32, text: &str) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(8 + text.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(text.len() as u32).to_le_bytes());
    frame.extend_from_slice(text.as_bytes());
    w.write_all(&frame).await?;
    w.flush().await
}

/// Next frame, or None when the other side closed the connection.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<(u32, String)>> {
    let mut header = [0u8; 8];
    match r.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof || e.kind() == std::io::ErrorKind::BrokenPipe => {
            return Ok(None)
        }
        Err(e) => return Err(e),
    }
    let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    let text = String::from_utf8(body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some((opcode, text)))
}

#[derive(Clone, Debug)]
enum Event {
    Speaking { user_id: String, channel_id: String, start: bool },
    Close,
}

/// Plays a speaker's audio. Gets a WAV path, or None for "make a sound for this long".
pub type Player = Box<dyn Fn(&str, Option<&Path>, Duration) + Send + Sync>;

struct World {
    scenario: Scenario,
    channel: Option<Channel>,
    /// Authorization code -> PKCE challenge.
    codes: HashMap<String, String>,
    access_tokens: HashSet<String>,
    used_refresh_tokens: HashSet<String>,
    issued: u64,
}

/// The fake Discord app. Share it with `Arc`; every connection and the HTTP server see the
/// same call.
pub struct FakeDiscord {
    world: Mutex<World>,
    events: broadcast::Sender<Event>,
    /// Commands received, as "CMD" or "CMD EVT", for tests and the console.
    commands: Mutex<Vec<String>>,
    timeline_started: AtomicBool,
    started: Instant,
    player: Mutex<Option<Player>>,
    /// Folder that relative WAV paths in the scenario are resolved against.
    base_dir: PathBuf,
    quiet: bool,
}

impl FakeDiscord {
    pub fn new(scenario: Scenario) -> Arc<Self> {
        Self::with_options(scenario, PathBuf::from("."), true)
    }

    /// `base_dir` resolves relative WAV paths; `quiet: false` prints what happens.
    pub fn with_options(scenario: Scenario, base_dir: PathBuf, quiet: bool) -> Arc<Self> {
        Arc::new(Self {
            world: Mutex::new(World {
                scenario,
                channel: None,
                codes: HashMap::new(),
                access_tokens: HashSet::new(),
                used_refresh_tokens: HashSet::new(),
                issued: 0,
            }),
            events: broadcast::channel(256).0,
            commands: Mutex::new(Vec::new()),
            timeline_started: AtomicBool::new(false),
            started: Instant::now(),
            player: Mutex::new(None),
            base_dir,
            quiet,
        })
    }

    pub fn set_player(&self, player: Player) {
        *self.player.lock().unwrap() = Some(player);
    }

    pub fn say(&self, text: impl AsRef<str>) {
        if !self.quiet {
            println!("[{:>7.1}s] {}", self.started.elapsed().as_secs_f64(), text.as_ref());
        }
    }

    /// Commands received so far, as "CMD" or "CMD EVT".
    pub fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    pub fn set_deny_authorize(&self, on: bool) {
        self.world.lock().unwrap().scenario.deny_authorize = on;
    }

    pub fn set_reject_tokens(&self, on: bool) {
        self.world.lock().unwrap().scenario.reject_tokens = on;
    }

    /// The channel and who's in it, for the console.
    pub fn status(&self) -> String {
        let w = self.world.lock().unwrap();
        match &w.channel {
            None => "Not in a voice channel".into(),
            Some(c) => {
                let names: Vec<String> = c.members.iter().map(|m| format!("{} ({})", label(m), m.id)).collect();
                format!(
                    "In {} ({}), type {}, with {}",
                    c.name.as_deref().unwrap_or("a call"),
                    c.id,
                    c.kind,
                    if names.is_empty() { "nobody else".into() } else { names.join(", ") }
                )
            }
        }
    }

    /// The first channel the scenario joins, for the console's `join`.
    pub fn scenario_channel(&self) -> Option<Channel> {
        self.world.lock().unwrap().scenario.timeline.iter().find_map(|s| match &s.action {
            Action::Join { channel } => Some(channel.clone()),
            _ => None,
        })
    }

    /// Does one step now. `Speak` returns at once and stops speaking later.
    pub fn apply(self: &Arc<Self>, action: Action) {
        match action {
            Action::Join { channel } => {
                self.say(format!("You join {} ({})", channel.name.as_deref().unwrap_or("a call"), channel.id));
                self.world.lock().unwrap().channel = Some(channel);
            }
            Action::Leave => {
                self.say("You leave the voice channel");
                self.world.lock().unwrap().channel = None;
            }
            Action::AddMember { member } => {
                let mut w = self.world.lock().unwrap();
                match w.channel.as_mut() {
                    Some(c) => {
                        c.members.retain(|m| m.id != member.id);
                        self.say(format!("{} joins", label(&member)));
                        c.members.push(member);
                    }
                    None => self.say("Can't add someone: you're not in a voice channel"),
                }
            }
            Action::RemoveMember { id } => {
                let mut w = self.world.lock().unwrap();
                if let Some(c) = w.channel.as_mut() {
                    c.members.retain(|m| m.id != id);
                    self.say(format!("{} leaves", id));
                }
            }
            Action::Speak { user, seconds, wav } => {
                let Some(channel_id) = self.world.lock().unwrap().channel.as_ref().map(|c| c.id.clone()) else {
                    self.say(format!("{} can't speak: you're not in a voice channel", user));
                    return;
                };
                let wav = wav.map(|p| if p.is_absolute() { p } else { self.base_dir.join(p) });
                let length = match (&wav, seconds) {
                    (_, Some(s)) => Duration::from_secs_f64(s),
                    (Some(p), None) => wav_length(p).unwrap_or_else(|e| {
                        self.say(format!("Can't read {}: {}", p.display(), e));
                        Duration::from_secs(2)
                    }),
                    (None, None) => Duration::from_secs(2),
                };
                let fake = self.clone();
                tokio::spawn(async move {
                    fake.say(format!("{} speaks for {:.1}s", user, length.as_secs_f64()));
                    if let Some(play) = fake.player.lock().unwrap().as_ref() {
                        play(&user, wav.as_deref(), length);
                    }
                    let _ = fake.events.send(Event::Speaking {
                        user_id: user.clone(),
                        channel_id: channel_id.clone(),
                        start: true,
                    });
                    tokio::time::sleep(length).await;
                    let _ = fake.events.send(Event::Speaking { user_id: user, channel_id, start: false });
                });
            }
            Action::Close => {
                self.say("Discord closes all connections");
                let _ = self.events.send(Event::Close);
            }
        }
    }

    /// Runs the scenario's timeline. Starts by itself on the first successful AUTHENTICATE.
    pub async fn run_timeline(self: Arc<Self>) {
        let steps = self.world.lock().unwrap().scenario.timeline.clone();
        if steps.is_empty() {
            return;
        }
        self.say(format!("Timeline starts ({} steps)", steps.len()));
        let start = tokio::time::Instant::now();
        for step in steps {
            tokio::time::sleep_until(start + Duration::from_secs_f64(step.at.max(0.0))).await;
            self.apply(step.action);
        }
        self.say("Timeline done");
    }

    pub fn restart_timeline(self: &Arc<Self>) {
        tokio::spawn(self.clone().run_timeline());
    }

    fn channel_json(&self, w: &World) -> Value {
        let Some(c) = &w.channel else { return Value::Null };
        let you = Member { id: w.scenario.user.id.clone(), username: w.scenario.user.username.clone(), nick: None };
        let voice_states: Vec<Value> = std::iter::once(&you)
            .chain(c.members.iter())
            .map(|m| {
                json!({
                    "nick": label(m),
                    "mute": false,
                    "volume": 100,
                    "pan": { "left": 1.0, "right": 1.0 },
                    "voice_state": { "mute": false, "deaf": false, "self_mute": false, "self_deaf": false, "suppress": false },
                    "user": { "id": m.id, "username": m.username, "discriminator": "0", "global_name": m.nick, "avatar": null, "bot": false },
                })
            })
            .collect();
        json!({
            "id": c.id,
            "name": c.name.clone().unwrap_or_default(),
            "type": c.kind,
            "topic": "",
            "bitrate": 64000,
            "user_limit": 0,
            "guild_id": c.guild_id,
            "position": 0,
            "messages": [],
            "voice_states": voice_states,
        })
    }

    /// One command on one connection. Returns the response payload.
    fn handle(self: &Arc<Self>, conn: &mut ConnState, req: &Value) -> Value {
        let cmd = req["cmd"].as_str().unwrap_or("").to_string();
        let evt = req["evt"].as_str().map(String::from);
        let args = &req["args"];
        self.commands.lock().unwrap().push(match &evt {
            Some(e) => format!("{} {}", cmd, e),
            None => cmd.clone(),
        });
        self.say(format!("<- {}{}", cmd, evt.as_deref().map(|e| format!(" {}", e)).unwrap_or_default()));

        let result: Result<Value, (i64, String)> = (|| match cmd.as_str() {
            "AUTHORIZE" => {
                let mut w = self.world.lock().unwrap();
                if w.scenario.deny_authorize {
                    return Err((ERR_OAUTH2, "OAuth2 Error: access_denied: The resource owner or authorization server denied the request".into()));
                }
                if args["client_id"].as_str().unwrap_or("").is_empty() {
                    return Err((ERR_OAUTH2, "OAuth2 Error: invalid_request: client_id is required".into()));
                }
                let scopes: Vec<&str> = args["scopes"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).collect();
                if scopes.iter().any(|s| !["rpc", "identify"].contains(s)) {
                    return Err((ERR_OAUTH2, format!("OAuth2 Error: invalid_scope: {:?}", scopes)));
                }
                if args["code_challenge_method"] != "S256" || args["code_challenge"].as_str().is_none() {
                    return Err((ERR_OAUTH2, "OAuth2 Error: invalid_request: PKCE with S256 is required".into()));
                }
                w.issued += 1;
                let code = format!("fake-code-{}", w.issued);
                w.codes.insert(code.clone(), args["code_challenge"].as_str().unwrap().to_string());
                Ok(json!({ "code": code }))
            }
            "AUTHENTICATE" => {
                let w = self.world.lock().unwrap();
                let token = args["access_token"].as_str().unwrap_or("");
                if w.scenario.reject_tokens || !w.access_tokens.contains(token) {
                    return Err((ERR_INVALID_TOKEN, "Invalid access token".into()));
                }
                conn.authenticated = true;
                Ok(json!({
                    "user": { "id": w.scenario.user.id, "username": w.scenario.user.username, "discriminator": "0", "avatar": null },
                    "scopes": ["rpc", "identify"],
                    "expires": "2099-01-01T00:00:00.000Z",
                    "application": { "id": conn.client_id, "name": "d-scribe", "description": "", "icon": null, "rpc_origins": [] },
                    "access_token": token,
                }))
            }
            _ if !conn.authenticated => Err((ERR_NOT_AUTHENTICATED, "Not authenticated or invalid scope".into())),
            "GET_SELECTED_VOICE_CHANNEL" => Ok(self.channel_json(&self.world.lock().unwrap())),
            "GET_GUILD" => {
                let id = args["guild_id"].as_str().unwrap_or("");
                match self.world.lock().unwrap().scenario.guilds.get(id) {
                    Some(name) => Ok(json!({ "id": id, "name": name, "icon_url": null, "members": [] })),
                    None => Err((ERR_INVALID_GUILD, format!("Invalid guild id: {}", id))),
                }
            }
            "SUBSCRIBE" | "UNSUBSCRIBE" => {
                let evt = evt.clone().unwrap_or_default();
                if evt != "SPEAKING_START" && evt != "SPEAKING_STOP" {
                    return Err((ERR_UNKNOWN_COMMAND, format!("Unknown event: {}", evt)));
                }
                let channel = args["channel_id"].as_str().unwrap_or("").to_string();
                if cmd == "SUBSCRIBE" {
                    conn.subscriptions.insert((evt.clone(), channel));
                } else {
                    conn.subscriptions.remove(&(evt.clone(), channel));
                }
                Ok(json!({ "evt": evt }))
            }
            other => Err((ERR_UNKNOWN_COMMAND, format!("Unknown command: {}", other))),
        })();

        if cmd == "AUTHENTICATE" && result.is_ok() && !self.timeline_started.swap(true, Ordering::SeqCst) {
            self.restart_timeline();
        }
        match result {
            Ok(data) => json!({ "cmd": cmd, "nonce": req["nonce"], "evt": null, "data": data }),
            Err((code, message)) => {
                self.say(format!("-> ERROR {}: {}", code, message));
                json!({ "cmd": cmd, "nonce": req["nonce"], "evt": "ERROR", "data": { "code": code, "message": message } })
            }
        }
    }

    /// Serves one client connection until it closes.
    pub async fn serve<S>(self: Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        match self.serve_inner(stream).await {
            Ok(()) => self.say("Connection closed"),
            Err(e) => self.say(format!("Connection ended: {}", e)),
        }
    }

    async fn serve_inner<S>(self: &Arc<Self>, stream: S) -> Result<(), String>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (opcode, hello) = read_frame(&mut reader)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("closed before the handshake")?;
        let hello: Value = serde_json::from_str(&hello).map_err(|e| format!("bad handshake: {}", e))?;
        let client_id = hello["client_id"].as_str().unwrap_or("").to_string();
        if opcode != OP_HANDSHAKE || hello["v"] != 1 || client_id.is_empty() {
            let close = json!({ "code": 4000, "message": "Invalid handshake" }).to_string();
            let _ = write_frame(&mut writer, OP_CLOSE, &close).await;
            return Err(format!("invalid handshake: opcode {} {}", opcode, hello));
        }
        self.say(format!("Connected (client id {})", client_id));

        let mut events = self.events.subscribe();
        let (frame_tx, mut frames) = mpsc::unbounded_channel();
        let reader_task = tokio::spawn(async move {
            while let Ok(Some(frame)) = read_frame(&mut reader).await {
                if frame_tx.send(frame).is_err() {
                    break;
                }
            }
        });
        let _abort_reader = AbortOnDrop(reader_task);

        let user = self.world.lock().unwrap().scenario.user.clone();
        let ready = json!({
            "cmd": "DISPATCH", "evt": "READY", "nonce": null,
            "data": { "v": 1, "config": { "cdn_host": "cdn.discordapp.com", "api_endpoint": "//discord.com/api", "environment": "production" }, "user": user },
        });
        write_frame(&mut writer, OP_FRAME, &ready.to_string()).await.map_err(|e| e.to_string())?;

        let mut conn = ConnState { client_id, authenticated: false, subscriptions: HashSet::new() };
        loop {
            tokio::select! {
                frame = frames.recv() => {
                    let Some((opcode, text)) = frame else { return Ok(()) };
                    match opcode {
                        OP_FRAME => {
                            let req: Value = serde_json::from_str(&text).map_err(|e| format!("bad frame: {}", e))?;
                            let response = self.handle(&mut conn, &req);
                            write_frame(&mut writer, OP_FRAME, &response.to_string()).await.map_err(|e| e.to_string())?;
                        }
                        OP_PING => write_frame(&mut writer, OP_PONG, &text).await.map_err(|e| e.to_string())?,
                        OP_CLOSE => return Ok(()),
                        _ => {}
                    }
                }
                event = events.recv() => match event {
                    Ok(Event::Speaking { user_id, channel_id, start }) => {
                        let evt = if start { "SPEAKING_START" } else { "SPEAKING_STOP" };
                        if conn.subscriptions.contains(&(evt.to_string(), channel_id.clone())) {
                            let payload = json!({ "cmd": "DISPATCH", "evt": evt, "nonce": null, "data": { "user_id": user_id, "channel_id": channel_id } });
                            write_frame(&mut writer, OP_FRAME, &payload.to_string()).await.map_err(|e| e.to_string())?;
                        }
                    }
                    Ok(Event::Close) => {
                        let close = json!({ "code": 1000, "message": "Discord is closing" }).to_string();
                        let _ = write_frame(&mut writer, OP_CLOSE, &close).await;
                        return Ok(());
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }

    /// Discord's token endpoint (`POST /oauth2/token`, form-encoded). Returns (status, body).
    pub fn token(&self, form: &str) -> (u16, Value) {
        let form = parse_form(form);
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let mut w = self.world.lock().unwrap();
        let invalid_grant = |why: &str| (400, json!({ "error": "invalid_grant", "error_description": why }));
        if get("client_id").is_empty() {
            return (401, json!({ "error": "invalid_client" }));
        }
        match get("grant_type") {
            "authorization_code" => {
                if get("redirect_uri") != REDIRECT_URI {
                    return (400, json!({ "error": "invalid_request", "error_description": "Invalid \"redirect_uri\" in request." }));
                }
                let Some(challenge) = w.codes.remove(get("code")) else {
                    return invalid_grant("Invalid \"code\" in request.");
                };
                let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
                if expected != challenge {
                    return invalid_grant("Invalid \"code_verifier\" in request.");
                }
            }
            "refresh_token" => {
                let refresh = get("refresh_token");
                if w.scenario.reject_tokens || !refresh.starts_with("fake-refresh-") {
                    return invalid_grant("Invalid \"refresh_token\" in request.");
                }
                // Discord rotates refresh tokens: each one works once.
                if !w.used_refresh_tokens.insert(refresh.to_string()) {
                    return invalid_grant("Invalid \"refresh_token\" in request.");
                }
            }
            _ => return (400, json!({ "error": "unsupported_grant_type" })),
        }
        w.issued += 1;
        let n = w.issued;
        let access = format!("fake-access-{}", n);
        w.access_tokens.insert(access.clone());
        self.say(format!("-> token {} for {}", access, get("grant_type")));
        (
            200,
            json!({
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": 604800,
                "refresh_token": format!("fake-refresh-{}", n),
                "scope": "rpc identify",
            }),
        )
    }

    /// `GET /v10/users/@me/guilds` with `Authorization: Bearer <token>`.
    pub fn user_guilds(&self, authorization: &str) -> (u16, Value) {
        let w = self.world.lock().unwrap();
        let token = authorization.strip_prefix("Bearer ").unwrap_or("");
        if !w.access_tokens.contains(token) {
            return (401, json!({ "message": "401: Unauthorized", "code": 0 }));
        }
        let guilds: Vec<Value> = w.scenario.guilds.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        (200, json!(guilds))
    }

    /// Serves the HTTP endpoints on `addr` (e.g. "127.0.0.1:6464", or port 0 for any) in a
    /// background thread. Returns the address it listens on.
    pub fn serve_http(self: &Arc<Self>, addr: &str) -> Result<String, String> {
        let server = tiny_http::Server::http(addr).map_err(|e| format!("Could not listen on {}: {}", addr, e))?;
        let bound = server.server_addr().to_ip().map(|a| a.to_string()).unwrap_or_else(|| addr.to_string());
        let fake = self.clone();
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let auth = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str().to_string())
                    .unwrap_or_default();
                let path = request.url().split('?').next().unwrap_or("").to_string();
                let (status, json) = match (request.method(), path.as_str()) {
                    (tiny_http::Method::Post, "/oauth2/token") => fake.token(&body),
                    (tiny_http::Method::Get, "/v10/users/@me/guilds") => fake.user_guilds(&auth),
                    _ => (404, json!({ "message": "404: Not Found", "code": 0 })),
                };
                let header = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let response = tiny_http::Response::from_string(json.to_string()).with_status_code(status).with_header(header);
                let _ = request.respond(response);
            }
        });
        Ok(bound)
    }
}

struct ConnState {
    client_id: String,
    authenticated: bool,
    /// (event, channel id)
    subscriptions: HashSet<(String, String)>,
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn label(m: &Member) -> &str {
    m.nick.as_deref().unwrap_or(&m.username)
}

fn wav_length(path: &Path) -> Result<Duration, String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    Ok(Duration::from_secs_f64(reader.duration() as f64 / spec.sample_rate as f64))
}

fn parse_form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (url_decode(k), url_decode(v)))
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
