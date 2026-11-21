//! `fake-discord`: run a fake Discord app that d-scribe connects to instead of the real one.
//!
//!     cargo run --manifest-path tools/fake-discord/Cargo.toml -- --scenario tools/fake-discord/scenarios/call.json
//!
//! It prints the two environment variables to start d-scribe with, then plays the scenario
//! once d-scribe has connected. Type `help` for commands to change the call by hand.

use fake_discord::scenario::{Action, Channel, Member, Scenario};
use fake_discord::FakeDiscord;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

const USAGE: &str = "fake-discord [--scenario FILE] [--pipe NAME] [--http ADDR] [--tones]

  --scenario FILE  the call to play (see tools/fake-discord/scenarios); default: an empty one
  --pipe NAME      where d-scribe connects; default: \\\\.\\pipe\\d-scribe-fake-discord on Windows,
                   $XDG_RUNTIME_DIR/d-scribe-fake-discord elsewhere
  --http ADDR      the token endpoint; default 127.0.0.1:6464
  --tones          play a tone while someone without a WAV clip speaks";

const HELP: &str = "Commands:
  status                    the channel and who's in it
  join                      join the scenario's channel (or a test channel)
  leave                     leave the voice channel
  add ID NAME               someone joins
  remove ID                 someone leaves
  speak ID [SECONDS]        someone talks (default 2 s)
  say ID FILE.wav           someone talks, playing a WAV clip
  close                     Discord closes every connection
  deny on|off               the next AUTHORIZE is cancelled
  reject on|off             tokens are refused
  replay                    run the scenario's timeline again
  log                       commands d-scribe sent so far
  quit";

struct Args {
    scenario: Option<PathBuf>,
    pipe: String,
    http: String,
    tones: bool,
}

fn default_pipe() -> String {
    if cfg!(windows) {
        r"\\.\pipe\d-scribe-fake-discord".into()
    } else {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
        Path::new(&dir).join("d-scribe-fake-discord").to_string_lossy().into_owned()
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { scenario: None, pipe: default_pipe(), http: "127.0.0.1:6464".into(), tones: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scenario" => args.scenario = Some(it.next().ok_or("--scenario needs a file")?.into()),
            "--pipe" => args.pipe = it.next().ok_or("--pipe needs a name")?,
            "--http" => args.http = it.next().ok_or("--http needs an address")?,
            "--tones" => args.tones = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("Unknown argument: {}", other)),
        }
    }
    Ok(args)
}

fn load_scenario(path: &Path) -> Result<Scenario, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Can't read {}: {}", path.display(), e))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{}\n", e);
            }
            eprintln!("{}", USAGE);
            std::process::exit(2);
        }
    };
    let (scenario, base_dir) = match &args.scenario {
        Some(p) => match load_scenario(p) {
            Ok(s) => (s, p.parent().map(Path::to_path_buf).unwrap_or_default()),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        },
        None => (Scenario::default(), PathBuf::from(".")),
    };
    let fake = FakeDiscord::with_options(scenario, base_dir, false);
    #[cfg(feature = "audio")]
    fake.set_player(audio::player(args.tones));
    #[cfg(not(feature = "audio"))]
    if args.tones {
        eprintln!("--tones needs the audio feature");
    }

    let http = match fake.serve_http(&args.http) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    println!("Fake Discord is running. Start d-scribe with:\n");
    if cfg!(windows) {
        println!("  $env:DSCRIBE_DISCORD_IPC = '{}'", args.pipe);
        println!("  $env:DSCRIBE_DISCORD_API = 'http://{}'", http);
        println!("  npm run tauri dev\n");
    } else {
        println!("  DSCRIBE_DISCORD_IPC='{}' DSCRIBE_DISCORD_API='http://{}' npm run tauri dev\n", args.pipe, http);
    }
    println!("Any Client ID works. Type `help` for commands.\n");

    let listener = {
        let fake = fake.clone();
        let pipe = args.pipe.clone();
        tokio::spawn(async move {
            if let Err(e) = listen(fake, &pipe).await {
                eprintln!("Listening on {} failed: {}", pipe, e);
                std::process::exit(1);
            }
        })
    };
    console(fake).await;
    listener.abort();
    #[cfg(unix)]
    let _ = std::fs::remove_file(&args.pipe);
}

