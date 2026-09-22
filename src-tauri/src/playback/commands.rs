//! Tauri commands for playback.
//!
//! These are thin, platform-agnostic adapters: they validate nothing, own
//! nothing, and forward to the actor. Every one of them is either a pure read
//! (`playback_get_state`, `get_speech_position`) or an explicit request to
//! change playback.

use crate::error::AppResult;
use crate::playback::actor::PlaybackHandle;
use crate::playback::state::{PlaybackJob, PlaybackSnapshot, PlaybackSource};
use tauri::State;

fn clipboard_job(text: &str) -> PlaybackJob {
    PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", text)
}

fn chapter_job(
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    text: &str,
) -> PlaybackJob {
    let title = format!("Chapter {}", chapter_index + 1);
    PlaybackJob::new(
        PlaybackSource::Book {
            book_id,
            chapter_index,
            total_chapters,
        },
        title,
        text,
    )
}

#[tauri::command]
pub fn speak_text(text: String, playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    playback.start(clipboard_job(&text))
}

#[tauri::command]
pub fn speak_book_chapter(
    text: String,
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    playback: State<'_, PlaybackHandle>,
) -> AppResult<()> {
    playback.start(chapter_job(book_id, chapter_index, total_chapters, &text))
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
#[tauri::command]
pub fn playback_get_state(playback: State<'_, PlaybackHandle>) -> AppResult<PlaybackSnapshot> {
    playback.snapshot()
}

/// Legacy tuple shape kept for the existing frontend:
/// `(spoken_chars, total_chars, mode, finished)`.
///
/// Both counts are **characters**. The `mode` string exists only so the current
/// overlay can tell a chapter job from a clipboard job; it is derived from the
/// snapshot rather than encoded in playback state.
#[tauri::command]
pub fn get_speech_position(
    playback: State<'_, PlaybackHandle>,
) -> AppResult<(u32, u32, String, bool)> {
    let snapshot = playback.snapshot()?;
    let mode = match &snapshot.source {
        None => "idle".to_string(),
        Some(PlaybackSource::Clipboard) => "clipboard".to_string(),
        Some(PlaybackSource::Book {
            book_id,
            chapter_index,
            ..
        }) => format!("book:{book_id}:{chapter_index}"),
    };
    Ok((
        snapshot.spoken_chars,
        snapshot.total_chars,
        mode,
        snapshot.finished,
    ))
}
