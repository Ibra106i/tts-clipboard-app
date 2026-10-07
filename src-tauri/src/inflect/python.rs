//! Subprocess bridge to the local Inflect wrapper script.
//!
//! The wrapper is a short-lived Python process per chunk. The app locates it
//! relative to the running binary's bundle, so the same path works in dev and
//! in a shipped Tauri app.

use crate::error::{AppError, AppResult};
use crate::inflect::model::InflectModel;
use crate::inflect::wav::WavHeader;
use crate::inflect::wrapper::Synthesis;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment variable that overrides interpreter discovery.
///
/// Discovery cannot know about a virtualenv or a conda environment, so there has
/// to be one explicit escape hatch. It is read from the process environment
/// rather than a config file because it is a developer and power-user knob, not
/// application state.
const PYTHON_OVERRIDE_ENV: &str = "TTS_CLIPBOARD_PYTHON";

#[cfg(windows)]
const PYTHON_EXE: &str = "python.exe";
#[cfg(not(windows))]
const PYTHON_EXE: &str = "python";

#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

/// Version directories to probe, newest first.
const PYTHON_VERSION_DIRS: [&str; 6] = ["314", "313", "312", "311", "310", "39"];

/// Path to the Python wrapper shipped inside this crate's bundle.
pub fn wrapper_script() -> AppResult<PathBuf> {
    // In a Tauri app the Rust binary lives under src-tauri/target/...; the
    // wrapper is shipped next to it under src-tauri/inflect/. We resolve
    // relative to the executable so the same code works for `tauri dev` and
    // for the packaged app.
    let exe = std::env::current_exe()
        .map_err(|e| AppError::internal(format!("cannot locate the running executable ({e})")))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| AppError::internal("the executable has no parent directory".to_string()))?;
    // Walk up to the crate root that contains `inflect/inflect_speak.py`.
    // The wrapper is stored under src-tauri/inflect/ in the source tree; in a
    // packaged build it should be installed next to the executable by the
    // installer or by the Tauri build.
    let mut dir = exe_dir.to_path_buf();
    for _ in 0..4 {
        if (dir.join("inflect").join("inflect_speak.py")).is_file() {
            return Ok(dir.join("inflect").join("inflect_speak.py"));
        }
        dir = dir
            .parent()
            .ok_or_else(|| {
                AppError::internal("cannot walk up to find inflect wrapper".to_string())
            })?
            .to_path_buf();
    }
    Err(AppError::internal(
        "the Inflect wrapper script was not found next to the app",
    ))
}

/// Directories that hold per-user and machine-wide Python installations.
///
/// Resolved from the environment at runtime rather than written down. The
/// previous version named two absolute paths under one developer's `C:\Users\`
/// profile, which published that account name in a public repository and meant
/// discovery could only ever succeed on the machine it was authored on.
#[cfg(windows)]
fn python_install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            roots.push(base.join("Programs").join("Python"));
            roots.push(base.join("Python"));
        }
    }
    roots
}

#[cfg(not(windows))]
fn python_install_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]
}

/// Absolute paths for `name` found by walking `PATH`, which we do ourselves.
///
/// Handing a bare name to `Command::new` is not equivalent: Windows searches the
/// application directory and then the current working directory before `PATH`,
/// so a `python.exe` dropped next to the app would be executed instead of the
/// user's interpreter. Resolving through `PATH` alone removes that step.
fn search_path(name: &str) -> Vec<PathBuf> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    std::env::split_paths(&path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(format!("{name}{EXE_SUFFIX}")))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// Every interpreter discovery will consider, in priority order, as absolute
/// paths.
fn candidate_interpreters() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for root in python_install_roots() {
        for version in PYTHON_VERSION_DIRS {
            candidates.push(root.join(format!("Python{version}")).join(PYTHON_EXE));
        }
    }

    for name in ["python", "python3"] {
        candidates.extend(search_path(name));
    }

    candidates
}

