//! Switches for running against `tools/fake-discord` instead of the Discord app. Both are
//! unset in normal use; the fake prints the values to set.
//!
//! - `DSCRIBE_DISCORD_IPC`: the pipe (Windows) or socket (Linux, macOS) to connect to instead
//!   of `discord-ipc-N`.
//! - `DSCRIBE_DISCORD_API`: replaces `https://discord.com/api` for the token exchange and REST.
//!
//! While either is set, the Discord login is kept in `discord_tokens.fake.json`, so a fake
//! login never replaces the real one.

const IPC_VAR: &str = "DSCRIBE_DISCORD_IPC";
const API_VAR: &str = "DSCRIBE_DISCORD_API";
const DISCORD_API: &str = "https://discord.com/api";

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The pipe or socket to use instead of Discord's.
pub fn ipc_path() -> Option<String> {
    var(IPC_VAR)
}

/// Base URL of the Discord API, without a trailing slash.
pub fn api_base() -> String {
    var(API_VAR).map(|v| v.trim_end_matches('/').to_string()).unwrap_or_else(|| DISCORD_API.into())
}

/// True when talking to the fake.
pub fn active() -> bool {
    ipc_path().is_some() || var(API_VAR).is_some()
}

/// Logs once at startup so a fake setup is never mistaken for the real thing.
pub fn log_if_active() {
    if active() {
        log::warn!(
            "[discord-rpc] Using a fake Discord: {}={}, {}={}",
            IPC_VAR,
            ipc_path().unwrap_or_default(),
            API_VAR,
            api_base()
        );
    }
}
