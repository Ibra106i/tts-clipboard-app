use crate::error::{AppError, AppResult};
use std::sync::Mutex;
use tauri::{Emitter, State};
use windows::Win32::Media::Speech::*;
use windows::Win32::System::Com::*;

const CHUNK_SIZE: usize = 2000;

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

pub struct TtsState {
    pub voice: Mutex<Option<ISpVoice>>,
    pub mode: Mutex<TtsMode>,
    pub chunks: Mutex<Vec<String>>,
    pub current_chunk: Mutex<usize>,
    pub total_chars_spoken: Mutex<u32>,
}

unsafe impl Send for TtsState {}
unsafe impl Sync for TtsState {}

impl TtsState {
    pub fn new() -> Self {
        Self {
            voice: Mutex::new(None),
            mode: Mutex::new(TtsMode::Idle),
            chunks: Mutex::new(Vec::new()),
            current_chunk: Mutex::new(0),
            total_chars_spoken: Mutex::new(0),
        }
    }
}

pub fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub fn init_voice() -> windows::core::Result<ISpVoice> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;

        // Try to prefer a OneCore voice over the default SAPI voice
        match try_select_onecore_voice(&voice) {
            Ok(()) => {
                log::info!("[tts] Using OneCore voice");
            }
            Err(e) => {
                log::error!("[tts] OneCore voice unavailable, using default: {e}");
            }
        }

        Ok(voice)
    }
}

fn try_select_onecore_voice(voice: &ISpVoice) -> windows::core::Result<()> {
    unsafe {
        let category: ISpObjectTokenCategory =
            CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)?;

        let onecore_path = windows::core::HSTRING::from(
            "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech_OneCore\\Voices",
        );
        category.SetId(&onecore_path, false)?;

        let enumerator = category.EnumTokens(None, None)?;

        let mut token: Option<ISpObjectToken> = None;
        enumerator.Next(1, &mut token, None)?;

        let token = token.ok_or_else(windows::core::Error::from_win32)?;
        voice.SetVoice(&token)?;

        Ok(())
    }
}

pub fn read_clipboard() -> AppResult<String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| AppError::playback(format!("clipboard unavailable ({e})")))?;
    clipboard
        .get_text()
        .map_err(|e| AppError::playback(format!("the clipboard could not be read ({e})")))
}

fn get_voice_status(voice: &ISpVoice) -> AppResult<SPVOICESTATUS> {
    unsafe {
        let mut status = SPVOICESTATUS::default();
        let mut bookmark = windows::core::PWSTR::null();
        voice
            .GetStatus(&mut status, &mut bookmark)
            .map_err(|e| AppError::playback(format!("speech status unavailable ({e})")))?;
        Ok(status)
    }
}

fn chunk_text(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_SIZE {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= CHUNK_SIZE {
            chunks.push(remaining.to_string());
            break;
        }

        let cut_at = remaining[..CHUNK_SIZE]
            .rfind(['.', '!', '?'])
            .map(|i| i + 1)
            .unwrap_or_else(|| remaining[..CHUNK_SIZE].rfind(' ').unwrap_or(CHUNK_SIZE));

        chunks.push(remaining[..cut_at].to_string());
        remaining = remaining[cut_at..].trim_start();
    }

    chunks
}

fn speak_next_chunk(state: &TtsState) -> AppResult<bool> {
    let chunks = state
        .chunks
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let mut idx = state
        .current_chunk
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;

    if *idx >= chunks.len() {
        return Ok(false); // No more chunks
    }

    let text = chunks[*idx].clone();
    *idx += 1;

    let voice_guard = state
        .voice
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let voice = voice_guard
        .as_ref()
        .ok_or_else(|| AppError::playback("the speech voice is not available"))?;
    let htext = windows::core::HSTRING::from(&text);
    unsafe {
        voice
            .Speak(&htext, SPF_ASYNC.0 as u32, Some(std::ptr::null_mut()))
            .map_err(|e| AppError::playback(format!("speech could not start ({e})")))?;
    }
    Ok(true)
}

pub fn get_total_chars(state: &TtsState) -> usize {
    let chunks = state.chunks.lock().unwrap_or_else(|e| e.into_inner());
    chunks.iter().map(|c| c.len()).sum()
}

