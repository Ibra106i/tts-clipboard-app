//! Error model for the Inflect Python wrapper subprocess.

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum WrapperErrorKind {
    #[error("Python could not be started ({reason})")]
    PythonUnavailable { reason: String },

    #[error("Inflect wrapper failed: {detail}")]
    WrapperFailed { detail: String },

    #[error("Inflect wrapper did not write the expected WAV ({path})")]
    MissingWav { path: String },

    #[error("Inflect wrapper produced an unreadable WAV ({path}: {detail})")]
    BadWav { path: String, detail: String },
}

impl WrapperErrorKind {
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::WrapperFailed { .. } | Self::BadWav { .. })
    }
}

/// Outcome of a chunk synthesis request.
#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    /// Path to the synthesized WAV. Owned by the caller and played before
    /// being deleted.
    pub wav_path: std::path::PathBuf,
    /// Sample rate the wrapper reported, in Hz.
    pub sample_rate: u32,
    /// Approximate duration of the chunk in milliseconds, from the WAV.
    pub duration_ms: u64,
    /// Characters synthesized.
    pub chars: usize,
}
