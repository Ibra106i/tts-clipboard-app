//! Playback: one owner, many readers.
//!
//! * [`state`] holds every fact about what is being spoken. It has no I/O and no
//!   platform dependencies, so the rules are unit-testable.
//! * [`actor`] is the only thread allowed to mutate that state, and the only
//!   thing that owns the speech engine.
//! * [`speaker`] is the engine boundary: SAPI on Windows, an explicit
//!   unsupported-platform error elsewhere.
//! * [`commands`] is the Tauri surface, and is the same on every platform.

pub mod actor;
pub mod commands;
pub mod speaker;
pub mod state;

#[cfg(not(windows))]
mod speaker_stub;
#[cfg(windows)]
mod speaker_windows;

#[cfg(not(windows))]
pub use speaker_stub::speaker_factory;
#[cfg(windows)]
pub use speaker_windows::speaker_factory;

pub use actor::{PlaybackEvents, PlaybackHandle};
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
