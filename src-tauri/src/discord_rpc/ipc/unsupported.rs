//! Discord IPC for systems without a backend yet. Linux and macOS use Unix sockets named
//! `discord-ipc-N` (on Linux in `$XDG_RUNTIME_DIR`, plus the Flatpak and Snap folders there);
//! that backend goes in its own file with these two functions, selected in `mod.rs`.

pub async fn connect() -> Result<(String, tokio::io::DuplexStream), String> {
    Err("Connecting to Discord isn't supported on this system yet.".into())
}

pub fn discord_running() -> bool {
    false
}
