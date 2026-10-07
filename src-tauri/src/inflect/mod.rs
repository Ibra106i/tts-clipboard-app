//! Local Inflect v2 integration.
//!
//! The app drives a small Python wrapper (src-tauri/inflect/inflect_speak.py)
//! as a short-lived subprocess per chunk. The wrapper uses the published
//! InflectTTS Python API and writes a WAV; the Rust side plays that WAV and
//! advances playback.
//!
//! Nothing here owns the playback loop. It is a chunk synthesizer + WAV player,
//! not a streaming voice.

pub mod commands;
pub mod model;
pub mod play;
pub mod python;
pub mod wav;
pub mod wrapper;

pub use commands::ModelInfo;
pub use model::{model_cache_dir, InflectModel, ModelCache, ModelSlot};
pub use play::play_wav;
pub use wrapper::WrapperErrorKind;

use crate::error::AppResult;
use crate::inflect::model::ModelCacheHandle;
use crate::playback::engine::TtsEngine;
use crate::playback::inflect_engine::InflectEngine;
use std::sync::Arc;

/// Factory that builds the Inflect engine on the playback thread.
pub fn engine_factory(
    cache: Arc<ModelCacheHandle>,
) -> impl FnOnce(&ModelCacheHandle) -> AppResult<Box<dyn TtsEngine>> + Send + 'static {
    let staging_base = cache.base().join("staging");
    std::fs::create_dir_all(&staging_base).ok();
    move |_cache_ref| {
        Ok(Box::new(InflectEngine::new(
            InflectModel::Micro,
            cache.clone(),
            staging_base.clone(),
        )?))
    }
}
