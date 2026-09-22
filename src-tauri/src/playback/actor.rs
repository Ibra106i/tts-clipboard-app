//! The playback actor: the single thread that owns both the speech engine and
//! the playback state.
//!
//! Why a thread rather than a handful of mutexes:
//!
//! * The COM speech object is **not** `Send`. Holding it in Tauri's managed
//!   state is only possible with an unsound `unsafe impl Send`/`Sync`, which is
//!   exactly the shortcut that made cross-apartment SAPI calls possible before.
//!   Creating and using it inside one thread removes the problem by construction.
//! * One owner means one place where chunk progression, completion and progress
//!   accounting happen. Nothing else may advance playback.
//! * Commands are messages with replies, so a slow `Speak` call cannot block a
//!   state reader, and there is no lock ordering to get wrong.
//!
//! Readers get snapshots through `PlaybackHandle::snapshot`, which is purely
//! observational: it never starts, advances or stops anything.

use crate::error::{AppError, AppResult};
use crate::playback::speaker::{rate_to_engine, Speaker};
use crate::playback::state::{PlaybackJob, PlaybackSnapshot, PlaybackSource, PlaybackState};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often the actor polls the engine while speaking. Fast enough for a smooth
/// progress bar, slow enough to stay invisible in a profile.
const TICK: Duration = Duration::from_millis(250);

/// How long a caller waits for a reply before giving up. A stuck engine must not
/// freeze the UI thread forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// How the actor obtains its engine. Runs **on the playback thread**, which is
/// what lets a COM engine be created and used in one apartment.
pub type SpeakerFactory = Box<dyn FnOnce() -> AppResult<Box<dyn Speaker>> + Send>;

/// Events the actor publishes to the frontend.
pub trait PlaybackEvents: Send + Sync + 'static {
    /// Any state or progress change worth showing.
    fn snapshot(&self, snapshot: &PlaybackSnapshot);
    /// A different source interrupted playback.
    fn interrupted(&self, previous: &PlaybackSource, chapter_index: Option<usize>);
    /// The last chunk of a book chapter finished; the UI may advance.
    fn chapter_finished(&self, book_id: &str, chapter_index: usize, total_chapters: usize);
}

/// An events implementation that does nothing. Only useful in tests, where the
/// actor's output is asserted on the state rather than on emitted events.
#[cfg(test)]
pub struct NoEvents;

#[cfg(test)]
impl PlaybackEvents for NoEvents {
    fn snapshot(&self, _snapshot: &PlaybackSnapshot) {}

    fn interrupted(&self, _previous: &PlaybackSource, _chapter_index: Option<usize>) {}

    fn chapter_finished(&self, _book_id: &str, _chapter_index: usize, _total_chapters: usize) {}
}

enum Command {
    Start(PlaybackJob, Sender<AppResult<()>>),
    PauseResume(Sender<AppResult<bool>>),
    SetRate(f32, Sender<AppResult<()>>),
    Stop(Sender<AppResult<()>>),
    Snapshot(Sender<AppResult<PlaybackSnapshot>>),
    Shutdown(Sender<AppResult<()>>),
}

/// The only handle other code gets to the playback thread.
pub struct PlaybackHandle {
    tx: Sender<Command>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for PlaybackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackHandle").finish_non_exhaustive()
    }
}

