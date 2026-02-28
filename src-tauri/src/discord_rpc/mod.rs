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
}

pub fn set_channel_info(info: ChannelInfo) {
    *CHANNEL_INFO.lock().unwrap() = Some(info);
}

pub fn get_channel_info() -> Option<ChannelInfo> {
    CHANNEL_INFO.lock().unwrap().clone()
}
