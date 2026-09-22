//! The playback state machine.
//!
//! This module is deliberately platform-agnostic and free of I/O: it decides
//! *what* should be spoken and *where* playback is, never *how* to speak. That
//! makes the rules - chunk progression, progress accounting, pause/resume,
//! completion - testable without a speech engine or a COM apartment.
//!
//! All lengths here are characters, matching `crate::text`.

use crate::text::{self, Chunk};
use serde::{Deserialize, Serialize};

/// Characters of the source text kept for display in the overlay.
pub const PREVIEW_CHARS: usize = 500;
/// Playback rates the UI may select. The engine maps these onto its own scale.
pub const MIN_RATE: f32 = 0.5;
pub const MAX_RATE: f32 = 4.0;
pub const DEFAULT_RATE: f32 = 1.0;

/// What is currently being spoken.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlaybackSource {
    Clipboard,
    Book {
        book_id: String,
        chapter_index: usize,
        total_chapters: usize,
    },
}

impl PlaybackSource {
    pub fn chapter_index(&self) -> Option<usize> {
        match self {
            Self::Clipboard => None,
            Self::Book { chapter_index, .. } => Some(*chapter_index),
        }
    }
}

/// Coarse playback state for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
}

/// A unit of work handed to the state machine.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackJob {
    pub source: PlaybackSource,
    /// Human label for the current job, e.g. `Clipboard` or `Book - Chapter 3`.
    pub title: String,
    /// First [`PREVIEW_CHARS`] characters, for display only.
    pub text_preview: String,
    pub chunks: Vec<Chunk>,
}

impl PlaybackJob {
    /// Build a job from raw text, chunking it with the shared rules.
    pub fn new(source: PlaybackSource, title: impl Into<String>, text: &str) -> Self {
        let chunks = text::split_into_chunks(text, text::CHUNK_CHARS);
        Self {
            source,
            title: title.into(),
            text_preview: preview(text),
            chunks,
        }
    }

    pub fn total_chars(&self) -> u32 {
        self.chunks
            .iter()
            .map(|chunk| chunk.char_count as u32)
            .sum()
    }
}

fn preview(text: &str) -> String {
    if text::char_count(text) <= PREVIEW_CHARS {
        return text.to_string();
    }
    text.chars().take(PREVIEW_CHARS).collect::<String>() + "..."
}

/// What the frontend needs to render playback: a read-only projection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub source: Option<PlaybackSource>,
    pub title: String,
    pub text_preview: String,
    /// Characters fully spoken so far, never greater than `total_chars`.
    pub spoken_chars: u32,
    pub total_chars: u32,
    pub rate: f32,
    /// True once the last chunk of the job has finished.
    pub finished: bool,
}

/// The single owner of playback state.
#[derive(Debug)]
pub struct PlaybackState {
    job: Option<PlaybackJob>,
    status: PlaybackStatus,
    /// Index of the chunk that is currently being spoken.
    current_chunk: usize,
    /// Characters in every chunk before `current_chunk`.
    completed_chars: u32,
    /// A chunk was queued and has not been accounted for as finished yet.
    awaiting_engine: bool,
    rate: f32,
    finished: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            job: None,
            status: PlaybackStatus::Idle,
            current_chunk: 0,
            completed_chars: 0,
            awaiting_engine: false,
            rate: DEFAULT_RATE,
            finished: false,
        }
    }
}

impl PlaybackState {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when nothing is loaded. The hotkey handler uses this to decide
    /// whether a new request may start.
    pub fn is_idle(&self) -> bool {
        self.job.is_none() && self.status == PlaybackStatus::Idle
    }

    /// Start a new job, replacing whatever was playing.
    pub fn start(&mut self, job: PlaybackJob) {
        self.job = Some(job);
        self.status = PlaybackStatus::Playing;
        self.current_chunk = 0;
        self.completed_chars = 0;
        self.awaiting_engine = false;
        self.finished = false;
    }

    pub fn stop(&mut self) {
        self.job = None;
        self.status = PlaybackStatus::Idle;
        self.current_chunk = 0;
        self.completed_chars = 0;
        self.awaiting_engine = false;
        self.finished = false;
    }

    /// Pause or resume; returns the new paused flag. A no-op when idle.
    pub fn set_paused(&mut self, paused: bool) -> bool {
        if self.job.is_none() {
            return false;
        }
        self.status = if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        paused
    }

    pub fn toggle_paused(&mut self) -> bool {
        match self.status {
            PlaybackStatus::Paused => self.set_paused(false),
            _ => self.set_paused(true),
        }
    }

    pub fn set_rate(&mut self, rate: f32) -> f32 {
        if rate.is_finite() {
            self.rate = rate.clamp(MIN_RATE, MAX_RATE);
        }
        self.rate
    }

    pub fn total_chars(&self) -> u32 {
        self.job.as_ref().map_or(0, PlaybackJob::total_chars)
    }