pub fn get_chars_before_current_chunk(state: &TtsState) -> u32 {
    let chunks = state.chunks.lock().unwrap_or_else(|e| e.into_inner());
    let idx = state
        .current_chunk
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    chunks.iter().take(*idx).map(|c| c.len() as u32).sum()
}

/// Speak text as clipboard (single chunk, no auto-advance)
#[tauri::command]
pub fn speak_text(text: String, state: State<'_, TtsState>) -> AppResult<()> {
    // Check if something is already playing
    {
        let mode = state
            .inner()
            .mode
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        if *mode != TtsMode::Idle {
            return Err(AppError::Busy);
        }
    }

    // Set mode to clipboard
    {
        let mut mode = state
            .inner()
            .mode
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        *mode = TtsMode::PlayingClipboard;
    }

    let chunks = chunk_text(&text);
    {
        let mut state_chunks = state
            .inner()
            .chunks
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        *state_chunks = chunks;
    }
    {
        let mut idx = state
            .inner()
            .current_chunk
            .lock()
            .map_err(|e| e.to_string())?;
        *idx = 0;
    }
    {
        let mut spoken = state
            .inner()
            .total_chars_spoken
            .lock()
            .map_err(|e| e.to_string())?;
        *spoken = 0;
    }

    speak_next_chunk(state.inner())?;
    Ok(())
}

/// Speak a book chapter (chunked, with auto-advance)
#[tauri::command]
pub fn speak_book_chapter(
    text: String,
    book_id: String,
    chapter_index: usize,
    total_chapters: usize,
    state: State<'_, TtsState>,
    app: tauri::AppHandle,
) -> AppResult<()> {
    // Check previous mode and emit interruption event if something was playing
    let previous_mode = {
        let mode = state
            .inner()
            .mode
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        mode.clone()
    };

    if previous_mode != TtsMode::Idle {
        let mut payload = serde_json::Map::new();
        match &previous_mode {
            TtsMode::Idle => {}
            TtsMode::PlayingClipboard => {
                payload.insert(
                    "previous_mode".to_string(),
                    serde_json::Value::String("clipboard".to_string()),
                );
            }
            TtsMode::PlayingBook {
                book_id,
                chapter_index,
                ..
            } => {
                payload.insert(
                    "previous_mode".to_string(),
                    serde_json::Value::String("book".to_string()),
                );
                payload.insert(
                    "book_id".to_string(),
                    serde_json::Value::String(book_id.clone()),
                );
                payload.insert(
                    "chapter_index".to_string(),
                    serde_json::Value::Number((*chapter_index).into()),
                );
            }
        }
        let _ = app.emit("tts-interrupted", serde_json::Value::Object(payload));
    }

    // Stop any current playback
    {
        let voice_guard = state
            .inner()
            .voice
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        if let Some(voice) = voice_guard.as_ref() {
            unsafe {
                let _ = voice.Speak(
                    &windows::core::HSTRING::default(),
                    0,
                    Some(std::ptr::null_mut()),
                );
            }
        }
    }

    // Set mode to book
    {
        let mut mode = state
            .inner()
            .mode
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        *mode = TtsMode::PlayingBook {
            book_id,
            chapter_index,
            total_chapters,
        };
    }

    let chunks = chunk_text(&text);
    {
        let mut state_chunks = state
            .inner()
            .chunks
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        *state_chunks = chunks;
    }
    {
        let mut idx = state
            .inner()
            .current_chunk
            .lock()
            .map_err(|e| e.to_string())?;
        *idx = 0;
    }
    {
        let mut spoken = state
            .inner()
            .total_chars_spoken
            .lock()
            .map_err(|e| e.to_string())?;
        *spoken = 0;
    }

    speak_next_chunk(state.inner())?;
    Ok(())
}

#[tauri::command]
pub fn pause_resume_tts(state: State<'_, TtsState>) -> AppResult<bool> {
    let tts = state.inner();
    let voice_guard = tts
        .voice
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let voice = voice_guard
        .as_ref()
        .ok_or_else(|| AppError::playback("the speech voice is not available"))?;
    let status = get_voice_status(voice)?;
    if status.dwRunningState == SPAS_PAUSE.0 as u32 {
        unsafe {
            voice
                .Resume()
                .map_err(|e| AppError::playback(format!("speech could not resume ({e})")))?;
        }
        Ok(false)
    } else {
        unsafe {
            voice
                .Pause()
                .map_err(|e| AppError::playback(format!("speech could not pause ({e})")))?;
        }
        Ok(true)
    }
}