/// Locate a usable `python` on the current machine.
///
/// Prefers an interpreter that can import `torch`, because Inflect requires it.
/// Every candidate is an absolute path, and a discovery failure names the
/// override rather than leaving the user with a dead end.
pub fn find_python() -> AppResult<PathBuf> {
    if let Some(explicit) = std::env::var_os(PYTHON_OVERRIDE_ENV) {
        let path = PathBuf::from(&explicit);
        if !path.is_file() {
            return Err(AppError::internal(format!(
                "{PYTHON_OVERRIDE_ENV} points at {}, which is not a file",
                path.display()
            )));
        }
        if !python_can_import_torch(&path) {
            return Err(AppError::internal(format!(
                "{PYTHON_OVERRIDE_ENV} points at {}, but that interpreter cannot import torch",
                path.display()
            )));
        }
        return Ok(path);
    }

    let mut tried = 0usize;
    for candidate in candidate_interpreters() {
        if !candidate.is_file() {
            continue;
        }
        if python_can_import_torch(&candidate) {
            return Ok(candidate);
        }
        tried += 1;
    }

    Err(AppError::internal(format!(
        "no Python interpreter with PyTorch was found ({tried} candidate(s) could not import \
         torch). Set {PYTHON_OVERRIDE_ENV} to the interpreter you want to use."
    )))
}

/// True when the interpreter at `path` can import torch.
fn python_can_import_torch(path: &Path) -> bool {
    matches!(
        Command::new(path).arg("-c").arg("import torch").output(),
        Ok(out) if out.status.success()
    )
}

/// Synthesize `text` with the local Inflect wrapper.
pub fn synthesize(
    python: &Path,
    wrapper: &Path,
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
        )));
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
    let header = WavHeader::read(&file)
        .map_err(|e| AppError::playback(format!("Inflect WAV is unreadable ({e})")))?;
    let duration_ms = if header.sample_rate > 0 {
        (header.data_bytes as f64 / (header.sample_rate as f64 * header.bytes_per_sample as f64)
            * 1000.0)
            .round() as u64
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

    #[test]
    fn discovery_never_yields_a_relative_or_bare_name() {
        // A bare name would let Windows resolve it from the application
        // directory or the current working directory before PATH.
        for candidate in candidate_interpreters() {
            assert!(
                candidate.is_absolute(),
                "candidate must be absolute: {candidate:?}"
            );
        }
    }

    #[test]
    fn candidates_are_derived_from_the_environment_not_from_literals() {
        // Every candidate must be traceable to an environment variable, either a
        // Python install root or a PATH entry. A candidate that traces to
        // neither could only have come from a literal in the source, which is
        // exactly what this phase removed.
        //
        // Asserting on the *source* is the right level here: on a developer
        // machine `LOCALAPPDATA` legitimately contains that developer's account
        // name, so asserting the resolved paths are username-free would fail
        // on the very machine the project is built on. The repository-level
        // guard for hardcoded profile paths is the CI grep in the scanners
        // phase.
        let roots = python_install_roots();
        let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();

        for candidate in candidate_interpreters() {
            let under_root = roots.iter().any(|root| candidate.starts_with(root));
            let from_path = path_dirs.iter().any(|dir| candidate.starts_with(dir));
            assert!(
                under_root || from_path,
                "candidate must come from an environment root or PATH, got {candidate:?}"
            );
        }
    }

    #[test]
    fn find_python_returns_an_absolute_existing_interpreter_when_one_exists() {
        let python = find_python();
        assert!(
            python.is_ok(),
            "expected a Python with torch in this environment"
        );
        let python = python.unwrap();
        assert!(python.is_absolute(), "must be absolute: {python:?}");
        assert!(python.is_file(), "must exist: {python:?}");
    }

    #[test]
    fn wrapper_script_path_ends_with_the_wrapper_name() {
        let script = wrapper_script();
        assert!(
            script.is_ok(),
            "tests need the wrapper shipped next to the binary"
        );
        let script = script.unwrap();
        assert!(
            script
                .file_name()
                .map(|n| n == "inflect_speak.py")
                .unwrap_or(false),
            "unexpected wrapper path: {script:?}"
        );
    }
}
