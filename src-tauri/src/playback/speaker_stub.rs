//! Non-Windows speech engine.
//!
//! Speech synthesis is Windows SAPI 5, so on any other platform this engine is
//! used instead. Every operation fails with `unsupported_platform`, which the UI
//! surfaces as a clear message rather than as silence.

use crate::error::{AppError, AppResult};
use crate::playback::actor::SpeakerFactory;
use crate::playback::speaker::{EngineProgress, Speaker};

const FEATURE: &str = "Text to speech";

pub struct UnsupportedSpeaker;

fn unsupported() -> AppError {
    AppError::unsupported_platform(FEATURE)
}

impl Speaker for UnsupportedSpeaker {
    fn speak(&mut self, _text: &str) -> AppResult<()> {
        Err(unsupported())
    }

    fn purge(&mut self) -> AppResult<()> {
        Ok(())
    }

    fn pause(&mut self) -> AppResult<()> {
        Err(unsupported())
    }

    fn resume(&mut self) -> AppResult<()> {
        Err(unsupported())
    }

    fn set_rate(&mut self, _sapi_rate: i32) -> AppResult<()> {
        Err(unsupported())
    }

    fn progress(&mut self) -> AppResult<EngineProgress> {
        // There is no engine, so it is never speaking.
        Ok(EngineProgress {
            running: false,
            offset_chars: 0,
        })
    }
}

/// How the actor obtains an engine. Runs on the playback thread.
pub fn speaker_factory() -> SpeakerFactory {
    Box::new(|| Ok(Box::new(UnsupportedSpeaker) as Box<dyn Speaker>))
}
