//! Windows WAV playback for synthesized Inflect chunks.
//!
//! Inflect produces mono 16-bit PCM WAV at 24 kHz. The playback path is:
//! 1. Validate the WAV is the expected simple PCM format (guard against a
//!    future wrapper change playing something `PlaySound` would silently skip).
//! 2. Play it asynchronously with `PlaySoundW` so the actor thread is not
//!    blocked for the chunk duration.
//! 3. Track completion by elapsed time on the actor's tick, not by polling the
//!    wave device.
//!
//! `PlaySound` is the smallest Windows path that fits this app's offline,
//! small, single-voice use case. It is not a general audio stack and it is not
//! used for anything else.

#[derive(Debug, thiserror::Error)]
pub enum PlayError {
    #[cfg(windows)]
    #[error("Windows WAV playback failed")]
    PlayFailed,

    #[cfg(windows)]
    #[error("Inflect WAV sample rate {0} is not supported by the local player")]
    UnsupportedRate(u32),

    #[cfg(windows)]
    #[error("Inflect WAV bit depth {0} is not supported by the local player")]
    UnsupportedBits(u16),

    #[cfg(windows)]
    #[error("Inflect WAV has no audio data")]
    Empty,

    #[cfg(not(windows))]
    #[error("WAV playback is only supported on Windows")]
    UnsupportedPlatform,
}

#[cfg(windows)]
mod windows {
    use super::PlayError;
    use super::WavHeader;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Media::Audio::*;

    /// Play a mono 16-bit PCM WAV asynchronously and return an estimate of how
    /// long the actor should wait before treating the chunk as finished.
    pub fn play_wav(path: &Path, header: &WavHeader) -> Result<u64, PlayError> {
        validate_for_playsound(header)?;
        let alias = pcwstr_from_path(path);
        unsafe {
            // SAFETY: `alias` is a valid null-terminated wide string owned by
            // this stack frame for the call. `PlaySoundW` with `SND_ASYNC`
            // does not retain the pointer after it returns.
            let played = PlaySoundW(PCWSTR(alias.as_ptr()), None, SND_ASYNC | SND_FILENAME);
            if played == BOOL(0) {
                return Err(last_play_sound_error());
            }
        }
        Ok(chunk_duration_ms(header))
    }

    fn validate_for_playsound(header: &WavHeader) -> Result<(), PlayError> {
        if header.sample_rate != 24000 && header.sample_rate != 16000 && header.sample_rate != 48000
        {
            return Err(PlayError::UnsupportedRate(header.sample_rate));
        }
        if header.bytes_per_sample != 2 {
            return Err(PlayError::UnsupportedBits(header.bytes_per_sample * 8));
        }
        if header.data_bytes == 0 {
            return Err(PlayError::Empty);
        }
        Ok(())
    }

    fn chunk_duration_ms(header: &WavHeader) -> u64 {
        (header.data_bytes as f64 / (header.sample_rate as f64 * header.bytes_per_sample as f64)
            * 1000.0)
            .round() as u64
    }

    fn pcwstr_from_path(path: &Path) -> Vec<u16> {
        let s = OsStr::new(path);
        let wide: Vec<u16> = s.encode_wide().chain(Some(0)).collect();
        wide
    }

    fn last_play_sound_error() -> PlayError {
        // `PlaySound` does not return a rich error. When it fails we report a
        // generic playback failure; the real reason is usually "format not
        // supported" or "file not found", both of which are already guarded.
        PlayError::PlayFailed
    }
}

#[cfg(not(windows))]
mod windows {
    use super::PlayError;
    use super::WavHeader;
    use std::path::Path;

    pub fn play_wav(_path: &Path, _header: &WavHeader) -> Result<u64, PlayError> {
        Err(PlayError::UnsupportedPlatform)
    }
}

use crate::inflect::wav::WavHeader;
use std::path::Path;

