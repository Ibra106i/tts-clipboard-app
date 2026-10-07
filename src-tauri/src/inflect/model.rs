//! Inflect v2 model cache and first-use download.
//!
//! Models are cached under the app's data directory as
//! `inflect/models/{micro,nano}`. The first time a slot is used it is downloaded
//! from Hugging Face if missing, otherwise it is reused.
//!
//! This deliberately mirrors the Inflect project's own recommendation:
//! `hf download owensong/Inflect-Micro-v2 --local-dir ...` and then run
//! inference from that directory. We do the download ourselves so the app can
//! report progress and fail with a structured error instead of handing the user
//! a raw CLI instruction.

use crate::error::{AppError, AppResult};
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InflectModel {
    Micro,
    Nano,
}

impl InflectModel {
    pub fn hf_repo(self) -> &'static str {
        match self {
            Self::Micro => "https://huggingface.co/owensong/Inflect-Micro-v2",
            Self::Nano => "https://huggingface.co/owensong/Inflect-Nano-v2",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Micro => "micro",
            Self::Nano => "nano",
        }
    }
}

impl std::fmt::Display for InflectModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which Inflect model the app is currently configured to use.
pub type ModelSlot = InflectModel;

/// The cached model directory for a slot, plus whether it is present on disk.
pub struct ModelCache {
    base: PathBuf,
}

impl ModelCache {
    pub fn new(base: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&base);
        Self { base }
    }

    /// Path to the cached model directory for `model`.
    pub fn slot_path(&self, model: InflectModel) -> PathBuf {
        self.base.join(model.as_str())
    }

    /// Ensure the model for `model` is present on disk, downloading it first if
    /// needed. Returns the path the inference wrapper should use.
    pub fn ensure(&self, model: InflectModel) -> AppResult<PathBuf> {
        let path = self.slot_path(model);
        if self.is_complete(&path) {
            return Ok(path);
        }
        ModelCache::download_model(model, &path)?;
        if !self.is_complete(&path) {
            return Err(AppError::playback(format!(
                "the Inflect {model} model download did not produce a usable model directory"
            )));
        }
        log::info!("Inflect {model} model ready at {path:?}");
        Ok(path)
    }

    /// True when the slot looks like a complete Inflect checkout.
    fn is_complete(&self, path: &PathBuf) -> bool {
        path.is_dir()
            && path.join("inference.py").is_file()
            && path.join("requirements.txt").exists()
    }

    /// Download `model` from Hugging Face into `path`.
    fn download_model(model: InflectModel, path: &PathBuf) -> AppResult<()> {
        let repo = model.hf_repo();
        log::info!("downloading Inflect {model} model from Hugging Face ({repo})");

        // Remove any partial cache so we get a clean checkout.
        let _ = std::fs::remove_dir_all(path);

        // Use git to get the full repo (inference.py + weights), not just LFS files.
        // git clone creates `path` itself, so we must not create it first.
        let status = std::process::Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--branch")
            .arg("main")
            .arg(repo)
            .arg(path)
            .status()
            .map_err(|e| {
                AppError::playback(format!(
                    "cannot start the git clone for Inflect {model} ({e})"
                ))
            })?;

        if !status.success() {
            return Err(AppError::playback(format!(
                "the Hugging Face download for Inflect {model} did not complete"
            )));
        }
        Ok(())
    }
}

/// A thread-safe handle to the model cache, so the actor and the commands can
/// share one cache without locking the whole playback subsystem.
pub struct ModelCacheHandle {
    inner: std::sync::RwLock<ModelCache>,
}

impl ModelCacheHandle {
    pub fn new(base: PathBuf) -> Self {
        Self {
            inner: std::sync::RwLock::new(ModelCache::new(base)),
        }
    }

    pub fn ensure(&self, model: InflectModel) -> AppResult<PathBuf> {
        let cache = self.inner.read().map_err(|e| {
            AppError::internal(format!("the model cache is unavailable ({e})"))
        })?;
        cache.ensure(model)
    }

    pub fn slot_path(&self, model: InflectModel) -> PathBuf {
        self.inner.read().map_err(|e| {
            AppError::internal(format!("the model cache is unavailable ({e})"))
        }).unwrap().slot_path(model)
    }

    pub fn base(&self) -> PathBuf {
        self.inner.read().map_err(|e| {
            AppError::internal(format!("the model cache is unavailable ({e})"))
        }).unwrap().base.clone()
    }
}

/// Build the default model cache location under the app's data directory.
pub fn model_cache_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::storage("locate", format!("app data directory ({e})")))
        .map(|dir| dir.join("inflect").join("models"))
}
