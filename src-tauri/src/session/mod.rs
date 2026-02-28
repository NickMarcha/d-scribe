//! Session recording and speaker segmentation.

mod recorder;

pub use recorder::{SessionAudioPaths, SessionSegment, SessionState};
pub use recorder::{record_speaking_event, start_session, stop_session};
