//! Playback: one owner, many readers.
//!
//! * [`state`] holds every fact about what is being spoken. It has no I/O and no
//!   platform dependencies, so the rules are unit-testable.
//! * [`actor`] is the only thread allowed to mutate that state, and the only
//!   thing that owns the local TTS engine.
//! * [`engine`] is the engine boundary: a local Inflect v2 wave synthesizer with
//!   a fake implementation for tests.
//! * [`commands`] is the Tauri surface, and is the same on every platform.

pub mod actor;
pub mod commands;
pub mod engine;
pub mod state;

pub mod inflect_engine;

pub use crate::inflect::model::ModelCacheHandle;
pub use actor::{PlaybackEvents, PlaybackHandle};
pub use engine::{ChunkPlayback, TtsEngine};
pub use state::{PlaybackJob, PlaybackSnapshot, PlaybackSource};

use crate::error::{AppError, AppResult};

/// Read the system clipboard as text.
///
/// Deliberately independent of the speech engine: the clipboard is read by the
/// hotkey handler on every platform so the failure is reported even where speech
/// is unavailable.
pub fn read_clipboard() -> AppResult<String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| AppError::playback(format!("clipboard unavailable ({e})")))?;
    clipboard
        .get_text()
        .map_err(|e| AppError::playback(format!("the clipboard could not be read ({e})")))
}

/// Build a shared model-cache handle from the app's app-data directory. Used by
/// tests and by the app setup path.
pub fn model_cache_for_app_dir(base: std::path::PathBuf) -> ModelCacheHandle {
    ModelCacheHandle::new(base)
}
