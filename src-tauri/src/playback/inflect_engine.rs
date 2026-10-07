//! The local Inflect TTS engine.
//!
//! This replaces the SAPI-shaped `Speaker` trait with a contract that matches a
//! wave-returning synthesizer: synthesize a chunk, play it, and let the actor
//! track progress from chunk completion and audio duration.
//!
//! The actor still owns playback state and chunk progression. This module only
//! owns the model, the Python wrapper subprocess, and the WAV player.

use crate::error::{AppError, AppResult};
use crate::inflect::{
    model::{InflectModel, ModelCacheHandle},
    play::play_wav,
    python::{find_python, synthesize, wrapper_script},
    wrapper::Synthesis,
};
use crate::playback::engine::{ChunkPlayback, TtsEngine};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A local TTS engine that synthesizes chunks to WAV and plays them.
pub struct InflectEngine {
    /// Python interpreter used to run the wrapper.
    python: PathBuf,
    /// Path to the wrapper script shipped with the app.
    wrapper: PathBuf,
    /// Cached model directory currently in use.
    model_dir: PathBuf,
    /// Which model is currently loaded.
    model: InflectModel,
    /// Cache handle used to (re)download models.
    cache: Arc<ModelCacheHandle>,
    /// Temporary directory for chunk WAVs.
    staging: PathBuf,
}

impl InflectEngine {
    pub fn new(
        model: InflectModel,
        cache: Arc<ModelCacheHandle>,
        staging: PathBuf,
    ) -> AppResult<Self> {
        let python = find_python()?;
        let wrapper = wrapper_script()?;
        let model_dir = cache.ensure(model)?;
        Ok(Self {
            python,
            wrapper,
            model_dir,
            model,
            cache,
            staging,
        })
    }

    /// Switch to a different model when playback is idle.
    pub fn switch_model(&mut self, model: InflectModel, cache: &ModelCacheHandle) -> AppResult<()> {
        if model == self.model {
            return Ok(());
        }
        let model_dir = cache.ensure(model)?;
        self.model = model;
        self.model_dir = model_dir;
        Ok(())
    }

    /// Model currently in use.
    pub fn model(&self) -> InflectModel {
        self.model
    }

    /// Synthesize `text` to a WAV and play it, returning how long playback is
    /// expected to last.
    pub fn speak_chunk(&mut self, text: &str, speed: f32, variation: f32, seed: Option<i64>) -> AppResult<Synthesis> {
        let tmp = self.staging.join(format!("chunk_{}.wav", std::process::id()));
        let synthesis = synthesize(
            &self.python,
            &self.wrapper,
            &self.model_dir,
            &self.model,
            text,
            speed,
            variation,
            seed,
            &tmp,
        )?;
        // Play it asynchronously; the actor tracks completion by elapsed time.
        let duration_ms = play_wav(&synthesis.wav_path).map_err(|e| {
            AppError::playback(format!("the synthesized WAV could not be played ({e})"))
        })?;
        Ok(Synthesis {
            wav_path: synthesis.wav_path,
            sample_rate: synthesis.sample_rate,
            duration_ms,
            chars: text.chars().count(),
        })
    }

}

impl TtsEngine for InflectEngine {
    fn play_chunk(
        &mut self,
        text: &str,
        speed: f32,
        variation: f32,
        seed: Option<i64>,
    ) -> AppResult<ChunkPlayback> {
        let synthesis = self.speak_chunk(text, speed, variation, seed)?;
        Ok(ChunkPlayback {
            wav_path: synthesis.wav_path,
            duration_ms: synthesis.duration_ms,
            chars: synthesis.chars,
        })
    }

    fn stop_playing(&mut self) -> AppResult<()> {
        // The WAV player is asynchronous (PlaySoundW SND_ASYNC), so stopping
        // means telling the actor to move on. The actor handles cleanup.
        Ok(())
    }

    fn model(&self) -> InflectModel {
        self.model()
    }
}

impl Drop for InflectEngine {
    fn drop(&mut self) {
        // Clean up any leftover chunk WAVs on shutdown.
        let _ = std::fs::remove_dir_all(&self.staging);
    }
}
