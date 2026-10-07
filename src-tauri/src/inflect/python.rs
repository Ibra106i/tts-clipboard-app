//! Subprocess bridge to the local Inflect wrapper script.
//!
//! The wrapper is a short-lived Python process per chunk. The app locates it
//! relative to the running binary's bundle, so the same path works in dev and
//! in a shipped Tauri app.

use crate::error::{AppError, AppResult};
use crate::inflect::model::InflectModel;
use crate::inflect::wrapper::{Synthesis, WrapperErrorKind};
use crate::inflect::wav::WavHeader;
use std::path::PathBuf;
use std::process::Command;

/// Path to the Python wrapper shipped inside this crate's bundle.
pub fn wrapper_script() -> AppResult<PathBuf> {
    // In a Tauri app the Rust binary lives under src-tauri/target/...; the
    // wrapper is shipped next to it under src-tauri/inflect/. We resolve
    // relative to the executable so the same code works for `tauri dev` and
    // for the packaged app.
    let exe = std::env::current_exe().map_err(|e| {
        AppError::internal(format!("cannot locate the running executable ({e})"))
    })?;
    let exe_dir = exe.parent().ok_or_else(|| {
        AppError::internal("the executable has no parent directory".to_string())
    })?;
    // Walk up to the crate root that contains `inflect/inflect_speak.py`.
    // The wrapper is stored under src-tauri/inflect/ in the source tree; in a
    // packaged build it should be installed next to the executable by the
    // installer or by the Tauri build.
    let mut dir = exe_dir.to_path_buf();
    for _ in 0..4 {
        if (dir.join("inflect").join("inflect_speak.py")).is_file() {
            return Ok(dir.join("inflect").join("inflect_speak.py"));
        }
        dir = dir.parent()
            .ok_or_else(|| AppError::internal("cannot walk up to find inflect wrapper".to_string()))?
            .to_path_buf();
    }
    Err(AppError::internal(
        "the Inflect wrapper script was not found next to the app"
    ))
}

/// Locate a usable `python` on the current machine.
/// Prefer a Python that can import `torch`, since Inflect requires it.
pub fn find_python() -> AppResult<PathBuf> {
    // Ordered list of Python interpreters to try. The first one that can
    // import torch (needed by Inflect) wins.
    let candidates = [
        // Prefer explicit paths known to carry torch on this machine.
        r"C:\Users\Ibrahim106\AppData\Local\Programs\Python\Python314\python.exe",
        // Fall back to PATH-based discovery.
        "python",
        "python3",
        r"C:\Users\Ibrahim106\AppData\Local\Programs\Python\Python312\python.exe",
    ];
    for candidate in &candidates {
        let path = if candidate.contains('\\') || candidate.contains('/') {
            PathBuf::from(candidate)
        } else {
            // PATH-based lookup.
            match Command::new(candidate).arg("--version").output() {
                Ok(out) if out.status.success() => PathBuf::from(candidate),
                _ => continue,
            }
        };
        if python_can_import_torch(&path) {
            return Ok(path);
        }
    }
    Err(AppError::internal(
        "no `python` with torch found (Inflect requires PyTorch)"
    ))
}

fn python_can_import_torch(path: &PathBuf) -> bool {
    match Command::new(path).arg("-c").arg("import torch").output() {
        Ok(out) if out.status.success() => true,
        _ => false,
    }
}

/// Synthesize `text` with the local Inflect wrapper.
pub fn synthesize(
    python: &PathBuf,
    wrapper: &PathBuf,
    model_dir: &std::path::Path,
    model: &InflectModel,
    text: &str,
    speed: f32,
    variation: f32,
    seed: Option<i64>,
    wav_path: &std::path::Path,
) -> AppResult<Synthesis> {
    let mut cmd = Command::new(python.as_os_str());
    cmd.arg(wrapper);
    cmd.arg("--model-dir");
    cmd.arg(model_dir);
    cmd.arg("--model");
    cmd.arg(model.to_string());
    cmd.arg("--text");
    cmd.arg(text);
    cmd.arg("--speed");
    cmd.arg(speed.to_string());
    cmd.arg("--variation");
    cmd.arg(variation.to_string());
    if let Some(seed) = seed {
        cmd.arg("--seed");
        cmd.arg(seed.to_string());
    }
    cmd.arg("--output");
    cmd.arg(wav_path);

    let output = cmd
        .output()
        .map_err(|e| AppError::playback(format!("cannot start Inflect wrapper ({e})")))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail
            .strip_prefix("InflectTTS")
            .or_else(|| detail.strip_prefix("Cannot"))
            .unwrap_or(&detail)
            .trim();
        return Err(AppError::playback(format!(
            "Inflect synthesis failed: {detail}"
        ))
        .into());
    }

    parse_wav(wav_path)
}

/// Read back the WAV the wrapper produced and report duration.
fn parse_wav(path: &std::path::Path) -> AppResult<Synthesis> {
    let path = path.to_path_buf();
    let file = std::fs::File::open(&path).map_err(|e| {
        AppError::playback(format!(
            "Inflect wrote a WAV file but it could not be opened ({e})"
        ))
    })?;
    let header = WavHeader::read(&file).map_err(|e| {
        AppError::playback(format!("Inflect WAV is unreadable ({e})"))
    })?;
    let duration_ms = if header.sample_rate > 0 {
        (header.data_bytes as f64 / (header.sample_rate as f64 * header.bytes_per_sample as f64) * 1000.0).round() as u64
    } else {
        0
    };
    Ok(Synthesis {
        wav_path: path,
        sample_rate: header.sample_rate,
        duration_ms,
        chars: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn find_python_returns_a_real_interpreter_when_one_exists() {
        let python = find_python();
        assert!(python.is_ok(), "expected a python on PATH in this environment");
    }

    #[test]
    fn wrapper_script_path_ends_with_the_wrapper_name() {
        let script = wrapper_script();
        assert!(script.is_ok(), "tests need the wrapper shipped next to the binary");
        let script = script.unwrap();
        assert!(
            script.file_name().map(|n| n == "inflect_speak.py").unwrap_or(false),
            "unexpected wrapper path: {script:?}"
        );
    }
}