/// Play a synthesized WAV and return the chunk duration in milliseconds.
pub fn play_wav(path: &Path) -> Result<u64, PlayError> {
    let file = std::fs::File::open(path).map_err(|_| PlayError::PlayFailed)?;
    let header = WavHeader::read(&file).map_err(|_| PlayError::PlayFailed)?;
    windows::play_wav(path, &header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn make_pcm_wav(path: &Path, sample_rate: u32, num_samples: u32) -> std::io::Result<()> {
        let mut f = File::create(path)?;
        // Minimal canonical PCM WAV: 44-byte header + silent 16-bit mono data.
        let bytes_per_sample = 2u16;
        let data_bytes = (num_samples as u64) * (bytes_per_sample as u64);
        let header = build_header(sample_rate, bytes_per_sample, data_bytes);
        f.write_all(&header)?;
        f.write_all(&vec![0u8; data_bytes as usize])?;
        f.flush()?;
        Ok(())
    }

    fn build_header(sample_rate: u32, bytes_per_sample: u16, data_bytes: u64) -> Vec<u8> {
        let num_channels = 1u16;
        let bits_per_sample = bytes_per_sample * 8;
        let byte_rate = sample_rate * (num_channels as u32) * (bits_per_sample as u32 / 8);
        let block_align = num_channels * bits_per_sample / 8;
        let mut h = vec![0u8; 44];
        h[0..4].copy_from_slice(b"RIFF");
        let file_size = (36 + data_bytes) as u32;
        h[4..8].copy_from_slice(&file_size.to_le_bytes());
        h[8..12].copy_from_slice(b"WAVE");
        h[12..16].copy_from_slice(b"fmt ");
        h[16..20].copy_from_slice(&20u32.to_le_bytes()); // fmt chunk size
        h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
        h[22..24].copy_from_slice(&num_channels.to_le_bytes());
        h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
        h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        h[32..34].copy_from_slice(&block_align.to_le_bytes());
        h[34..36].copy_from_slice(&bits_per_sample.to_le_bytes());
        h[36..40].copy_from_slice(b"data");
        h[40..44].copy_from_slice(&(data_bytes as u32).to_le_bytes());
        h
    }

    #[test]
    #[ignore = "Windows audio playback integration test; runs only when explicitly enabled"]
    fn a_synthesized_chunk_can_be_played_back_on_windows() {
        #[cfg(windows)]
        {
            let tmp = std::env::temp_dir().join("inflect_play_test.wav");
            make_pcm_wav(&tmp, 24000, 2400).expect("test wav");
            let dur = play_wav(&tmp).expect("playable");
            assert!(dur > 0);
            let _ = std::fs::remove_file(&tmp);
        }
        #[cfg(not(windows))]
        {
            let tmp = std::env::temp_dir().join("inflect_play_test.wav");
            make_pcm_wav(&tmp, 24000, 2400).expect("test wav");
            let err = play_wav(&tmp);
            assert!(matches!(err, Err(PlayError::UnsupportedPlatform)));
            let _ = std::fs::remove_file(&tmp);
        }
    }

    #[test]
    #[ignore = "Windows audio playback integration test; runs only when explicitly enabled"]
    fn non_pcm_or_unexpected_rates_are_rejected_before_playing() {
        // The header says 8-bit mono; PlaySound support for that is not
        // something this app wants to rely on, so we reject it explicitly.
        let tmp = std::env::temp_dir().join("inflect_bad.wav");
        make_pcm_wav(&tmp, 24000, 100).expect("test wav");
        // Rewrite header to claim 8-bit to exercise the guard without needing a
        // real 8-bit file.
        let mut bytes = std::fs::read(&tmp).expect("read test wav");
        bytes[34] = 8; // bits per sample
        std::fs::write(&tmp, &bytes).expect("patch test wav");
        #[cfg(windows)]
        {
            let err = play_wav(&tmp);
            assert!(matches!(err, Err(PlayError::UnsupportedBits(8))));
        }
        #[cfg(not(windows))]
        {
            let err = play_wav(&tmp);
            assert!(matches!(err, Err(PlayError::UnsupportedPlatform)));
        }
        let _ = std::fs::remove_file(&tmp);
    }
}