    /// Text of the next chunk to speak, and how many characters it holds.
    /// Returns `None` when the job is exhausted; completion is then recorded.
    pub fn take_next_chunk(&mut self) -> Option<Chunk> {
        let job = self.job.as_ref()?;
        let chunk = job.chunks.get(self.current_chunk).cloned();
        match chunk {
            Some(chunk) => {
                self.awaiting_engine = true;
                Some(chunk)
            }
            None => {
                self.finished = true;
                None
            }
        }
    }

    /// True while a chunk has been handed to the engine but not yet accounted
    /// for. Callers use this to avoid reading "engine is silent" as "chunk is
    /// finished" before anything was ever queued.
    pub fn is_awaiting_engine(&self) -> bool {
        self.awaiting_engine
    }

    /// Characters of the current chunk the engine has already spoken, clamped
    /// to that chunk so a stray engine value can never over-report.
    fn clamp_offset(&self, offset_in_chunk: u32) -> u32 {
        let current_length = self
            .job
            .as_ref()
            .and_then(|job| job.chunks.get(self.current_chunk))
            .map_or(0, |chunk| chunk.char_count as u32);
        offset_in_chunk.min(current_length)
    }

    /// Total characters spoken so far, given the engine's offset inside the
    /// current chunk.
    pub fn spoken_chars(&self, offset_in_chunk: u32) -> u32 {
        let spoken = self
            .completed_chars
            .saturating_add(self.clamp_offset(offset_in_chunk));
        spoken.min(self.total_chars())
    }

    /// Mark the current chunk as fully spoken and move to the next one.
    /// Returns `true` when another chunk is waiting.
    pub fn advance_chunk(&mut self) -> bool {
        let Some(job) = self.job.as_ref() else {
            return false;
        };
        let Some(chunk) = job.chunks.get(self.current_chunk) else {
            self.finished = true;
            return false;
        };

        self.completed_chars = self.completed_chars.saturating_add(chunk.char_count as u32);
        self.current_chunk += 1;
        self.awaiting_engine = false;

        let has_more = self.current_chunk < job.chunks.len();
        if !has_more {
            self.finished = true;
            self.status = PlaybackStatus::Idle;
        }
        has_more
    }

    /// How many chunks the current job was split into.
    pub fn chunk_count(&self) -> usize {
        self.job.as_ref().map_or(0, |job| job.chunks.len())
    }