#[tauri::command]
pub fn set_tts_rate(rate: f32, state: State<'_, TtsState>) -> AppResult<()> {
    let tts = state.inner();
    let voice_guard = tts
        .voice
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let voice = voice_guard
        .as_ref()
        .ok_or_else(|| AppError::playback("the speech voice is not available"))?;
    let sapi_rate = ((rate - 1.0) * 13.333) as i32;
    let sapi_rate = sapi_rate.clamp(-10, 10);
    unsafe {
        voice
            .SetRate(sapi_rate)
            .map_err(|e| AppError::playback(format!("speech rate could not be changed ({e})")))?;
    }
    Ok(())
}

/// Returns (current_pos, total_chars, mode_clone, is_chunk_done)
#[tauri::command]
pub fn get_speech_position(state: State<'_, TtsState>) -> AppResult<(u32, u32, String, bool)> {
    let tts = state.inner();
    let voice_guard = tts
        .voice
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let voice = voice_guard
        .as_ref()
        .ok_or_else(|| AppError::playback("the speech voice is not available"))?;
    let status = get_voice_status(voice)?;

    let mode = tts
        .mode
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    let mode_str = match &*mode {
        TtsMode::Idle => "idle".to_string(),
        TtsMode::PlayingClipboard => "clipboard".to_string(),
        TtsMode::PlayingBook {
            book_id,
            chapter_index,
            ..
        } => format!("book:{book_id}:{chapter_index}"),
    };

    let total = get_total_chars(tts) as u32;
    let before = get_chars_before_current_chunk(tts);
    let current = before + status.ulInputWordPos;

    // Check if current chunk is done (SAPI finished speaking)
    let is_done = status.dwRunningState == 0 && total > 0 && current >= total;

    // If chunk is done and we have more chunks, speak next
    if is_done {
        let idx = tts
            .current_chunk
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        let chunks = tts
            .chunks
            .lock()
            .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
        let has_more = *idx < chunks.len();
        drop(idx);
        drop(chunks);

        if has_more {
            drop(voice_guard);
            drop(mode);
            speak_next_chunk(tts)?;
            // Re-read position after speaking next chunk
            let voice_guard2 = tts
                .voice
                .lock()
                .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
            let voice2 = voice_guard2
                .as_ref()
                .ok_or_else(|| AppError::playback("the speech voice is not available"))?;
            let status2 = get_voice_status(voice2)?;
            let before2 = get_chars_before_current_chunk(tts);
            return Ok((before2 + status2.ulInputWordPos, total, mode_str, false));
        }
    }

    let chunk_done = is_done;
    Ok((current, total, mode_str, chunk_done))
}

#[tauri::command]
pub fn stop_tts(state: State<'_, TtsState>) -> AppResult<()> {
    let tts = state.inner();
    let voice_guard = tts
        .voice
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))?;
    if let Some(voice) = voice_guard.as_ref() {
        unsafe {
            let _ = voice.Speak(
                &windows::core::HSTRING::default(),
                0,
                Some(std::ptr::null_mut()),
            );
        }
    }
    drop(voice_guard);

    *tts.mode
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))? = TtsMode::Idle;
    *tts.chunks
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))? = Vec::new();
    *tts.current_chunk
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))? = 0;
    *tts.total_chars_spoken
        .lock()
        .map_err(|e| AppError::internal(format!("playback lock poisoned: {e}")))? = 0;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_a_single_chunk() {
        assert_eq!(chunk_text("Hello world."), vec!["Hello world.".to_string()]);
    }

    #[test]
    fn text_at_the_chunk_limit_is_not_split() {
        let text = "a".repeat(CHUNK_SIZE);
        assert_eq!(chunk_text(&text).len(), 1);
    }

    /// Non-whitespace content must survive chunking byte-for-byte, and chunk
    /// boundaries are allowed to drop only separator whitespace.
    fn non_whitespace(input: &str) -> String {
        input.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn long_ascii_text_is_split_without_losing_content() {
        let long = format!(
            "{} {}",
            "First sentence. ".repeat(200),
            "Second. ".repeat(200)
        );
        let chunks = chunk_text(&long);

        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        assert_eq!(non_whitespace(&chunks.concat()), non_whitespace(&long));
    }
}
