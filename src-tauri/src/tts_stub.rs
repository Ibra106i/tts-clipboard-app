//! Non-Windows stand-in for the SAPI playback module.
//!
//! Speech synthesis here is Windows SAPI 5 through the `windows` crate, so on
//! any other platform this module is compiled instead. It keeps the same public
//! surface as the real implementation so the rest of the crate builds unchanged,
//! and it fails with a clear `unsupported_platform` error rather than a link
//! error or a mystery silence.

use crate::error::{AppError, AppResult};
use tauri::State;

const FEATURE: &str = "Text to speech";

/// Playback mode. Mirrors the Windows implementation so shared code type-checks.
#[derive(Clone, Debug, PartialEq)]
pub enum TtsMode {
    Idle,
    PlayingClipboard,
    PlayingBook {
        book_id: String,
        chapter_index: usize,
        total_chapters: usize,
    },
}

/// Held in Tauri's managed state. Carries no voice on this platform.
pub struct TtsState;

impl TtsState {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TtsState {
    fn default() -> Self {
        Self::new()
    }
}

fn unsupported() -> AppError {
    AppError::unsupported_platform(FEATURE)
}

/// COM initialisation is a Windows concern; this is a no-op elsewhere.
pub fn init_com() {}

pub fn init_voice(_state: &State<'_, TtsState>) -> AppResult<()> {
    Err(unsupported())
}

pub fn is_idle(_state: &State<'_, TtsState>) -> AppResult<bool> {
    Ok(true)
}

pub fn read_clipboard() -> AppResult<String> {
    Err(unsupported())
}

#[tauri::command]
pub fn speak_text(_text: String, _state: State<'_, TtsState>) -> AppResult<()> {
    Err(unsupported())
}

#[tauri::command]
pub fn speak_book_chapter(
    _text: String,
    _book_id: String,
    _chapter_index: usize,
    _total_chapters: usize,
    _state: State<'_, TtsState>,
    _app: tauri::AppHandle,
) -> AppResult<()> {
    Err(unsupported())
}

#[tauri::command]
pub fn pause_resume_tts(_state: State<'_, TtsState>) -> AppResult<bool> {
    Err(unsupported())
}

#[tauri::command]
pub fn set_tts_rate(_rate: f32, _state: State<'_, TtsState>) -> AppResult<()> {
    Err(unsupported())
}

#[tauri::command]
pub fn get_speech_position(_state: State<'_, TtsState>) -> AppResult<(u32, u32, String, bool)> {
    Err(unsupported())
}

#[tauri::command]
pub fn stop_tts(_state: State<'_, TtsState>) -> AppResult<()> {
    Err(unsupported())
}
