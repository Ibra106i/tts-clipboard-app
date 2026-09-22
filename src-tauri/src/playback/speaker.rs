//! The speech engine boundary.
//!
//! The actor never talks to SAPI directly; it drives a [`Speaker`]. That keeps
//! the playback rules testable with a fake engine, and it confines the COM
//! object (which is not `Send` and must not leave its thread) to one module.

use crate::error::AppResult;

/// Engine progress, in characters of the currently queued input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineProgress {
    /// Whether the engine is actively producing speech.
    pub running: bool,
    /// Character offset inside the queued text. Clamped by the state machine, so
    /// a bogus value from the engine can never over-report progress.
    pub offset_chars: u32,
}

impl EngineProgress {
    /// The engine has nothing left to say from what it was given.
    pub fn is_silent(self) -> bool {
        !self.running
    }
}

/// A speech engine.
///
/// Deliberately **not** `Send`: an engine is created by, owned by, and destroyed
/// inside the playback thread, so it never has to cross a thread boundary. That
/// is what removes the `unsafe impl Send` the previous implementation needed for
/// the COM object, and it is enforced by the compiler rather than by convention.
pub trait Speaker: 'static {
    fn speak(&mut self, text: &str) -> AppResult<()>;

    /// Stop speaking immediately and drop anything queued.
    fn purge(&mut self) -> AppResult<()>;

    fn pause(&mut self) -> AppResult<()>;

    fn resume(&mut self) -> AppResult<()>;

    /// `sapi_rate` is on the engine's own -10..10 scale.
    fn set_rate(&mut self, sapi_rate: i32) -> AppResult<()>;

    fn progress(&mut self) -> AppResult<EngineProgress>;
}

/// Convert a user-facing multiplier into SAPI's -10..10 scale.
pub fn rate_to_engine(rate: f32) -> i32 {
    (((rate - 1.0) * 13.333) as i32).clamp(-10, 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_rate_maps_to_zero() {
        assert_eq!(rate_to_engine(1.0), 0);
    }

    #[test]
    fn faster_rates_are_positive_and_slower_rates_negative() {
        assert!(rate_to_engine(1.25) > 0);
        assert!(rate_to_engine(2.0) > 0);
        assert!(rate_to_engine(0.5) < 0);
    }

    #[test]
    fn the_engine_scale_is_clamped() {
        assert_eq!(rate_to_engine(4.0), 10);
        assert_eq!(rate_to_engine(0.5), -6);
        assert!(rate_to_engine(100.0) <= 10);
        assert!(rate_to_engine(-100.0) >= -10);
    }
}
