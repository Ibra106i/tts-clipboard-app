//! The playback state machine.
//!
//! This module is deliberately platform-agnostic and free of I/O: it decides
//! *what* should be spoken and *where* playback is, never *how* to speak. That
//! makes the rules - chunk progression, progress accounting, pause/resume,
//! completion - testable without a speech engine or a COM apartment.
//!
//! All lengths here are characters, matching `crate::text`.

use crate::models::TextRange;
use crate::text::{self, Chunk};
use serde::{Deserialize, Serialize};

/// Characters of the source text kept for display in the overlay.
pub const PREVIEW_CHARS: usize = 500;
/// Playback rates the UI may select. The engine maps these onto its own scale.
pub const MIN_RATE: f32 = 0.5;
pub const MAX_RATE: f32 = 4.0;
pub const DEFAULT_RATE: f32 = 1.0;

/// What is currently being spoken.
///
/// Serialized as `{ "kind": "clipboard" }` or
/// `{ "kind": "book", "book_id": ..., ... }`. The wire shape is pinned by a
/// test because the frontend's typed IPC layer depends on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

/// Coarse playback state for the UI. Serialized as `"idle"`, `"playing"` or
/// `"paused"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Offset, in the full source text, of the first character this job reads.
    ///
    /// Zero for a whole-text job. The frontend reports it so the reader can show
    /// where playback began, and persists it as the book's reading position.
    /// Chunk offsets stay absolute, so `spoken_chars` remains comparable with a
    /// position in the chapter the reader is looking at.
    pub start_char: usize,
    /// The span of the source text this job covers, once resolved against the
    /// text itself. `None` means the whole text.
    pub range: Option<ResolvedRange>,
}

/// A [`TextRange`] after its offsets have been clamped to the text they address
/// and, when asked, snapped forward to a sentence boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRange {
    pub start: usize,
    pub end: usize,
}

impl PlaybackJob {
    /// Build a job from raw text, chunking it with the shared rules.
    ///
    /// `range` restricts the job to a span of `text`. A span that resolves to
    /// nothing — an empty selection, or a click past the end of the chapter —
    /// produces a job with no chunks, which reports itself finished rather than
    /// failing, so a stale offset can never make the reader refuse to speak.
    ///
    /// Chunk offsets are rebased onto `text`, not onto the slice, so every
    /// offset this job reports is an absolute position in the chapter.
    pub fn new(
        source: PlaybackSource,
        title: impl Into<String>,
        text: &str,
        range: Option<TextRange>,
    ) -> Self {
        let resolved = range.map(|requested| resolve_range(text, requested));
        let (start, slice) = match resolved {
            Some(ResolvedRange { start, end }) => {
                let from = text::byte_offset_of_char(text, start);
                let to = text::byte_offset_of_char(text, end);
                (start, &text[from..to])
            }
            None => (0, text),
        };

        let mut chunks = text::split_into_chunks(slice, text::CHUNK_CHARS);
        for chunk in &mut chunks {
            chunk.start_char += start;
        }

        Self {
            source,
            title: title.into(),
            text_preview: preview(slice),
            chunks,
            start_char: start,
            range: resolved,
        }
    }

    pub fn total_chars(&self) -> u32 {
        self.chunks
            .iter()
            .map(|chunk| chunk.char_count as u32)
            .sum()
    }
}

