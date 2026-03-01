//! Discord RPC client for connecting to the local Discord client and subscribing to voice events.

mod client;
mod events;
mod token_store;

#[cfg(windows)]
mod ipc;

pub use token_store::{load_tokens, save_tokens, DiscordTokens};

pub use client::DiscordRpcClient;
pub use events::{ChannelInfo, SpeakingEvent};

use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref CHANNEL_INFO: Mutex<Option<ChannelInfo>> = Mutex::new(None);
    static ref RPC_CONNECTED: Mutex<bool> = Mutex::new(false);
}

pub fn set_rpc_connected(connected: bool) {
    *RPC_CONNECTED.lock().unwrap() = connected;
}

pub fn is_rpc_connected() -> bool {
    *RPC_CONNECTED.lock().unwrap()
}

pub fn set_channel_info(info: ChannelInfo) {
    *CHANNEL_INFO.lock().unwrap() = Some(info);
}

pub fn clear_channel_info() {
    *CHANNEL_INFO.lock().unwrap() = None;
}

pub fn get_channel_info() -> Option<ChannelInfo> {
    CHANNEL_INFO.lock().unwrap().clone()
}
