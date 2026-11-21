//! The fake behaves like Discord on the rules d-scribe depends on.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fake_discord::scenario::{Action, Channel, Member, Scenario};
use fake_discord::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::DuplexStream;

struct Client {
    io: DuplexStream,
    nonce: u32,
}

impl Client {
    async fn connect(fake: &std::sync::Arc<FakeDiscord>) -> (Self, Value) {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        tokio::spawn(fake.clone().serve(theirs));
        let mut c = Client { io: ours, nonce: 0 };
        write_frame(&mut c.io, OP_HANDSHAKE, r#"{"v":1,"client_id":"42"}"#).await.unwrap();
        let ready = c.next().await;
        (c, ready)
    }

    async fn next(&mut self) -> Value {
        let (op, text) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.io))
            .await
            .expect("no frame within 5 s")
            .unwrap()
            .expect("connection closed");
        assert_eq!(op, OP_FRAME, "{}", text);
        serde_json::from_str(&text).unwrap()
    }

    async fn request(&mut self, cmd: &str, args: Value, evt: Option<&str>) -> Value {
        self.nonce += 1;
        let req = json!({ "cmd": cmd, "args": args, "evt": evt, "nonce": self.nonce.to_string() });
        write_frame(&mut self.io, OP_FRAME, &req.to_string()).await.unwrap();
        let resp = self.next().await;
        assert_eq!(resp["nonce"], self.nonce.to_string());
        resp
    }
}

fn pkce() -> (String, String) {
    let verifier = "a-verifier-that-is-long-enough-for-pkce-0123456789".to_string();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", k, v.replace(':', "%3A").replace('/', "%2F"))).collect::<Vec<_>>().join("&")
}

async fn login(fake: &std::sync::Arc<FakeDiscord>, c: &mut Client) -> (String, String) {
    let (verifier, challenge) = pkce();
    let auth = c
        .request("AUTHORIZE", json!({ "client_id": "42", "scopes": ["rpc", "identify"], "code_challenge": challenge, "code_challenge_method": "S256" }), None)
        .await;
    let code = auth["data"]["code"].as_str().unwrap().to_string();
    let (status, tokens) = fake.token(&form(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("client_id", "42"),
        ("code_verifier", &verifier),
        ("redirect_uri", REDIRECT_URI),
    ]));
    assert_eq!(status, 200, "{}", tokens);
    let access = tokens["access_token"].as_str().unwrap().to_string();
    let resp = c.request("AUTHENTICATE", json!({ "access_token": access }), None).await;
    assert_eq!(resp["evt"], Value::Null, "{}", resp);
    (access, tokens["refresh_token"].as_str().unwrap().to_string())
}

fn channel() -> Channel {
    Channel {
        id: "301".into(),
        name: Some("General".into()),
        guild_id: Some("300".into()),
        kind: 2,
        members: vec![Member { id: "201".into(), username: "alice".into(), nick: Some("Alice".into()) }],
    }
}

#[tokio::test]
async fn sends_ready_after_the_handshake() {
    let fake = FakeDiscord::new(Scenario::default());
    let (_c, ready) = Client::connect(&fake).await;
    assert_eq!(ready["evt"], "READY");
    assert_eq!(ready["data"]["user"]["id"], "100");
}

#[tokio::test]
async fn commands_need_authentication() {
    let fake = FakeDiscord::new(Scenario::default());
    let (mut c, _) = Client::connect(&fake).await;
    let resp = c.request("GET_SELECTED_VOICE_CHANNEL", json!({}), None).await;
    assert_eq!(resp["evt"], "ERROR");
    assert_eq!(resp["data"]["code"], 4006);
    let resp = c.request("AUTHENTICATE", json!({ "access_token": "made-up" }), None).await;
    assert_eq!(resp["data"]["code"], 4009);
}

#[tokio::test]
async fn oauth_checks_pkce_and_codes_work_once() {
    let fake = FakeDiscord::new(Scenario::default());
    let (mut c, _) = Client::connect(&fake).await;
    let (_, challenge) = pkce();
    let auth = c
        .request("AUTHORIZE", json!({ "client_id": "42", "scopes": ["rpc", "identify"], "code_challenge": challenge, "code_challenge_method": "S256" }), None)
        .await;
    let code = auth["data"]["code"].as_str().unwrap();
    let exchange = |verifier: &str, redirect: &str| {
        fake.token(&form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", "42"),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
        ]))
    };
    assert_eq!(exchange(&pkce().0, "https://example.com").0, 400, "wrong redirect_uri");
    assert_eq!(exchange("wrong-verifier", REDIRECT_URI).0, 400, "wrong verifier");
    // The failed verifier used up the code, as Discord does.
    assert_eq!(exchange(&pkce().0, REDIRECT_URI).0, 400);
}

