//! Tauri commands for the Inflect TTS engine.
//!
//! These are thin adapters over the playback actor. They validate nothing and
//! own nothing; they just forward to the actor.

use crate::error::AppResult;
use crate::inflect::model::InflectModel;
use crate::playback::actor::dispatch;
use crate::playback::state::PlaybackStatus;
use crate::playback::PlaybackHandle;
use std::sync::RwLock;
use tauri::State;

pub type ModelState = RwLock<InflectModel>;

/// Which Inflect model is currently active and whether it is ready.
#[tauri::command]
pub async fn tts_model_info(
    playback: State<'_, PlaybackHandle>,
    model: State<'_, ModelState>,
) -> AppResult<crate::inflect::commands::ModelInfo> {
    let current = model.read().map(|m| *m).unwrap_or(InflectModel::Micro);
    let idle = dispatch(playback.sender(), |reply| {
        crate::playback::actor::Command::Snapshot(reply)
    })
    .await
    .map(|s| s.status == PlaybackStatus::Idle)
    .unwrap_or(false);
    Ok(crate::inflect::commands::ModelInfo {
        current: current.as_str().to_string(),
        ready: true,
        can_switch: idle,
    })
}

/// Switch the active Inflect model. Only allowed when nothing is playing.
#[tauri::command]
pub async fn tts_switch_model(
    model: String,
    playback: State<'_, PlaybackHandle>,
    current: State<'_, ModelState>,
) -> AppResult<()> {
    let next = match model.as_str() {
        "micro" => InflectModel::Micro,
        "nano" => InflectModel::Nano,
        other => {
            return Err(crate::error::AppError::invalid_input(format!(
                "unknown Inflect model: {other}"
            )))
        }
    };
    if next == *current.read().unwrap() {
        return Ok(());
    }
    dispatch(playback.sender(), move |reply| {
        crate::playback::actor::Command::SwitchModel(next, reply)
    })
    .await?;
    *current.write().unwrap() = next;
    Ok(())
}

#[derive(Clone, serde::Serialize)]
pub struct ModelInfo {
    pub current: String,
    pub ready: bool,
    pub can_switch: bool,
}
