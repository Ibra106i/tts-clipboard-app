//! Tauri commands for playback.
//!
//! These are thin, platform-agnostic adapters: they validate nothing, own
//! nothing, and forward to the actor. Every one of them is either a pure read
//! (`playback_get_state`) or an explicit request to change playback.
//!
//! All of them are `async`. Each one ends in a wait on the playback thread,
//! which can take as long as `REPLY_TIMEOUT` when the engine is slow to answer.
//! A plain `fn` command runs on the window's own thread, so every one of those
//! waits froze the entire UI - worst of all `speak_book_chapter`, which also
//! chunks the whole chapter before it even reaches the actor.
//!
//! `dispatch` builds the command on the caller's thread and moves only the wait
//! onto the blocking pool, so a runtime worker is never parked for the duration
//! and the expensive work happens on the playback thread, where the engine is.

use crate::error::AppResult;
use crate::models::TextRange;
use crate::playback::actor::{dispatch, PlaybackHandle};
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
pub async fn speak_text(text: String, playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    let job = clipboard_job(&text);
    dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::Start(job, reply)
    })
    .await
}

/// Read a chapter, or a span of it.
///
/// `range` is `None` for a whole-chapter read. When present it selects the span
/// of `text` to read: the reader sends one when the user clicks a paragraph or
/// selects a passage. Offsets are characters, and a span that no longer matches
/// the text is clamped rather than rejected.
#[tauri::command]
pub async fn speak_book_chapter(
    text: String,
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    range: Option<TextRange>,
    playback: State<'_, PlaybackHandle>,
) -> AppResult<()> {
    let job = chapter_job(book_id, chapter_index, total_chapters, &text, range);
    dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::Start(job, reply)
    })
    .await
}

#[tauri::command]
pub async fn pause_resume_tts(playback: State<'_, PlaybackHandle>) -> AppResult<bool> {
    dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::PauseResume(reply)
    })
    .await
}

#[tauri::command]
pub async fn set_tts_rate(rate: f32, playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    dispatch(playback.sender(), move |reply| {
        crate::playback::actor::Command::SetRate(rate, reply)
    })
    .await
}

#[tauri::command]
pub async fn stop_tts(playback: State<'_, PlaybackHandle>) -> AppResult<()> {
    dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::Stop(reply)
    })
    .await
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
pub async fn playback_get_state(
    playback: State<'_, PlaybackHandle>,
) -> AppResult<PlaybackSnapshot> {
    dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::Snapshot(reply)
    })
    .await
}
