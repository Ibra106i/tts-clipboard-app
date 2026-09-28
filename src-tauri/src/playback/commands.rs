//! Tauri commands for playback.
//!
//! These are thin, platform-agnostic adapters: they validate nothing, own
//! nothing, and forward to the actor. Every one of them is either a pure read
//! (`playback_get_state`) or an explicit request to change playback.

use crate::error::AppResult;
use crate::models::TextRange;
use crate::playback::actor::PlaybackHandle;
use crate::playback::state::{PlaybackJob, PlaybackSnapshot, PlaybackSource};
use tauri::State;

fn clipboard_job(text: &str) -> PlaybackJob {
    PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", text, None)
}

fn chapter_job(
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    text: &str,
    range: Option<TextRange>,
) -> PlaybackJob {
    let title = if range.is_some() {
        format!("Chapter {} selection", chapter_index + 1)
    } else {
        format!("Chapter {}", chapter_index + 1)
    };
    PlaybackJob::new(
        PlaybackSource::Book {
            book_id,
            chapter_index,
            total_chapters,
        },
        title,
        text,
        range,
    )
}

#[tauri::command]
pub fn speak_text(text: String, playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    playback.start(clipboard_job(&text))
}

/// Read a chapter, or a span of it.
///
/// `range` is `None` for a whole-chapter read. When present it selects the span
/// of `text` to read: the reader sends one when the user clicks a paragraph or
/// selects a passage. Offsets are characters, and a span that no longer matches
/// the text is clamped rather than rejected.
#[tauri::command]
pub fn speak_book_chapter(
    text: String,
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    range: Option<TextRange>,
    playback: State<'_, PlaybackHandle>,
) -> AppResult<()> {
    playback.start(chapter_job(
        book_id,
        chapter_index,
        total_chapters,
        &text,
        range,
    ))
}

#[tauri::command]
pub fn pause_resume_tts(playback: State<'_, PlaybackHandle>) -> AppResult<bool> {
    playback.pause_resume()
}

#[tauri::command]
pub fn set_tts_rate(rate: f32, playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    playback.set_rate(rate)
}

#[tauri::command]
pub fn stop_tts(playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    playback.stop()
}

/// Current playback state. Purely observational.
///
/// This is the single playback read command. Progress is expressed as
/// `spoken_chars` out of `total_chars`, both in Unicode characters, so the UI
/// never has to reconcile bytes, UTF-16 code units and engine offsets.
///
/// The previous `get_speech_position` returned a tuple whose `mode` field
/// packed a book id and chapter into a string and whose reader advanced the
/// chunk cursor as a side effect. Both are gone; the discriminated `source`
/// carries the same information without parsing, and reading cannot mutate.
#[tauri::command]
pub fn playback_get_state(playback: State<'_, PlaybackHandle>) -> AppResult<PlaybackSnapshot> {
    playback.snapshot()
}
