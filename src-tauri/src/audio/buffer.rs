//! Ring buffer for real-time audio extraction.
//! Holds samples for segment extraction during recording.

use std::collections::VecDeque;

/// At 16 kHz: 1 ms = 16 samples.
const SAMPLES_PER_MS: u64 = 16;

/// Max buffer duration: 5 minutes at 16 kHz.
/// Prevents unbounded memory growth.
const MAX_SAMPLES: usize = 16 * 60 * 5 * 1000; // 4_800_000 samples

/// Audio buffer for real-time segment extraction.
/// Samples are appended in order; we can extract by (start_ms, end_ms).
/// Bounded to ~5 min. Tracks base_sample for correct indexing when old data is dropped.
pub struct AudioBuffer {
    samples: VecDeque<i16>,
    base_sample: u64, // sample index of first element (increases as we drop)
}

impl AudioBuffer {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(64 * 1024),
            base_sample: 0,
        }
    }

    /// Append a sample. Drops oldest when at capacity.
    pub fn push(&mut self, sample: i16) {
        if self.samples.len() >= MAX_SAMPLES {
            self.samples.pop_front();
            self.base_sample += 1;
        }
        self.samples.push_back(sample);
    }

    /// Extract samples for start_ms..end_ms. Returns empty if range not available or already dropped.
    pub fn extract(&self, start_ms: u64, end_ms: u64) -> Vec<i16> {
        let start_sample = start_ms * SAMPLES_PER_MS;
        let end_sample = end_ms * SAMPLES_PER_MS;
        if start_sample < self.base_sample {
            return Vec::new(); // range already dropped
        }
        let rel_start = (start_sample - self.base_sample) as usize;
        let rel_end = (end_sample - self.base_sample) as usize;
        let count = rel_end.saturating_sub(rel_start);
        if rel_start >= self.samples.len() || count == 0 {
            return Vec::new();
        }
        let end = (rel_start + count).min(self.samples.len());
        self.samples.range(rel_start..end).copied().collect()
    }

    /// Current length in samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }
}