/// Clamp a requested range to the text and apply sentence alignment.
fn resolve_range(text: &str, requested: TextRange) -> ResolvedRange {
    let start = if requested.align_to_sentence {
        text::snap_to_sentence_start(text, requested.start)
    } else {
        requested.start.min(text::char_count(text))
    };
    let end = requested
        .end
        .unwrap_or_else(|| text::char_count(text))
        .min(text::char_count(text))
        .max(start);
    ResolvedRange { start, end }
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
    /// Offset, in the full source text, of the first character being read.
    /// Non-zero when playback started part-way into a chapter, so the reader can
    /// show and persist where the user is.
    pub start_char: usize,
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
    /// The engine has been observed speaking at least once for the current
    /// chunk. Silence only means "finished" once this is set: `speak` is
    /// asynchronous, so a progress read taken immediately after it returns can
    /// see a voice that has not started yet.
    observed_speaking: bool,
    /// Engine offset inside the current chunk as last reported by the tick.
    last_offset: u32,
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
            observed_speaking: false,
            last_offset: 0,
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
        self.observed_speaking = false;
        self.last_offset = 0;
        self.finished = false;
    }

    pub fn stop(&mut self) {
        self.job = None;
        self.status = PlaybackStatus::Idle;
        self.current_chunk = 0;
        self.completed_chars = 0;
        self.awaiting_engine = false;
        self.observed_speaking = false;
        self.last_offset = 0;
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

    /// True once playback has been paused. A paused voice will not report
    /// progress, so the tick skips it rather than spending a COM call per
    /// interval for as long as the user leaves it alone.
    pub fn is_paused(&self) -> bool {
        self.status == PlaybackStatus::Paused
    }

    /// True when the engine has been seen speaking during the current chunk.
    /// Until it has, silence carries no information: the queue call is
    /// asynchronous and the engine may simply not have started yet.
    pub fn has_spoken(&self) -> bool {
        self.observed_speaking
    }

    /// Record what the engine reported for the current chunk, so silence can
    /// later be read as completion and on-demand snapshots do not have to ask
    /// the engine themselves.
    pub fn observe_progress(&mut self, running: bool, offset_in_chunk: u32) {
        self.last_offset = self.clamp_offset(offset_in_chunk);
        if running {
            self.observed_speaking = true;
        }
    }

    /// The engine's offset inside the current chunk as last reported by the
    /// tick. Reading the engine from here instead would make every on-demand
    /// snapshot a COM round trip competing with the tick for the same voice.
    pub fn last_offset(&self) -> u32 {
        self.last_offset
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
        // The next chunk has not started yet, so silence proves nothing about
        // it until the engine is seen running again.
        self.observed_speaking = false;
        self.last_offset = 0;

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
                start_char: job.start_char,
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
                start_char: 0,
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
        PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", text, None)
    }

    fn book_job(book_id: &str, chapter_index: usize, text: &str) -> PlaybackJob {
        book_job_in(book_id, chapter_index, text, None)
    }

    fn book_job_in(
        book_id: &str,
        chapter_index: usize,
        text: &str,
        range: Option<TextRange>,
    ) -> PlaybackJob {
        PlaybackJob::new(
            PlaybackSource::Book {
                book_id: book_id.to_string(),
                chapter_index,
                total_chapters: 7,
            },
            format!("Book - Chapter {}", chapter_index + 1),
            text,
            range,
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

    /// Readers must never be writers. `snapshot`, `spoken_chars`,
    /// `total_chars` and `chunk_count` all take `&self`, and this test pins
    /// that guarantee: hammering the read API must leave the state, including
    /// the chunk cursor, byte-for-byte where it was. The old
    /// `get_speech_position` advanced chunks as a side effect, which made the
    /// frontend poll part of the control flow.
    #[test]
    fn the_read_api_is_pure() {
        let mut state = PlaybackState::new();
        // Long enough to span several chunks, so the chunk cursor is observable.
        state.start(clipboard_job(&"a".repeat(5000)));
        assert!(state.chunk_count() > 1);
        let _ = state.take_next_chunk();

        let before = state.snapshot(0);
        for offset in [0u32, 1, 7, 0, 10_000, 3] {
            let _ = state.spoken_chars(offset);
            let _ = state.total_chars();
            let _ = state.chunk_count();
            let _ = state.is_awaiting_engine();
            let _ = state.is_idle();
        }
        // Many reads later, a read at the same offset is still identical.
        let after = state.snapshot(0);
        assert_eq!(after.spoken_chars, before.spoken_chars);
        assert_eq!(after.finished, before.finished);
        assert_eq!(after.status, before.status); // The chunk cursor is still exactly where the writer left it.
        assert!(
            state.advance_chunk(),
            "the next chunk is the one after read"
        );
    }

    /// The frontend's typed IPC layer decodes these shapes. Pinning them here
    /// means a rename cannot silently break the webview at runtime.
    #[test]
    fn the_wire_shape_of_snapshots_is_stable() {
        let mut state = PlaybackState::new();
        state.start(book_job("book-1", 2, "hello world"));

        let value = serde_json::to_value(state.snapshot(0)).expect("serialize snapshot");

        assert_eq!(value["status"], "playing");
        assert_eq!(value["source"]["kind"], "book");
        assert_eq!(value["source"]["book_id"], "book-1");
        assert_eq!(value["source"]["chapter_index"], 2);
        assert_eq!(value["total_chars"], 11);
        assert_eq!(value["spoken_chars"], 0);
        assert_eq!(value["start_char"], 0);
        assert!(value["finished"].is_boolean());

        let idle = PlaybackState::new();
        let value = serde_json::to_value(idle.snapshot(0)).expect("serialize idle snapshot");
        assert_eq!(value["status"], "idle");
        assert!(value["source"].is_null());
        assert_eq!(value["start_char"], 0);
    }

    fn range(start: usize, end: Option<usize>, align: bool) -> Option<TextRange> {
        Some(TextRange {
            start,
            end,
            align_to_sentence: align,
        })
    }

    fn spoken(job: &PlaybackJob) -> String {
        job.chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_range_job_chunks_only_the_selected_text() {
        // "First sentence." is characters 0..=14, so "Second sentence." is 16..=31.
        let text = "First sentence. Second sentence. Third sentence.";
        let job = book_job_in("book-1", 0, text, range(16, Some(32), false));

        assert_eq!(spoken(&job), "Second sentence.");
        assert_eq!(job.total_chars(), 16);
    }

    #[test]
    fn a_range_job_reports_its_absolute_start_offset() {
        let text = "First sentence. Second sentence. Third sentence.";
        let job = book_job_in("book-1", 0, text, range(16, Some(32), false));

        assert_eq!(job.start_char, 16);
        // Chunk offsets are rebased onto the chapter, not onto the slice, so the
        // frontend can compare them with a position in the text on screen.
        assert_eq!(job.chunks[0].start_char, 16);
    }

    #[test]
    fn a_click_range_runs_to_the_end_of_the_chapter_from_a_sentence_boundary() {
        // "Third sentence." opens at character 33.
        let text = "First sentence. Second sentence. Third sentence.";
        // Character 20 is inside "Second sentence.", so reading begins at the
        // sentence after it and runs to the end of the chapter.
        let job = book_job_in("book-1", 0, text, range(20, None, true));

        assert_eq!(job.start_char, 33);
        assert_eq!(spoken(&job), "Third sentence.");
    }

    #[test]
    fn an_empty_range_produces_a_job_that_finishes_immediately() {
        let text = "Some text that exists.";
        let mut state = PlaybackState::new();
        state.start(book_job_in("book-1", 0, text, range(5, Some(5), false)));

        // Nothing to read is not a failure: the reader must not refuse to speak
        // because a stale selection collapsed.
        assert_eq!(state.snapshot(0).total_chars, 0);
        assert!(state.take_next_chunk().is_none());
        assert!(state.snapshot(0).finished);
    }

    #[test]
    fn a_range_starting_past_the_end_of_the_text_reads_nothing_rather_than_failing() {
        // A stale offset — a book re-imported shorter than the saved position —
        // clamps to the end of the text, which is an empty span. That is a job
        // with nothing to say, not an error the reader has to dismiss.
        let text = "Only one short sentence.";
        let mut state = PlaybackState::new();
        state.start(book_job_in("book-1", 0, text, range(5_000, None, true)));

        assert_eq!(state.snapshot(0).start_char, text::char_count(text));
        assert_eq!(state.snapshot(0).total_chars, 0);
        assert!(state.take_next_chunk().is_none());
    }

    #[test]
    fn a_range_ending_past_the_text_reads_to_the_end_rather_than_failing() {
        // "Second sentence." opens at 16, and the end is clamped from 9_000 to
        // the end of the text, so the sentence is read in full.
        let text = "First sentence. Second sentence.";
        let job = book_job_in("book-1", 0, text, range(16, Some(9_000), false));

        assert_eq!(spoken(&job), "Second sentence.");
        assert_eq!(job.range, Some(ResolvedRange { start: 16, end: 32 }));
    }

    #[test]
    fn a_range_reading_backwards_is_normalised_to_an_empty_span() {
        let text = "Some text that exists.";
        let mut state = PlaybackState::new();
        // An end before the start cannot be honoured; it must not panic or wrap.
        state.start(book_job_in("book-1", 0, text, range(20, Some(5), false)));

        assert_eq!(state.snapshot(0).total_chars, 0);
    }

    #[test]
    fn a_range_job_previews_the_text_it_reads_rather_than_the_whole_chapter() {
        // The overlay shows `text_preview`; previewing the chapter start while
        // reading chapter three would show the reader the wrong words.
        let text = format!("First chapter text. {}", "Tail sentence. ".repeat(200));
        let job = book_job_in("book-1", 0, &text, range(20, Some(35), false));

        assert!(job.text_preview.starts_with("Tail sentence."));
        assert!(!job.text_preview.starts_with("First chapter text."));
    }

    #[test]
    fn a_whole_text_job_is_unaffected_by_the_range_grammar() {
        let text = "First sentence. Second sentence.";
        let job = book_job("book-1", 0, text);

        assert_eq!(job.start_char, 0);
        assert_eq!(job.range, None);
        assert_eq!(spoken(&job), "First sentence. Second sentence.");
    }

    #[test]
    fn a_range_of_multi_byte_text_never_splits_a_character() {
        // Byte-slicing at a character offset is the historical bug this file's
        // sibling tests guard against; a range must be immune to it.
        let text = "Emoji 👍 lead in. Second sentence with 👍 more.";
        let length = text::char_count(text);
        for start in 0..=length {
            let job = book_job_in("book-1", 0, text, range(start, None, true));
            for chunk in &job.chunks {
                let byte = text::byte_offset_of_char(text, chunk.start_char);
                assert!(
                    text.is_char_boundary(byte),
                    "range from {start} produced a non-boundary chunk start {}",
                    chunk.start_char
                );
            }
        }
    }
}
