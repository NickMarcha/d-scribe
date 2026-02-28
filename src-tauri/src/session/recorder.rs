//! Session recorder - tracks speaking events and segments.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate session/project name from template.
/// Placeholders: {guild}, {channel}, {timestamp}, {date}, {time}
fn format_session_id(
    template: &str,
    guild: Option<&str>,
    channel: Option<&str>,
) -> String {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp().to_string();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H-%M-%S").to_string();

    let mut s = template.to_string();
    s = s.replace("{guild}", guild.unwrap_or("Unknown"));
    s = s.replace("{channel}", channel.unwrap_or("Unknown"));
    s = s.replace("{timestamp}", &timestamp);
    s = s.replace("{date}", &date);
    s = s.replace("{time}", &time);
    // Sanitize for filesystem: replace invalid chars
    s.replace(|c: char| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'), "_")
}

/// A single segment of speech from a speaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub user_id: String,
    pub speaker_name: Option<String>,
}

/// Full session state for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub created_at: u64,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    pub channel_id: Option<String>,
    pub self_user_id: Option<String>,
    pub segments: Vec<SessionSegment>,
    pub transcript_texts: Vec<String>,
    pub audio_paths: SessionAudioPaths,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionAudioPaths {
    pub loopback: Option<String>,
    pub microphone: Option<String>,
}

/// Pending segment waiting for merge buffer - not finalized until silence exceeds buffer.
struct PendingSegment {
    start_ms: u64,
    stop_ms: u64,
    user_id: String,
}

struct ActiveSession {
    start_time: SystemTime,
    segments: Vec<SessionSegment>,
    user_labels: HashMap<String, String>,
    self_user_id: Option<String>,
    guild_name: Option<String>,
    channel_name: Option<String>,
    channel_id: Option<String>,
    open_segments: HashMap<String, u64>, // user_id -> start_ms
    pending_cooldown: HashMap<String, PendingSegment>, // user_id -> pending (waiting to see if they speak again)
    segment_merge_buffer_ms: u64, // min silence (ms) before splitting; e.g. 1000 = merge if gap < 1s
    project_name_template: String,
}

lazy_static::lazy_static! {
    static ref ACTIVE_SESSION: Mutex<Option<ActiveSession>> = Mutex::new(None);
}

fn elapsed_ms_since(start: SystemTime) -> u64 {
    SystemTime::now()
        .duration_since(start)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Start a new recording session.
/// `segment_merge_buffer_ms`: min silence (ms) before splitting segments; e.g. 1000 = merge if gap < 1s.
/// `project_name_template`: template for session_id, e.g. "{guild}_{channel}_{timestamp}".
pub fn start_session(
    guild_name: Option<String>,
    channel_name: Option<String>,
    channel_id: Option<String>,
    self_user_id: Option<String>,
    user_labels: HashMap<String, String>,
    segment_merge_buffer_ms: u64,
    project_name_template: String,
) {
    let session = ActiveSession {
        start_time: SystemTime::now(),
        segments: Vec::new(),
        user_labels,
        self_user_id,
        guild_name,
        channel_name,
        channel_id,
        open_segments: HashMap::new(),
        pending_cooldown: HashMap::new(),
        segment_merge_buffer_ms: segment_merge_buffer_ms.max(1),
        project_name_template: if project_name_template.is_empty() {
            "{guild}_{channel}_{timestamp}".to_string()
        } else {
            project_name_template
        },
    };
    *ACTIVE_SESSION.lock().unwrap() = Some(session);
}

/// Flush a pending segment to the segments list.
fn flush_pending(session: &mut ActiveSession, user_id: &str) {
    if let Some(pending) = session.pending_cooldown.remove(user_id) {
        let speaker_name = session.user_labels.get(user_id).cloned();
        session.segments.push(SessionSegment {
            start_ms: pending.start_ms,
            end_ms: pending.stop_ms,
            user_id: pending.user_id,
            speaker_name,
        });
    }
}

/// Record a SPEAKING_START or SPEAKING_STOP event.
/// Uses segment_merge_buffer_ms: brief silences (< buffer) are merged into one segment.
pub fn record_speaking_event(is_start: bool, user_id: String) {
    let mut guard = ACTIVE_SESSION.lock().unwrap();
    if let Some(ref mut session) = *guard {
        let elapsed = elapsed_ms_since(session.start_time);
        let buffer = session.segment_merge_buffer_ms;

        if is_start {
            // Flush pending for OTHER users (they've been silent, we're switching speakers)
            let others: Vec<String> = session
                .pending_cooldown
                .keys()
                .filter(|id| *id != &user_id)
                .cloned()
                .collect();
            for id in others {
                flush_pending(session, &id);
            }

            if let Some(pending) = session.pending_cooldown.remove(&user_id) {
                let gap = elapsed.saturating_sub(pending.stop_ms);
                if gap <= buffer {
                    // Same utterance - merge: keep speaking, extend the segment
                    session.open_segments.insert(user_id.clone(), pending.start_ms);
                } else {
                    // Gap exceeded buffer - finalize previous, start new
                    let speaker_name = session.user_labels.get(&user_id).cloned();
                    session.segments.push(SessionSegment {
                        start_ms: pending.start_ms,
                        end_ms: pending.stop_ms,
                        user_id: pending.user_id.clone(),
                        speaker_name,
                    });
                    session.open_segments.insert(user_id.clone(), elapsed);
                }
            } else if !session.open_segments.contains_key(&user_id) {
                // Fresh start
                session.open_segments.insert(user_id.clone(), elapsed);
            }
            // else: already in open_segments (duplicate start), ignore
        } else {
            // Stop
            if let Some(start_ms) = session.open_segments.remove(&user_id) {
                session.pending_cooldown.insert(
                    user_id.clone(),
                    PendingSegment {
                        start_ms,
                        stop_ms: elapsed,
                        user_id: user_id.clone(),
                    },
                );
            } else if let Some(ref mut pending) = session.pending_cooldown.get_mut(&user_id) {
                // Stop without start - extend stop time
                pending.stop_ms = elapsed;
            }
        }
    }
}

/// Stop the session and return the state for persistence.
pub fn stop_session(audio_paths: SessionAudioPaths) -> Option<SessionState> {
    let mut guard = ACTIVE_SESSION.lock().unwrap();
    if let Some(mut session) = guard.take() {
        // Flush all pending and open segments
        for user_id in session.pending_cooldown.keys().cloned().collect::<Vec<_>>() {
            flush_pending(&mut session, &user_id);
        }
        for (user_id, start_ms) in session.open_segments.drain() {
            let elapsed = elapsed_ms_since(session.start_time);
            let speaker_name = session.user_labels.get(&user_id).cloned();
            session.segments.push(SessionSegment {
                start_ms,
                end_ms: elapsed,
                user_id,
                speaker_name,
            });
        }

        let created_at = session
            .start_time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let session_id = format_session_id(
            &session.project_name_template,
            session.guild_name.as_deref(),
            session.channel_name.as_deref(),
        );
        Some(SessionState {
            session_id,
            created_at,
            guild_name: session.guild_name,
            channel_name: session.channel_name,
            channel_id: session.channel_id,
            self_user_id: session.self_user_id,
            segments: session.segments,
            transcript_texts: vec![], // Filled by transcription or manual edit
            audio_paths,
        })
    } else {
        None
    }
}
