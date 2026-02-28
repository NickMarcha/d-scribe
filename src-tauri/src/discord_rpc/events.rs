//! Discord RPC event types.

use serde::Deserialize;

/// Voice channel information from Discord RPC.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct VoiceChannel {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "guild_id")]
    pub guild_id: Option<String>,
    #[serde(rename = "voice_states")]
    pub voice_states: Option<Vec<VoiceState>>,
}

/// Voice state for a user in a voice channel.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct VoiceState {
    pub user: Option<VoiceStateUser>,
    pub nick: Option<String>,
}

/// User info in a voice state.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct VoiceStateUser {
    pub id: String,
    pub username: Option<String>,
}

/// Speaking event - either start or stop.
#[derive(Debug, Clone)]
pub enum SpeakingEvent {
    Start { user_id: String },
    Stop { user_id: String },
}

/// Authenticated user info from AUTHENTICATE response.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct AuthenticatedUser {
    pub id: Option<String>,
    pub username: Option<String>,
}

/// Channel info from GET_SELECTED_VOICE_CHANNEL, stored for session start.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub channel_id: String,
    pub channel_name: Option<String>,
    pub guild_id: Option<String>,
    pub guild_name: Option<String>,
    pub self_user_id: Option<String>,
    pub user_labels: std::collections::HashMap<String, String>,
}
