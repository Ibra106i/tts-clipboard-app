use std::sync::Mutex;
use tauri::State;
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
        Ok(voice)
    }
}

pub fn read_clipboard() -> Result<String, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Clipboard init failed: {e}"))?;
    clipboard
        .get_text()
        .map_err(|e| format!("Clipboard read failed: {e}"))
}

fn get_voice_status(voice: &ISpVoice) -> Result<SPVOICESTATUS, String> {
    unsafe {
        let mut status = SPVOICESTATUS::default();
        let mut bookmark = windows::core::PWSTR::null();
        voice
            .GetStatus(&mut status, &mut bookmark)
            .map_err(|e| format!("GetStatus failed: {e}"))?;
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
            .rfind(|c: char| c == '.' || c == '!' || c == '?')
            .map(|i| i + 1)
            .unwrap_or_else(|| remaining[..CHUNK_SIZE].rfind(' ').unwrap_or(CHUNK_SIZE));

        chunks.push(remaining[..cut_at].to_string());
        remaining = remaining[cut_at..].trim_start();
    }

    chunks
}

fn speak_next_chunk(state: &TtsState) -> Result<bool, String> {
    let chunks = state.chunks.lock().map_err(|e| e.to_string())?;
    let mut idx = state.current_chunk.lock().map_err(|e| e.to_string())?;

    if *idx >= chunks.len() {
        return Ok(false); // No more chunks
    }

    let text = chunks[*idx].clone();
    *idx += 1;

    let voice_guard = state.voice.lock().map_err(|e| e.to_string())?;
    let voice = voice_guard.as_ref().ok_or("TTS voice not initialized")?;
    let htext = windows::core::HSTRING::from(&text);
    unsafe {
        voice
            .Speak(&htext, SPF_ASYNC.0 as u32, Some(std::ptr::null_mut()))
            .map_err(|e| format!("Speak failed: {e}"))?;
    }
    Ok(true)
}

pub fn get_total_chars(state: &TtsState) -> usize {
    let chunks = state.chunks.lock().unwrap_or_else(|e| e.into_inner());
    chunks.iter().map(|c| c.len()).sum()
}

pub fn get_chars_before_current_chunk(state: &TtsState) -> u32 {
    let chunks = state.chunks.lock().unwrap_or_else(|e| e.into_inner());
    let idx = state.current_chunk.lock().unwrap_or_else(|e| e.into_inner());
    chunks.iter().take(*idx).map(|c| c.len() as u32).sum()
}

