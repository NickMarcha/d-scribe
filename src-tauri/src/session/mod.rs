//! Session recording and speaker segmentation.

mod recorder;

pub use recorder::{
    clear_live_segment_tx, flush_pending_if_elapsed, record_speaking_event, set_live_segment_tx,
    start_session, stop_session,
};
pub use recorder::{SessionAudioPaths, SessionSegment, SessionState};
