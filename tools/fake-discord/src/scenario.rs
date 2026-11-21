//! Scenario files: who you are, which servers exist, how Discord behaves, and a timeline of
//! what happens in the call. See `scenarios/` for examples.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// The logged-in user (you).
    #[serde(default = "default_user")]
    pub user: User,
    /// Guild id -> name, answered by GET_GUILD and the guilds REST endpoint.
    #[serde(default)]
    pub guilds: BTreeMap<String, String>,
    /// AUTHORIZE fails as if you clicked Cancel in Discord's popup.
    #[serde(default)]
    pub deny_authorize: bool,
    /// Access and refresh tokens are refused, as when a login expired or was revoked.
    #[serde(default)]
    pub reject_tokens: bool,
    /// Steps, timed from the first successful AUTHENTICATE.
    #[serde(default)]
    pub timeline: Vec<Step>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            user: default_user(),
            guilds: BTreeMap::new(),
            deny_authorize: false,
            reject_tokens: false,
            timeline: Vec::new(),
        }
    }
}

fn default_user() -> User {
    User { id: "100".into(), username: "you".into() }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Member {
    pub id: String,
    pub username: String,
    /// Server nickname; d-scribe shows it instead of the username.
    #[serde(default)]
    pub nick: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Absent for DMs and group DMs.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// Discord channel type: 1 = DM, 2 = server voice channel, 3 = group DM.
    #[serde(rename = "type", default = "voice_type")]
    pub kind: u8,
    /// Everyone in the call except you (you're added automatically).
    #[serde(default)]
    pub members: Vec<Member>,
}

fn voice_type() -> u8 {
    2
}

#[derive(Clone, Debug, Deserialize)]
pub struct Step {
    /// Seconds after the timeline starts.
    pub at: f64,
    #[serde(flatten)]
    pub action: Action,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// You join (or switch to) a voice channel.
    Join { channel: Channel },
    /// You leave the voice channel.
    Leave,
    /// Someone joins your channel.
    AddMember { member: Member },
    /// Someone leaves your channel.
    RemoveMember { id: String },
    /// Someone talks: SPEAKING_START, then SPEAKING_STOP after `seconds`, or after the length of
    /// `wav` (played on the default output device). Neither: 2 seconds.
    Speak {
        user: String,
        #[serde(default)]
        seconds: Option<f64>,
        #[serde(default)]
        wav: Option<PathBuf>,
    },
    /// Discord closes every connection (as when it quits or restarts for an update).
    Close,
}