/// Speak text as clipboard (single chunk, no auto-advance)
#[tauri::command]
pub fn speak_text(text: String, state: State<'_, TtsState>) -> Result<(), String> {
    // Check if something is already playing
    {
        let mode = state.inner().mode.lock().map_err(|e| e.to_string())?;
        if *mode != TtsMode::Idle {
            return Err("busy".to_string());
        }
    }

    // Set mode to clipboard
    {
        let mut mode = state.inner().mode.lock().map_err(|e| e.to_string())?;
        *mode = TtsMode::PlayingClipboard;
    }

    let chunks = chunk_text(&text);
    {
        let mut state_chunks = state.inner().chunks.lock().map_err(|e| e.to_string())?;
        *state_chunks = chunks;
    }
    {
        let mut idx = state.inner().current_chunk.lock().map_err(|e| e.to_string())?;
        *idx = 0;
    }
    {
        let mut spoken = state.inner().total_chars_spoken.lock().map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
    // Stop any current playback
    {
        let voice_guard = state.inner().voice.lock().map_err(|e| e.to_string())?;
        if let Some(voice) = voice_guard.as_ref() {
            unsafe { let _ = voice.Speak(&windows::core::HSTRING::default(), 0, Some(std::ptr::null_mut())); }
        }
    }

    // Set mode to book
    {
        let mut mode = state.inner().mode.lock().map_err(|e| e.to_string())?;
        *mode = TtsMode::PlayingBook {
            book_id,
            chapter_index,
            total_chapters,
        };
    }

    let chunks = chunk_text(&text);
    {
        let mut state_chunks = state.inner().chunks.lock().map_err(|e| e.to_string())?;
        *state_chunks = chunks;
    }
    {
        let mut idx = state.inner().current_chunk.lock().map_err(|e| e.to_string())?;
        *idx = 0;
    }
    {
        let mut spoken = state.inner().total_chars_spoken.lock().map_err(|e| e.to_string())?;
        *spoken = 0;
    }

    speak_next_chunk(state.inner())?;
    Ok(())
}

#[tauri::command]
pub fn pause_resume_tts(state: State<'_, TtsState>) -> Result<bool, String> {
    let tts = state.inner();
    let voice_guard = tts.voice.lock().map_err(|e| e.to_string())?;
    let voice = voice_guard.as_ref().ok_or("TTS voice not initialized")?;
    let status = get_voice_status(voice)?;
    if status.dwRunningState == SPAS_PAUSE.0 as u32 {
        unsafe {
            voice.Resume().map_err(|e| format!("Resume failed: {e}"))?;
        }
        Ok(false)
    } else {
        unsafe {
            voice.Pause().map_err(|e| format!("Pause failed: {e}"))?;
        }
        Ok(true)
    }
}

#[tauri::command]
pub fn set_tts_rate(rate: f32, state: State<'_, TtsState>) -> Result<(), String> {
    let tts = state.inner();
    let voice_guard = tts.voice.lock().map_err(|e| e.to_string())?;
    let voice = voice_guard.as_ref().ok_or("TTS voice not initialized")?;
    let sapi_rate = ((rate - 1.0) * 13.333) as i32;
    let sapi_rate = sapi_rate.clamp(-10, 10);
    unsafe {
        voice
            .SetRate(sapi_rate)
            .map_err(|e| format!("SetRate failed: {e}"))?;
    }
    Ok(())
}

/// Returns (current_pos, total_chars, mode_clone, is_chunk_done)
#[tauri::command]
pub fn get_speech_position(state: State<'_, TtsState>) -> Result<(u32, u32, String, bool), String> {
    let tts = state.inner();
    let voice_guard = tts.voice.lock().map_err(|e| e.to_string())?;
    let voice = voice_guard.as_ref().ok_or("TTS voice not initialized")?;
    let status = get_voice_status(voice)?;

    let mode = tts.mode.lock().map_err(|e| e.to_string())?;
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
        let idx = tts.current_chunk.lock().map_err(|e| e.to_string())?;
        let chunks = tts.chunks.lock().map_err(|e| e.to_string())?;
        let has_more = *idx < chunks.len();
        drop(idx);
        drop(chunks);

        if has_more {
            drop(voice_guard);
            drop(mode);
            speak_next_chunk(tts)?;
            // Re-read position after speaking next chunk
            let voice_guard2 = tts.voice.lock().map_err(|e| e.to_string())?;
            let voice2 = voice_guard2.as_ref().ok_or("TTS voice not initialized")?;
            let status2 = get_voice_status(voice2)?;
            let before2 = get_chars_before_current_chunk(tts);
            return Ok((before2 + status2.ulInputWordPos, total, mode_str, false));
        }
    }

    let chunk_done = is_done;
    Ok((current, total, mode_str, chunk_done))
}

#[tauri::command]
pub fn stop_tts(state: State<'_, TtsState>) -> Result<(), String> {
    let tts = state.inner();
    let voice_guard = tts.voice.lock().map_err(|e| e.to_string())?;
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

    *tts.mode.lock().map_err(|e| e.to_string())? = TtsMode::Idle;
    *tts.chunks.lock().map_err(|e| e.to_string())? = Vec::new();
    *tts.current_chunk.lock().map_err(|e| e.to_string())? = 0;
    *tts.total_chars_spoken.lock().map_err(|e| e.to_string())? = 0;
    Ok(())
}