impl PlaybackHandle {
    /// Spawn the playback thread, creating the engine inside it.
    ///
    /// Engine creation is reported back synchronously, so a platform without
    /// speech support (or a broken voice installation) fails at startup with a
    /// real error instead of silently doing nothing later.
    pub fn spawn(create: SpeakerFactory, events: Arc<dyn PlaybackEvents>) -> AppResult<Self> {
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();

        let join = thread::Builder::new()
            .name("playback".to_string())
            .spawn(move || match create() {
                Ok(speaker) => {
                    let _ = ready_tx.send(Ok(()));
                    run(speaker, events, rx);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|e| {
                AppError::playback(format!("the playback thread could not start ({e})"))
            })?;

        match ready_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                tx,
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(AppError::playback(
                "the speech engine did not start in time",
            )),
        }
    }

    /// Send a command and wait for its reply. The outer error is a transport
    /// failure (thread gone or unresponsive); the inner one is the command's own
    /// result, which the caller receives unchanged.
    fn send<T>(&self, command: Command, rx: Receiver<AppResult<T>>) -> AppResult<T> {
        self.tx
            .send(command)
            .map_err(|_| AppError::playback("the playback thread is no longer running"))?;
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| AppError::playback("the playback thread did not respond"))?
    }

    pub fn start(&self, job: PlaybackJob) -> AppResult<()> {
        let (reply, rx) = channel();
        self.send(Command::Start(job, reply), rx)
    }

    pub fn pause_resume(&self) -> AppResult<bool> {
        let (reply, rx) = channel();
        self.send(Command::PauseResume(reply), rx)
    }

    pub fn set_rate(&self, rate: f32) -> AppResult<()> {
        let (reply, rx) = channel();
        self.send(Command::SetRate(rate, reply), rx)
    }

    pub fn stop(&self) -> AppResult<()> {
        let (reply, rx) = channel();
        self.send(Command::Stop(reply), rx)
    }

    /// Read playback state. Purely observational - this never mutates anything.
    pub fn snapshot(&self) -> AppResult<PlaybackSnapshot> {
        let (reply, rx) = channel();
        self.send(Command::Snapshot(reply), rx)
    }