    /// Read-only projection for the frontend.
    pub fn snapshot(&self, offset_in_chunk: u32) -> PlaybackSnapshot {
        match self.job.as_ref() {
            Some(job) => PlaybackSnapshot {
                status: self.status,
                source: Some(job.source.clone()),
                title: job.title.clone(),
                text_preview: job.text_preview.clone(),
                spoken_chars: self.spoken_chars(offset_in_chunk),
                total_chars: job.total_chars(),
                rate: self.rate,
                finished: self.finished,
            },
            None => PlaybackSnapshot {
                status: PlaybackStatus::Idle,
                source: None,
                title: String::new(),
                text_preview: String::new(),
                spoken_chars: 0,
                total_chars: 0,
                rate: self.rate,
                finished: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clipboard_job(text: &str) -> PlaybackJob {
        PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", text)
    }

    fn book_job(book_id: &str, chapter_index: usize, text: &str) -> PlaybackJob {
        PlaybackJob::new(
            PlaybackSource::Book {
                book_id: book_id.to_string(),
                chapter_index,
                total_chapters: 7,
            },
            format!("Book - Chapter {}", chapter_index + 1),
            text,
        )
    }

    #[test]
    fn a_fresh_state_is_idle_with_nothing_to_report() {
        let state = PlaybackState::new();
        assert!(state.is_idle());
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Idle);
        assert_eq!(state.total_chars(), 0);
        assert_eq!(state.chunk_count(), 0);

        let snapshot = state.snapshot(0);
        assert_eq!(snapshot.status, PlaybackStatus::Idle);
        assert!(snapshot.source.is_none());
        assert_eq!(snapshot.total_chars, 0);
        assert_eq!(snapshot.rate, DEFAULT_RATE);
    }

    #[test]
    fn starting_a_job_resets_progress_and_counts_characters() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("héllo wörld"));

        assert!(!state.is_idle());
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Playing);
        assert_eq!(state.total_chars(), 11);
        assert_eq!(state.spoken_chars(0), 0);
        assert_eq!(state.snapshot(0).text_preview, "héllo wörld");
    }

    #[test]
    fn progress_is_measured_in_characters_across_chunk_boundaries() {
        // Two chunks of 10 characters each with a limit of 10.
        let text = "abcdefghij klmnopqrst";
        let mut state = PlaybackState::new();
        state.start(PlaybackJob {
            chunks: text::split_into_chunks(text, 10),
            ..clipboard_job(text)
        });
        assert_eq!(state.chunk_count(), 2);

        assert_eq!(state.spoken_chars(0), 0);
        assert_eq!(state.spoken_chars(4), 4);

        assert!(state.advance_chunk());
        // The dropped separator whitespace is not counted as spoken.
        assert_eq!(state.spoken_chars(0), 10);
        assert_eq!(state.spoken_chars(5), 15);
    }

    #[test]
    fn the_engine_offset_is_clamped_to_the_current_chunk() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("0123456789"));

        // A bogus offset from the engine can never over-report progress.
        assert_eq!(state.spoken_chars(u32::MAX), 10);
        assert_eq!(state.spoken_chars(999), 10);
    }

    #[test]
    fn progress_never_exceeds_the_total() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job(&"word ".repeat(100)));

        while state.advance_chunk() {
            assert!(state.spoken_chars(u32::MAX) <= state.total_chars());
        }
        assert_eq!(state.spoken_chars(u32::MAX), state.total_chars());
    }

    #[test]
    fn advancing_past_the_last_chunk_finishes_the_job() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("one chunk only"));

        assert!(
            !state.advance_chunk(),
            "there is nothing after the only chunk"
        );
        assert!(state.snapshot(0).finished);
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Idle);
        // The text stays available so the UI can still show what was read.
        assert_eq!(state.snapshot(0).title, "Clipboard");
    }

    #[test]
    fn taking_a_chunk_marks_the_engine_as_busy_until_it_finishes() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job(&"a".repeat(25)));

        assert!(!state.is_awaiting_engine());
        assert!(state.take_next_chunk().is_some());
        assert!(state.is_awaiting_engine());

        state.advance_chunk();
        assert!(!state.is_awaiting_engine());
    }

    #[test]
    fn taking_chunks_walks_through_the_job_in_order() {
        // Force three chunks by chunking with a small limit instead of relying
        // on the production chunk size.
        let text = "a".repeat(25);
        let mut state = PlaybackState::new();
        state.start(PlaybackJob {
            chunks: text::split_into_chunks(&text, 10),
            ..clipboard_job(&text)
        });

        let first = state.take_next_chunk().expect("first chunk");
        assert_eq!(first.start_char, 0);
        assert!(state.advance_chunk());

        let second = state.take_next_chunk().expect("second chunk");
        assert!(second.start_char > 0);
    }

    #[test]
    fn pausing_and_resuming_round_trips() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("hello"));

        assert!(state.toggle_paused());
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Paused);
        assert!(state.snapshot(0).status == PlaybackStatus::Paused);

        assert!(!state.toggle_paused());
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Playing);
    }

    #[test]
    fn pausing_an_idle_state_does_nothing() {
        let mut state = PlaybackState::new();
        assert!(!state.toggle_paused());
        assert!(state.is_idle());
        assert_eq!(state.snapshot(0).status, PlaybackStatus::Idle);
    }

    #[test]
    fn stopping_clears_the_job_entirely() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("some text"));
        state.stop();

        assert!(state.is_idle());
        assert_eq!(state.total_chars(), 0);
        assert_eq!(state.chunk_count(), 0);
        let snapshot = state.snapshot(0);
        assert!(snapshot.source.is_none());
        assert_eq!(snapshot.spoken_chars, 0);
    }

    #[test]
    fn rate_is_clamped_to_the_supported_range() {
        let mut state = PlaybackState::new();
        assert_eq!(state.set_rate(1.5), 1.5);
        assert_eq!(state.set_rate(99.0), MAX_RATE);
        assert_eq!(state.set_rate(0.01), MIN_RATE);
        // Invalid input is ignored rather than poisoning the state.
        assert_eq!(state.set_rate(f32::NAN), MIN_RATE);
        assert_eq!(state.snapshot(0).rate, MIN_RATE);
    }

    #[test]
    fn a_book_job_reports_its_chapter_and_book_identity() {
        let mut state = PlaybackState::new();
        state.start(book_job("book-1", 2, "chapter text"));

        let snapshot = state.snapshot(0);
        let source = snapshot.source.expect("book source");
        assert_eq!(source.chapter_index(), Some(2));
        assert_eq!(snapshot.title, "Book - Chapter 3");
        assert_eq!(snapshot.text_preview, "chapter text");
    }

    #[test]
    fn clipboards_are_not_attributed_to_any_book() {
        assert_eq!(PlaybackSource::Clipboard.chapter_index(), None);
    }

    #[test]
    fn the_preview_is_truncated_on_a_character_boundary() {
        let text = "👍".repeat(PREVIEW_CHARS + 50);
        let job = clipboard_job(&text);
        assert!(job.text_preview.ends_with("..."));
        assert!(text::char_count(&job.text_preview) <= PREVIEW_CHARS + 3);
    }

    #[test]
    fn starting_a_new_job_replaces_the_previous_one() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("first text"));
        state.advance_chunk();

        state.start(book_job("book-9", 0, "second text"));

        assert_eq!(state.spoken_chars(0), 0);
        assert_eq!(state.total_chars(), 11);
        assert_eq!(
            state.snapshot(0).source.expect("book").chapter_index(),
            Some(0)
        );
    }

    #[test]
    fn an_empty_job_is_finished_immediately() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("   "));

        assert_eq!(state.total_chars(), 0);
        assert!(state.take_next_chunk().is_none());
        assert!(state.snapshot(0).finished);
    }

    #[test]
    fn snapshots_are_independent_copies() {
        let mut state = PlaybackState::new();
        state.start(clipboard_job("original"));
        let snapshot = state.snapshot(0);

        state.stop();

        assert_eq!(snapshot.title, "Clipboard");
        assert!(state.is_idle());
    }
}