#[tokio::test]
async fn refresh_tokens_rotate() {
    let fake = FakeDiscord::new(Scenario::default());
    let (mut c, _) = Client::connect(&fake).await;
    let (_, refresh) = login(&fake, &mut c).await;
    let refresh_with = |t: &str| fake.token(&form(&[("grant_type", "refresh_token"), ("refresh_token", t), ("client_id", "42")]));
    let (status, tokens) = refresh_with(&refresh);
    assert_eq!(status, 200);
    assert_ne!(tokens["refresh_token"], refresh);
    assert_eq!(refresh_with(&refresh).0, 400, "an old refresh token is refused");
    fake.set_reject_tokens(true);
    assert_eq!(refresh_with(tokens["refresh_token"].as_str().unwrap()).0, 400);
}

#[tokio::test]
async fn denied_authorize_reads_as_cancelled() {
    let fake = FakeDiscord::new(Scenario { deny_authorize: true, ..Scenario::default() });
    let (mut c, _) = Client::connect(&fake).await;
    let resp = c
        .request("AUTHORIZE", json!({ "client_id": "42", "scopes": ["rpc"], "code_challenge": "x", "code_challenge_method": "S256" }), None)
        .await;
    assert_eq!(resp["evt"], "ERROR");
    assert!(resp["data"]["message"].as_str().unwrap().contains("denied"));
}

#[tokio::test]
async fn channel_includes_you_and_speaking_needs_a_subscription() {
    let fake = FakeDiscord::new(Scenario { guilds: [("300".to_string(), "Fake server".to_string())].into(), ..Scenario::default() });
    let (mut c, _) = Client::connect(&fake).await;
    login(&fake, &mut c).await;

    assert_eq!(c.request("GET_SELECTED_VOICE_CHANNEL", json!({}), None).await["data"], Value::Null);
    fake.apply(Action::Join { channel: channel() });
    let ch = c.request("GET_SELECTED_VOICE_CHANNEL", json!({}), None).await["data"].clone();
    let ids: Vec<&str> = ch["voice_states"].as_array().unwrap().iter().map(|v| v["user"]["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["100", "201"]);
    assert_eq!(ch["voice_states"][1]["nick"], "Alice");
    assert_eq!(c.request("GET_GUILD", json!({ "guild_id": "300" }), None).await["data"]["name"], "Fake server");

    // Not subscribed yet: nothing arrives. Then subscribed: start and stop arrive in order.
    fake.apply(Action::Speak { user: "201".into(), seconds: Some(0.05), wav: None });
    tokio::time::sleep(Duration::from_millis(150)).await;
    c.request("SUBSCRIBE", json!({ "channel_id": "301" }), Some("SPEAKING_START")).await;
    c.request("SUBSCRIBE", json!({ "channel_id": "301" }), Some("SPEAKING_STOP")).await;
    fake.apply(Action::Speak { user: "201".into(), seconds: Some(0.05), wav: None });
    let start = c.next().await;
    assert_eq!((start["evt"].as_str(), start["data"]["user_id"].as_str()), (Some("SPEAKING_START"), Some("201")));
    assert_eq!(c.next().await["evt"], "SPEAKING_STOP");

    assert_eq!(
        fake.commands(),
        [
            "AUTHORIZE",
            "AUTHENTICATE",
            "GET_SELECTED_VOICE_CHANNEL",
            "GET_SELECTED_VOICE_CHANNEL",
            "GET_GUILD",
            "SUBSCRIBE SPEAKING_START",
            "SUBSCRIBE SPEAKING_STOP"
        ]
    );
}

#[tokio::test]
async fn close_ends_every_connection() {
    let fake = FakeDiscord::new(Scenario::default());
    let (mut c, _) = Client::connect(&fake).await;
    fake.apply(Action::Close);
    let (op, _) = read_frame(&mut c.io).await.unwrap().unwrap();
    assert_eq!(op, OP_CLOSE);
    assert!(read_frame(&mut c.io).await.unwrap().is_none());
}

#[test]
fn example_scenarios_parse() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let s: Scenario = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        assert!(!s.timeline.is_empty(), "{}", path.display());
        n += 1;
    }
    assert!(n >= 3);
}