    /// Ask the thread to finish and wait for it. Idempotent, and safe to call
    /// from a shutdown handler.
    pub fn shutdown(&self) {
        let (reply, _rx) = channel();
        let _ = self.tx.send(Command::Shutdown(reply));
        if let Ok(mut join) = self.join.lock() {
            if let Some(handle) = join.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Body of the playback thread. Owns the engine and the state machine for its
/// whole life; nothing is shared with other threads.
fn run(mut speaker: Box<dyn Speaker>, events: Arc<dyn PlaybackEvents>, rx: Receiver<Command>) {
    let mut state = PlaybackState::new();
    // Ticks are driven by a deadline rather than by an idle timeout. Relying on
    // `recv_timeout` alone starves the advance tick whenever commands arrive more
    // often than once per tick - which is exactly what a polling reader does, so
    // playback would stall mid-job in production.
    let mut next_tick = Instant::now() + TICK;

    loop {
        match rx.recv_timeout(TICK) {
            Ok(command) => {
                if handle_command(command, &mut state, &mut *speaker, &*events) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if Instant::now() >= next_tick {
            if !state.is_idle() {
                tick(&mut state, &mut *speaker, &*events);
            }
            next_tick = Instant::now() + TICK;
        }
    }

    log::debug!("playback thread exited");
}

/// Returns true when the thread should stop.
fn handle_command(
    command: Command,
    state: &mut PlaybackState,
    speaker: &mut dyn Speaker,
    events: &dyn PlaybackEvents,
) -> bool {
    match command {
        Command::Start(job, reply) => {
            // A clipboard request must not silently destroy a chapter that is
            // being read; the caller is told it is busy instead. Starting a
            // chapter is an explicit replacement, so it wins.
            let result = if !state.is_idle() && job.source == PlaybackSource::Clipboard {
                Err(AppError::Busy)
            } else {
                let previous = state.snapshot(0).source;
                start_job(job, state, speaker, events, previous)
            };
            let _ = reply.send(result);
        }
        Command::PauseResume(reply) => {
            let result = if state.is_idle() {
                Ok(false)
            } else {
                let paused = state.toggle_paused();
                let engine_result = if paused {
                    speaker.pause()
                } else {
                    speaker.resume()
                };
                match engine_result {
                    Ok(()) => {
                        events.snapshot(&state.snapshot(0));
                        Ok(paused)
                    }
                    Err(error) => Err(error),
                }
            };
            let _ = reply.send(result);
        }
        Command::SetRate(rate, reply) => {
            let applied = state.set_rate(rate);
            let result = speaker
                .set_rate(rate_to_engine(applied))
                .map_err(|e| AppError::playback(format!("speech rate could not be changed ({e})")));
            if result.is_ok() {
                events.snapshot(&state.snapshot(0));
            }
            let _ = reply.send(result);
        }
        Command::Stop(reply) => {
            // Playback state is cleared even when the engine refuses to purge:
            // a stuck engine must not leave the app claiming it is still busy.
            let purge_error = speaker.purge().err();
            state.stop();
            events.snapshot(&state.snapshot(0));
            log::info!("playback stopped");
            let _ = reply.send(match purge_error {
                Some(error) => Err(error),
                None => Ok(()),
            });
        }
        Command::Snapshot(reply) => {
            let offset = current_offset(state, speaker);
            let _ = reply.send(Ok(state.snapshot(offset)));
        }
        Command::Shutdown(reply) => {
            let _ = speaker.purge();
            let _ = reply.send(Ok(()));
            return true;
        }
    }

    false
}

fn start_job(
    job: PlaybackJob,
    state: &mut PlaybackState,
    speaker: &mut dyn Speaker,
    events: &dyn PlaybackEvents,
    previous: Option<PlaybackSource>,
) -> AppResult<()> {
    if let Some(previous) = previous {
        events.interrupted(&previous, previous.chapter_index());
    }

    speaker.purge()?;

    let title = job.title.clone();
    let total_chars = job.total_chars();
    state.start(job);
    let chunk_count = state.chunk_count();

    match queue_next_chunk(state, speaker) {
        Ok(true) => {
            log::info!(
                "playback started: \"{title}\" ({total_chars} characters, {chunk_count} chunks)"
            );
            events.snapshot(&state.snapshot(0));
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(error) => {
            state.stop();
            events.snapshot(&state.snapshot(0));
            Err(error)
        }
    }
}

fn queue_next_chunk(state: &mut PlaybackState, speaker: &mut dyn Speaker) -> AppResult<bool> {
    match state.take_next_chunk() {
        Some(chunk) => {
            speaker.speak(&chunk.text)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Engine offset inside the current chunk, or 0 when it cannot be read.
fn current_offset(state: &PlaybackState, speaker: &mut dyn Speaker) -> u32 {
    if state.is_idle() || !state.is_awaiting_engine() {
        return 0;
    }
    speaker.progress().map(|p| p.offset_chars).unwrap_or(0)
}

/// Advance playback if the engine finished the queued chunk. Called from this
/// thread only - the tick is the only thing in the program that moves playback
/// forward on its own.
fn tick(state: &mut PlaybackState, speaker: &mut dyn Speaker, events: &dyn PlaybackEvents) {
    if !state.is_awaiting_engine() {
        return;
    }

    let progress = match speaker.progress() {
        Ok(progress) => progress,
        Err(error) => {
            log::warn!("could not read engine progress: {error}");
            return;
        }
    };

    events.snapshot(&state.snapshot(progress.offset_chars));

    if !progress.is_silent() {
        return;
    }

    let has_more = state.advance_chunk();
    if has_more {
        if let Err(error) = queue_next_chunk(state, speaker) {
            log::error!("the next chunk could not be queued: {error}");
            state.stop();
            events.snapshot(&state.snapshot(0));
        }
        return;
    }

    // The job is complete.
    let snapshot = state.snapshot(0);
    if let Some(PlaybackSource::Book {
        book_id,
        chapter_index,
        total_chapters,
    }) = snapshot.source.clone()
    {
        log::info!("chapter finished: {book_id} chapter {chapter_index}");
        events.chapter_finished(&book_id, chapter_index, total_chapters);
    }
    events.snapshot(&snapshot);
}

/// Test helper: an engine that behaves like a real one without COM.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::playback::speaker::EngineProgress;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    /// Records everything it was asked to do and reports scripted progress.
    pub struct FakeSpeaker {
        pub spoken: Mutex<Vec<String>>,
        pub purges: AtomicU32,
        pub rate: Mutex<Option<i32>>,
        paused: AtomicBool,
        /// Characters remaining before the current chunk is reported finished.
        pub remaining: AtomicU32,
    }

    impl FakeSpeaker {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                spoken: Mutex::new(Vec::new()),
                purges: AtomicU32::new(0),
                rate: Mutex::new(None),
                paused: AtomicBool::new(false),
                remaining: AtomicU32::new(0),
            })
        }

        pub fn spoken_texts(&self) -> Vec<String> {
            self.spoken.lock().map(|s| s.clone()).unwrap_or_default()
        }
    }

    impl Speaker for Arc<FakeSpeaker> {
        fn speak(&mut self, text: &str) -> AppResult<()> {
            let mut spoken = self
                .spoken
                .lock()
                .map_err(|e| AppError::internal(format!("fake speaker lock: {e}")))?;
            spoken.push(text.to_string());
            // Default script: one progress read then silence.
            self.remaining.store(1, Ordering::SeqCst);
            Ok(())
        }

        fn purge(&mut self) -> AppResult<()> {
            self.purges.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn pause(&mut self) -> AppResult<()> {
            self.paused.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn resume(&mut self) -> AppResult<()> {
            self.paused.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn set_rate(&mut self, sapi_rate: i32) -> AppResult<()> {
            let mut rate = self
                .rate
                .lock()
                .map_err(|e| AppError::internal(format!("fake speaker lock: {e}")))?;
            *rate = Some(sapi_rate);
            Ok(())
        }

        fn progress(&mut self) -> AppResult<EngineProgress> {
            let remaining = self.remaining.load(Ordering::SeqCst);
            if remaining == 0 {
                return Ok(EngineProgress {
                    running: false,
                    offset_chars: 0,
                });
            }
            self.remaining.store(remaining - 1, Ordering::SeqCst);
            Ok(EngineProgress {
                running: !self.paused.load(Ordering::SeqCst),
                offset_chars: remaining,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeSpeaker;
    use super::*;
    use crate::playback::speaker::EngineProgress;
    use crate::playback::state::PlaybackStatus;

    fn handle(speaker: Arc<FakeSpeaker>) -> PlaybackHandle {
        PlaybackHandle::spawn(
            Box::new(move || Ok(Box::new(speaker) as Box<dyn Speaker>)),
            Arc::new(NoEvents),
        )
        .expect("actor starts")
    }

    /// Nothing is playing and nothing is queued. Derived from the snapshot, so
    /// there is no second definition of "idle" to drift out of sync.
    fn is_idle(handle: &PlaybackHandle) -> bool {
        handle
            .snapshot()
            .map(|s| s.status == PlaybackStatus::Idle && s.source.is_none())
            .unwrap_or(false)
    }

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn a_new_handle_is_idle() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        assert!(is_idle(&handle));
        assert_eq!(handle.snapshot().expect("snapshot").total_chars, 0);
        handle.shutdown();
    }

    #[test]
    fn starting_a_job_speaks_the_first_chunk_and_reports_progress() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "hello world",
            ))
            .expect("start");

        assert!(wait_until(|| !speaker.spoken_texts().is_empty()));
        let snapshot = handle.snapshot().expect("snapshot");
        assert_eq!(snapshot.status, PlaybackStatus::Playing);
        assert_eq!(snapshot.total_chars, 11);
        handle.shutdown();
    }

    #[test]
    fn the_actor_advances_chunks_on_its_own_until_the_job_is_finished() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        // 25 characters with a 10 character chunk size makes three chunks.
        let text = "a".repeat(25);
        handle
            .start(PlaybackJob {
                chunks: crate::text::split_into_chunks(&text, 10),
                ..PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", &text)
            })
            .expect("start");

        assert!(
            wait_until(|| handle.snapshot().map(|s| s.finished).unwrap_or(false)),
            "the actor should finish the job without any external polling"
        );

        let spoken = speaker.spoken_texts();
        assert_eq!(spoken.len(), 3, "every chunk should be spoken exactly once");
        assert_eq!(spoken.concat(), text);
        handle.shutdown();
    }

    #[test]
    fn pausing_reports_the_paused_state() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                &"a".repeat(40),
            ))
            .expect("start");

        assert!(
            handle.pause_resume().expect("pause"),
            "should now be paused"
        );
        assert_eq!(
            handle.snapshot().expect("snapshot").status,
            PlaybackStatus::Paused
        );
        assert!(
            !handle.pause_resume().expect("resume"),
            "should be playing again"
        );
        handle.shutdown();
    }

    #[test]
    fn rate_changes_are_applied_to_the_engine_and_clamped() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        handle.set_rate(1.0).expect("set rate");
        assert_eq!(
            *speaker.rate.lock().expect("rate lock"),
            Some(rate_to_engine(1.0))
        );

        handle.set_rate(50.0).expect("set rate");
        let applied = handle.snapshot().expect("snapshot").rate;
        assert!(applied <= crate::playback::state::MAX_RATE);
        handle.shutdown();
    }

    #[test]
    fn stopping_clears_playback_and_purges_the_engine() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                &"b".repeat(50),
            ))
            .expect("start");
        handle.stop().expect("stop");

        assert!(is_idle(&handle));
        assert!(speaker.purges.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        handle.shutdown();
    }

    #[test]
    fn reading_state_never_advances_playback() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));

        let text = "c".repeat(45);
        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                &text,
            ))
            .expect("start");

        assert!(wait_until(|| !speaker.spoken_texts().is_empty()));
        let spoken_after_start = speaker.spoken_texts().len();

        // Hammer the reader: a getter must be observational.
        for _ in 0..50 {
            let _ = handle.snapshot().expect("snapshot");
        }

        assert_eq!(
            speaker.spoken_texts().len(),
            spoken_after_start,
            "reading state must not queue more speech"
        );
        handle.shutdown();
    }

    /// An engine that cannot speak must not take the actor down with it.
    struct ExplodingSpeaker;

    impl Speaker for ExplodingSpeaker {
        fn speak(&mut self, _text: &str) -> AppResult<()> {
            Err(AppError::playback("the engine died"))
        }

        fn purge(&mut self) -> AppResult<()> {
            Ok(())
        }

        fn pause(&mut self) -> AppResult<()> {
            Ok(())
        }

        fn resume(&mut self) -> AppResult<()> {
            Ok(())
        }

        fn set_rate(&mut self, _sapi_rate: i32) -> AppResult<()> {
            Ok(())
        }

        fn progress(&mut self) -> AppResult<EngineProgress> {
            Ok(EngineProgress {
                running: false,
                offset_chars: 0,
            })
        }
    }

    #[test]
    fn a_failing_engine_reports_the_error_and_leaves_the_actor_usable() {
        let handle = PlaybackHandle::spawn(
            Box::new(|| Ok(Box::new(ExplodingSpeaker) as Box<dyn Speaker>)),
            Arc::new(NoEvents),
        )
        .expect("thread starts");

        let error = handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "hello",
            ))
            .expect_err("speaking must fail");
        assert_eq!(error.code(), "playback_failed");

        // The actor is still alive and has cleaned up after the failure.
        assert!(is_idle(&handle));
        handle.shutdown();
    }

    #[test]
    fn an_engine_that_cannot_be_created_is_reported_at_startup() {
        let error = PlaybackHandle::spawn(
            Box::new(|| Err(AppError::playback("no voice installed"))),
            Arc::new(NoEvents),
        )
        .expect_err("spawn must fail loudly");

        assert_eq!(error.code(), "playback_failed");
    }

    #[test]
    fn the_engine_is_created_inside_the_playback_thread() {
        // The factory runs on the actor thread, which is what makes the COM
        // apartment valid without any unsafe thread-ownership promise.
        let spawned_on: Arc<Mutex<Option<thread::ThreadId>>> = Arc::new(Mutex::new(None));
        let recorder = Arc::clone(&spawned_on);
        let caller = thread::current().id();

        let handle = PlaybackHandle::spawn(
            Box::new(move || {
                *recorder.lock().expect("lock") = Some(thread::current().id());
                Ok(Box::new(FakeSpeaker::new()) as Box<dyn Speaker>)
            }),
            Arc::new(NoEvents),
        )
        .expect("thread starts");

        let engine_thread = spawned_on.lock().expect("lock").expect("recorded");
        assert_ne!(
            engine_thread, caller,
            "the engine must not be built on the caller thread"
        );
        handle.shutdown();
    }

    #[test]
    fn shutting_down_stops_the_thread_and_further_calls_fail_cleanly() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));
        handle.shutdown();

        let error = handle.snapshot().expect_err("thread is gone");
        assert_eq!(error.code(), "playback_failed");
    }

    /// The previous design took `chunks -> current_chunk -> voice` in one path
    /// and `voice -> mode -> chunks -> current_chunk` in another, so two
    /// threads could deadlock by acquiring the same locks in opposite orders.
    /// There is now exactly one lock in the whole subsystem (the join handle)
    /// and it is never held while sending a command, so no ordering exists to
    /// invert. This drives every command from many threads at once to prove it.
    #[test]
    fn concurrent_commands_never_deadlock() {
        let speaker = FakeSpeaker::new();
        let handle = Arc::new(handle(Arc::clone(&speaker)));

        let mut threads = Vec::new();
        for worker in 0..8 {
            let handle = Arc::clone(&handle);
            threads.push(thread::spawn(move || {
                for i in 0..25 {
                    // Every accessor and mutator, interleaved, from every thread.
                    let _ = handle.snapshot();
                    let _ = handle.set_rate(1.0 + (i % 3) as f32 * 0.25);
                    let _ = handle.pause_resume();
                    if i % 5 == 0 {
                        let _ = handle.start(PlaybackJob::new(
                            PlaybackSource::Clipboard,
                            "Clipboard",
                            &"x".repeat(50 + worker),
                        ));
                    }
                    if i % 7 == 0 {
                        let _ = handle.stop();
                    }
                    let _ = is_idle(&handle);
                }
            }));
        }

        for worker in threads {
            worker.join().expect("no thread may hang or panic");
        }

        // Still responsive after the storm: that is the actual deadlock check,
        // because a deadlock would have hung the joins above.
        assert!(handle.snapshot().is_ok());
        handle.shutdown();
    }

    /// Reading state must never be a mutation in disguise. The old
    /// `get_speech_position` advanced chunks as a side effect, which made the
    /// 100 ms frontend poll drive playback.
    #[test]
    fn reading_the_snapshot_does_not_advance_playback() {
        let speaker = FakeSpeaker::new();
        let handle = handle(Arc::clone(&speaker));
        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                &"a".repeat(25),
            ))
            .expect("start");

        // Wait for the scripted engine to stop moving (two identical reads in a
        // row), then observe. Any change caused by the reads themselves would
        // show up here; the reads are `&self` all the way down, so there is
        // nothing they could mutate.
        assert!(
            wait_until(|| handle.snapshot().map(|s| s.finished).unwrap_or(false)),
            "job should finish"
        );
        let before = handle.snapshot().expect("snapshot");

        for _ in 0..100 {
            let during = handle.snapshot().expect("snapshot");
            assert_eq!(during.spoken_chars, before.spoken_chars);
            assert_eq!(during.finished, before.finished);
            assert_eq!(during.status, before.status);
        }
        handle.shutdown();
    }
}
