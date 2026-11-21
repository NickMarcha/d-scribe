# fake-discord

A stand-in for the Discord desktop app, so you can try d-scribe without Discord, a second account or anyone to call. d-scribe connects to it like it connects to Discord. It plays a scenario (people join, talk and leave), and you can change the call by typing commands.

The Rust tests use the same fake on an in-memory stream (`src-tauri/src/discord_rpc/client.rs`).

## Run it

Quit any running d-scribe first (a second copy just opens the first one).

```
cargo run --manifest-path tools/fake-discord/Cargo.toml -- --scenario tools/fake-discord/scenarios/call.json --tones
```

It prints the two environment variables to start d-scribe with. On Windows, in another PowerShell:

```
$env:DSCRIBE_DISCORD_IPC = '\\.\pipe\d-scribe-fake-discord'
$env:DSCRIBE_DISCORD_API = 'http://127.0.0.1:6464'
npm run tauri dev
```

Then click Connect in Settings > Discord. Any Client ID works, and the popup is approved right away. The timeline starts when d-scribe has signed in. With the variables set, d-scribe keeps this login in `discord_tokens.fake.json`, so your real Discord login stays as it is. Recordings go to your normal recordings list.

## Audio

d-scribe records only Discord's own audio by default, and the fake isn't Discord. To record what the fake plays, pick your speakers or headphones as the source for others in Settings > Audio. A speaker with a `wav` clip plays that clip. Without one, `--tones` plays a tone (a different pitch per person) while they speak, so you can see in the recording whether the speaker labels line up with the sound.

For transcription tests, record a few short clips of real speech and point `wav` at them (paths are relative to the scenario file).

## Scenarios

A scenario is a JSON file. Everything is optional:

```json
{
  "user": { "id": "100", "username": "you" },
  "guilds": { "300": "Fake server" },
  "deny_authorize": false,
  "reject_tokens": false,
  "timeline": [
    { "at": 0, "do": "join", "channel": { "id": "301", "name": "General", "guild_id": "300", "type": 2,
        "members": [{ "id": "201", "username": "alice", "nick": "Alice" }] } },
    { "at": 8, "do": "speak", "user": "201", "seconds": 3 },
    { "at": 12, "do": "speak", "user": "201", "wav": "clips/hello.wav" },
    { "at": 15, "do": "add_member", "member": { "id": "202", "username": "bob" } },
    { "at": 20, "do": "remove_member", "id": "202" },
    { "at": 25, "do": "close" },
    { "at": 30, "do": "leave" }
  ]
}
```

- `at` is seconds after d-scribe signs in. Leave a few seconds before the first `speak`: auto-record checks every 3 seconds.
- Channel `type`: 1 = DM, 2 = server voice channel (the default), 3 = group DM. You're added to the members by yourself.
- `close` closes every connection, like Discord quitting or restarting for an update.
- `deny_authorize` cancels the sign-in popup; `reject_tokens` refuses the saved login (as when it expired or was revoked).

Examples are in `scenarios/`: `call.json` (a server call with overlapping speakers, a short interjection, someone joining and someone leaving), `dm.json`, and `discord-restarts.json`.

## Commands

Type `help` while it runs: `status`, `join`, `leave`, `add ID NAME`, `remove ID`, `speak ID [SECONDS]`, `say ID FILE.wav`, `close`, `deny on|off`, `reject on|off`, `replay`, `log`, `quit`.

## How close to Discord it is

It answers what d-scribe uses: the IPC handshake and framing, READY, AUTHORIZE (with PKCE S256), AUTHENTICATE, GET_SELECTED_VOICE_CHANNEL, GET_GUILD, SUBSCRIBE and UNSUBSCRIBE for SPEAKING_START/STOP, PING, CLOSE, and the HTTP endpoints `POST /oauth2/token` and `GET /v10/users/@me/guilds`. It follows Discord's rules where d-scribe depends on them: commands need AUTHENTICATE first, codes work once and need the right verifier and `redirect_uri`, refresh tokens rotate, speaking events only reach connections subscribed to that channel. Responses carry the fields d-scribe reads plus the main others Discord sends. When Discord behaves differently from the fake, change the fake to match and add a test in `tests/protocol.rs`.