#[cfg(windows)]
async fn listen(fake: Arc<FakeDiscord>, pipe: &str) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut server = ServerOptions::new().first_pipe_instance(true).create(pipe)?;
    loop {
        server.connect().await?;
        let connected = server;
        server = ServerOptions::new().create(pipe)?;
        tokio::spawn(fake.clone().serve(connected));
    }
}

#[cfg(unix)]
async fn listen(fake: Arc<FakeDiscord>, path: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(fake.clone().serve(stream));
    }
}

fn test_channel() -> Channel {
    Channel {
        id: "200".into(),
        name: Some("General".into()),
        guild_id: None,
        kind: 3,
        members: vec![Member { id: "201".into(), username: "alice".into(), nick: Some("Alice".into()) }],
    }
}

async fn console(fake: Arc<FakeDiscord>) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let words: Vec<&str> = line.split_whitespace().collect();
        let on = |w: Option<&&str>| matches!(w, Some(&"on"));
        match words.as_slice() {
            [] => {}
            ["help"] => println!("{}", HELP),
            ["status"] => println!("{}", fake.status()),
            ["join"] => fake.apply(Action::Join { channel: fake.scenario_channel().unwrap_or_else(test_channel) }),
            ["leave"] => fake.apply(Action::Leave),
            ["add", id, name @ ..] if !name.is_empty() => fake.apply(Action::AddMember {
                member: Member { id: id.to_string(), username: name.join(" "), nick: None },
            }),
            ["remove", id] => fake.apply(Action::RemoveMember { id: id.to_string() }),
            ["speak", id] => fake.apply(Action::Speak { user: id.to_string(), seconds: None, wav: None }),
            ["speak", id, secs] => match secs.parse::<f64>() {
                Ok(s) => fake.apply(Action::Speak { user: id.to_string(), seconds: Some(s), wav: None }),
                Err(_) => println!("Seconds must be a number"),
            },
            ["say", id, file] => fake.apply(Action::Speak { user: id.to_string(), seconds: None, wav: Some(file.into()) }),
            ["close"] => fake.apply(Action::Close),
            ["deny", ..] => fake.set_deny_authorize(on(words.get(1))),
            ["reject", ..] => fake.set_reject_tokens(on(words.get(1))),
            ["replay"] => fake.restart_timeline(),
            ["log"] => println!("{}", fake.commands().join("\n")),
            ["quit"] | ["exit"] => break,
            _ => println!("Unknown command. Type `help`."),
        }
    }
}

#[cfg(feature = "audio")]
mod audio {
    use super::*;
    use rodio::Source;
    use std::sync::mpsc;
    use std::sync::Mutex;

    enum Play {
        Wav(PathBuf),
        Tone(f32, Duration),
    }

    /// Plays on the default output device from its own thread (rodio's stream isn't Send).
    pub fn player(tones: bool) -> fake_discord::Player {
        let (tx, rx) = mpsc::channel::<Play>();
        std::thread::spawn(move || {
            let (_stream, handle) = match rodio::OutputStream::try_default() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("No audio output ({}); speaking events only.", e);
                    return;
                }
            };
            for item in rx {
                let Ok(sink) = rodio::Sink::try_new(&handle) else { continue };
                match item {
                    Play::Wav(path) => match std::fs::File::open(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|f| rodio::Decoder::new(std::io::BufReader::new(f)).map_err(|e| e.to_string()))
                    {
                        Ok(source) => sink.append(source),
                        Err(e) => eprintln!("Can't play {}: {}", path.display(), e),
                    },
                    Play::Tone(hz, length) => {
                        sink.append(rodio::source::SineWave::new(hz).take_duration(length).amplify(0.15))
                    }
                }
                sink.detach();
            }
        });
        let tx = Mutex::new(tx);
        Box::new(move |user: &str, wav: Option<&Path>, length: Duration| {
            let item = match wav {
                Some(p) => Play::Wav(p.to_path_buf()),
                None if tones => {
                    // A different pitch per person, so you can tell them apart in a recording.
                    let n: u32 = user.bytes().map(u32::from).sum();
                    Play::Tone(300.0 + (n % 8) as f32 * 60.0, length)
                }
                None => return,
            };
            let _ = tx.lock().unwrap().send(item);
        })
    }
}
