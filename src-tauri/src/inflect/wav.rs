//! A small PCM WAV header reader.
//!
//! Inflect writes standard 16-bit PCM WAV files, so this only needs to parse a
//! header and report the facts the playback path cares about: sample rate,
//! bytes per sample, and data size. It is deliberately not a general audio
//! library.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub struct WavHeader {
    pub sample_rate: u32,
    pub bytes_per_sample: u16,
    pub data_bytes: u64,
}

impl WavHeader {
    pub fn read(file: &File) -> Result<Self, WavError> {
        let mut f = file;
        let mut buf = [0u8; 44];
        f.read_exact(&mut buf)
            .map_err(|e| WavError::Read(e.to_string()))?;

        if &buf[0..4] != b"RIFF" {
            return Err(WavError::NotWav);
        }
        if &buf[8..12] != b"WAVE" {
            return Err(WavError::NotWav);
        }

        let fmt_chunk = &buf[12..36];
        if &fmt_chunk[0..4] != b"fmt " {
            return Err(WavError::NoFmtChunk);
        }

        let audio_format = u16::from_le_bytes(
            fmt_chunk[8..10]
                .try_into()
                .map_err(|_| WavError::Read("slice to u16".into()))?,
        );
        if audio_format != 1 {
            return Err(WavError::NotPcm(audio_format));
        }

        let num_channels = u16::from_le_bytes(
            fmt_chunk[10..12]
                .try_into()
                .map_err(|_| WavError::Read("slice to u16".into()))?,
        );
        if num_channels != 1 {
            return Err(WavError::NotMono(num_channels));
        }

        let sample_rate = u32::from_le_bytes(
            fmt_chunk[12..16]
                .try_into()
                .map_err(|_| WavError::Read("slice to u32".into()))?,
        );
        let bytes_per_sample = u16::from_le_bytes(
            fmt_chunk[22..24]
                .try_into()
                .map_err(|_| WavError::Read("slice to u16".into()))?,
        ) / num_channels;

        // Find the "data" chunk properly in case there are extra chunks.
        let mut data_bytes = 0u64;
        loop {
            f.seek(SeekFrom::Current(0))
                .map_err(|e| WavError::Read(e.to_string()))?;
            let mut chunk_header = [0u8; 8];
            match f.read_exact(&mut chunk_header) {
                Ok(_) => {}
                Err(e) => return Err(WavError::Read(e.to_string())),
            }
            let chunk_id = &chunk_header[0..4];
            let chunk_size = u32::from_le_bytes(
                chunk_header[4..8]
                    .try_into()
                    .map_err(|_| WavError::Read("slice to u32".into()))?,
            );
            if chunk_id == b"data" {
                data_bytes = chunk_size as u64;
                break;
            }
            // Skip this chunk's data.
            f.seek(SeekFrom::Current(chunk_size as i64))
                .map_err(|e| WavError::Read(e.to_string()))?;
        }

        Ok(Self {
            sample_rate,
            bytes_per_sample,
            data_bytes,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("cannot read WAV file")]
    Read(String),
    #[error("not a WAV file")]
    NotWav,
    #[error("missing fmt chunk")]
    NoFmtChunk,
    #[error("unsupported audio format {0}")]
    NotPcm(u16),
    #[error("only mono WAV is supported (got {0} channels)")]
    NotMono(u16),
}
