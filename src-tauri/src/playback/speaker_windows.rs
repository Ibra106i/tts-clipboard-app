//! SAPI 5 speech engine.
//!
//! Every COM call in the application lives in this file. The object it wraps is
//! not `Send`, so it is created by - and never leaves - the playback thread
//! (see `speaker_factory`, which runs inside that thread rather than before it).

use crate::error::{AppError, AppResult};
use crate::playback::actor::SpeakerFactory;
use crate::playback::speaker::{EngineProgress, Speaker};
use windows::Win32::Media::Speech::*;
use windows::Win32::System::Com::*;

/// The engine object plus its COM apartment.
pub struct SapiSpeaker {
    voice: ISpVoice,
}

impl SapiSpeaker {
    /// Create the voice. Must run on the thread that will own it: COM is
    /// initialised here and the apartment is never shared.
    ///
    /// Apartment contract: the caller is the playback thread, and this is the
    /// first COM call it makes. `CoInitializeEx` therefore establishes the
    /// apartment before any interface pointer exists on that thread, and every
    /// later call in this file happens on the same thread. Nothing here is
    /// marshalled across apartments, which is what the previous
    /// `unsafe impl Send` silently assumed and never enforced.
    pub fn new() -> AppResult<Self> {
        // SAFETY: first COM call on this thread; the process is single-threaded
        // with respect to this apartment and never uninitialises it.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL).map_err(|e| {
                AppError::playback(format!("the speech voice could not be created ({e})"))
            })?;

            // Prefer a OneCore voice over the legacy default when one exists.
            match select_onecore_voice(&voice) {
                Ok(()) => log::info!("using a OneCore speech voice"),
                Err(e) => log::warn!("OneCore voice unavailable, using the default voice: {e}"),
            }

            Ok(Self { voice })
        }
    }
}

fn select_onecore_voice(voice: &ISpVoice) -> windows::core::Result<()> {
    // SAFETY: `voice` is a live interface pointer created on this thread, and
    // the token objects are created and released within this scope.
    unsafe {
        let category: ISpObjectTokenCategory =
            CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)?;

        let onecore_path = windows::core::HSTRING::from(
            "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech_OneCore\\Voices",
        );
        category.SetId(&onecore_path, false)?;

        let enumerator = category.EnumTokens(None, None)?;
        let mut token: Option<ISpObjectToken> = None;
        enumerator.Next(1, &mut token, None)?;

        let token = token.ok_or_else(windows::core::Error::from_win32)?;
        voice.SetVoice(&token)?;
        Ok(())
    }
}

impl Speaker for SapiSpeaker {
    fn speak(&mut self, text: &str) -> AppResult<()> {
        let htext = windows::core::HSTRING::from(text);
        // SAFETY: the pointer is owned by this struct, is used only from the
        // thread that created it, and the HSTRING outlives the call.
        unsafe {
            self.voice
                .Speak(&htext, SPF_ASYNC.0 as u32, Some(std::ptr::null_mut()))
                .map_err(|e| AppError::playback(format!("speech could not start ({e})")))
        }
    }

    /// An empty string with no flags tells SAPI to stop immediately and drop
    /// anything queued.
    fn purge(&mut self) -> AppResult<()> {
        // SAFETY: same-thread use of an owned interface pointer.
        unsafe {
            self.voice
                .Speak(
                    &windows::core::HSTRING::default(),
                    0,
                    Some(std::ptr::null_mut()),
                )
                .map_err(|e| AppError::playback(format!("speech could not be stopped ({e})")))
        }
    }

    fn pause(&mut self) -> AppResult<()> {
        // SAFETY: same-thread use of an owned interface pointer.
        unsafe {
            self.voice
                .Pause()
                .map_err(|e| AppError::playback(format!("speech could not pause ({e})")))
        }
    }

    fn resume(&mut self) -> AppResult<()> {
        // SAFETY: same-thread use of an owned interface pointer.
        unsafe {
            self.voice
                .Resume()
                .map_err(|e| AppError::playback(format!("speech could not resume ({e})")))
        }
    }

    fn set_rate(&mut self, sapi_rate: i32) -> AppResult<()> {
        // SAFETY: same-thread use of an owned interface pointer.
        unsafe {
            self.voice
                .SetRate(sapi_rate)
                .map_err(|e| AppError::playback(format!("speech rate could not be changed ({e})")))
        }
    }

    fn progress(&mut self) -> AppResult<EngineProgress> {
        // SAFETY: same-thread use of an owned interface pointer; the out
        // parameters are stack locals that outlive the call.
        unsafe {
            let mut status = SPVOICESTATUS::default();
            let mut bookmark = windows::core::PWSTR::null();
            self.voice
                .GetStatus(&mut status, &mut bookmark)
                .map_err(|e| AppError::playback(format!("speech status unavailable ({e})")))?;

            Ok(EngineProgress {
                // dwRunningState is 0 once SAPI has spoken everything queued.
                running: status.dwRunningState != 0,
                offset_chars: status.ulInputWordPos,
            })
        }
    }
}

/// How the actor obtains an engine. Runs on the playback thread.
pub fn speaker_factory() -> SpeakerFactory {
    Box::new(|| Ok(Box::new(SapiSpeaker::new()?) as Box<dyn Speaker>))
}
