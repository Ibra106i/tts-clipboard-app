//! The local TTS engine boundary.
//!
//! The actor no longer talks to SAPI. It talks to a [`TtsEngine`], which is a
//! wave-returning local synthesizer. That keeps the playback rules testable with
//! a fake engine and keeps the COM/SAPI details (which no longer exist in this
//! version) out of the actor.
//!
//! Progress is chunk-based: a chunk is synthesized to a WAV, played, and the
//! actor treats it as finished when its expected duration elapses or a stop
//! arrives. There is no character-offset polling anymore.

use crate::error::{AppError, AppResult};
use crate::inflect::model::{InflectModel, ModelCacheHandle};

/// Outcome of synthesizing and starting playback of one chunk.
#[derive(Clone, Debug)]
pub struct ChunkPlayback {
    /// Path to the synthesized WAV. The actor plays it and deletes it after the
    /// chunk completes.
    pub wav_path: std::path::PathBuf,
    /// Expected chunk duration in milliseconds, used to advance progress.
    pub duration_ms: u64,
    /// Characters the chunk held, for progress accounting.
    pub chars: usize,
}

/// A local TTS engine.
pub trait TtsEngine: Send {
    /// Begin playback of a synthesized chunk.
    fn play_chunk(&mut self, text: &str, speed: f32, variation: f32, seed: Option<i64>) -> AppResult<ChunkPlayback>;

    /// Stop any in-progress chunk playback.
    fn stop_playing(&mut self) -> AppResult<()>;

    /// Model currently in use.
    fn model(&self) -> InflectModel;

    /// Switch the active model. Only called when playback is idle.
    fn switch_model(&mut self, model: InflectModel, cache: &ModelCacheHandle) -> AppResult<()> {
        Err(AppError::internal("the active engine does not support switching models"))
    }
}

/// Convert a user-facing multiplier into an Inflect "speed" value.
pub fn speed_to_inflect(rate: f32) -> f32 {
    rate.clamp(0.5, 4.0)
}
