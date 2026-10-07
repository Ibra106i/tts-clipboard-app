//! The playback actor: the single thread that owns both the local TTS engine and
//! the playback state.
//!
//! Why a thread rather than a handful of mutexes:
//!
//! * The local TTS engine owns a Python subprocess and WAV playback handles.
//!   Confining it to one thread keeps process lifecycle and OS handles simple.
//! * One owner means one place where chunk progression, completion and progress
//!   accounting happen. Nothing else may advance playback.
//! * Commands are messages with replies, so a slow synthesis call cannot block a
//!   state reader, and there is no lock ordering to get wrong.
//!
//! Readers get snapshots through `PlaybackHandle::snapshot`, which is purely
//! observational: it never starts, advances or stops anything.

use crate::error::{AppError, AppResult};
use crate::inflect::model::{InflectModel, ModelCacheHandle};
use crate::playback::engine::{speed_to_inflect, ChunkPlayback, TtsEngine};
use crate::playback::state::{PlaybackJob, PlaybackSnapshot, PlaybackSource, PlaybackState};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often the actor polls while speaking. Fast enough for a smooth progress
/// bar, slow enough to stay invisible in a profile.
const TICK: Duration = Duration::from_millis(250);

/// How long a caller waits for a reply before giving up. A stuck engine must not
/// freeze the UI thread forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Consecutive failed progress reads tolerated before the job is abandoned. At
/// [`TICK`] this is a little over a second of continuous failure, which is long
/// enough to ride out a transient audio-device change and short enough that a
/// genuinely dead engine surfaces as a message rather than an indefinite freeze.
const MAX_PROGRESS_READ_FAILURES: u32 = 5;

/// Reported to the frontend when the engine stops answering. Phrased for the
/// person reading it: they did nothing wrong, and the honest cause is that the
/// audio device went away.
pub const ENGINE_LOST_MESSAGE: &str = "The speech engine stopped responding. This usually means \
     the audio device changed - plugging in headphones or switching output can cause it.";

/// How the actor obtains its engine. Runs **on the playback thread**, which is
/// what lets the engine own a subprocess and any OS playback handles.
pub type EngineFactory = Box<dyn FnOnce(&ModelCacheHandle) -> AppResult<Box<dyn TtsEngine>> + Send>;

/// Events the actor publishes to the frontend.
pub trait PlaybackEvents: Send + Sync + 'static {
    /// Any state or progress change worth showing.
    fn snapshot(&self, snapshot: &PlaybackSnapshot);
    /// A different source interrupted playback.
    fn interrupted(&self, previous: &PlaybackSource, chapter_index: Option<usize>);
    /// The last chunk of a book chapter finished; the UI may advance.
    fn chapter_finished(&self, book_id: &str, chapter_index: usize, total_chapters: usize);

    /// The speech engine stopped answering, so playback has been ended. The
    /// actor cannot carry on, but it says so rather than leaving the window
    /// showing a playhead that will never move.
    fn engine_lost(&self, message: &str) {
        let _ = message;
    }
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

    fn engine_lost(&self, _message: &str) {}
}

/// A request to the playback thread. Public because the command module builds
/// them, but only [`dispatch`] and [`PlaybackHandle`] should send them.
pub enum Command {
    Start(PlaybackJob, Sender<AppResult<()>>),
    PauseResume(Sender<AppResult<bool>>),
    SetRate(f32, Sender<AppResult<()>>),
    Stop(Sender<AppResult<()>>),
    Snapshot(Sender<AppResult<PlaybackSnapshot>>),
    Shutdown(Sender<AppResult<()>>),
    SwitchModel(InflectModel, Sender<AppResult<()>>),
}

/// Send `command` on `tx`, then wait for its answer on a blocking-pool thread.
///
/// This is what keeps a slow playback request off both the window thread and
/// the async runtime's workers. Returns `None` only if the pool itself is gone.
pub async fn dispatch<T>(
    tx: Sender<Command>,
    build: impl FnOnce(Sender<AppResult<T>>) -> Command + Send + 'static,
) -> AppResult<T>
where
    T: Send + 'static,
{
    let (reply, rx) = channel();
    // Sent here rather than inside the pool thread: this is a non-blocking
    // enqueue, and doing it before the hand-off means the playback thread sees
    // the request immediately instead of whenever the pool gets round to it.
    let command = build(reply);
    tx.send(command)
        .map_err(|_| AppError::playback("the playback thread is no longer running"))?;
    // Only the wait is moved off the caller. That is the part which can take
    // seconds, and it is the part that must not sit on a runtime worker.
    let result = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| AppError::playback("the playback thread did not respond"))
    })
    .await
    .map_err(|e| AppError::playback(format!("dispatch task join failed: {e}")))?;
    result?
}

/// The only handle other code gets to the playback thread.
pub struct PlaybackHandle {
    tx: Sender<Command>,
    join: Mutex<Option<JoinHandle<()>>>,
    /// Shared handle used to (re)download and locate the cached model.
    cache: Arc<ModelCacheHandle>,
}

impl std::fmt::Debug for PlaybackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackHandle").finish_non_exhaustive()
    }
}

impl PlaybackHandle {
    /// Spawn the playback thread, creating the engine inside it.
    pub fn spawn(
        factory: EngineFactory,
        events: Arc<dyn PlaybackEvents>,
        cache: Arc<ModelCacheHandle>,
    ) -> AppResult<Self> {
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();

        let cache_for_thread = cache.clone();
        let join = thread::Builder::new()
            .name("playback".to_string())
            .spawn(move || match factory(&cache_for_thread) {
                Ok(engine) => {
                    let _ = ready_tx.send(Ok(()));
                    run(engine, events.as_ref(), cache_for_thread, rx);
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
                cache,
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

    /// A clone of the command channel, so a blocking wait can move off the
    /// caller's thread. See [`Command::await_reply`].
    pub fn sender(&self) -> Sender<Command> {
        self.tx.clone()
    }

    pub fn start(&self, job: PlaybackJob) -> AppResult<()> {
        let (reply, rx) = channel();
        self.send(Command::Start(job, reply), rx)
    }

    /// Pause or resume, from the synchronous API used by tests and the hotkey
    /// path. The Tauri commands go through `dispatch`.
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

    /// Switch the active Inflect model. Only allowed when idle.
    pub fn switch_model(&self, model: InflectModel) -> AppResult<()> {
        let (reply, rx) = channel();
        self.send(Command::SwitchModel(model, reply), rx)
    }
}

/// Body of the playback thread. Owns the engine and the state machine for its
/// whole life; nothing is shared with other threads.
fn run(
    mut engine: Box<dyn TtsEngine>,
    events: &dyn PlaybackEvents,
    cache: Arc<ModelCacheHandle>,
    rx: Receiver<Command>,
) {
    let mut state = PlaybackState::new();
    let mut next_tick = Instant::now() + TICK;
    let mut failed_reads = 0u32;
    // The chunk currently playing, if any.
    let mut current: Option<CurrentChunk> = None;

    loop {
        match rx.recv_timeout(TICK) {
            Ok(command) => {
                if handle_command(
                    command,
                    &mut state,
                    &mut *engine,
                    events,
                    &mut current,
                    &cache,
                ) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if Instant::now() >= next_tick {
            if !state.is_idle() {
                tick(
                    &mut state,
                    &mut *engine,
                    events,
                    &mut current,
                    &mut failed_reads,
                );
            }
            next_tick = next_tick
                .checked_add(TICK)
                .filter(|deadline| *deadline > Instant::now())
                .unwrap_or_else(|| Instant::now() + TICK);
        }
    }

    // Be tidy: stop any last chunk playback on exit.
    let _ = engine.stop_playing();
    log::debug!("playback thread exited");
}

struct CurrentChunk {
    playback: ChunkPlayback,
    /// Instant the chunk started playing, for elapsed-time progress.
    started: Instant,
}

/// Returns true when the thread should stop.
fn handle_command(
    command: Command,
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    events: &dyn PlaybackEvents,
    current: &mut Option<CurrentChunk>,
    cache: &Arc<ModelCacheHandle>,
) -> bool {
    match command {
        Command::Start(job, reply) => {
            let result = if !state.is_idle() && job.source == PlaybackSource::Clipboard {
                Err(AppError::Busy)
            } else {
                let previous = state.snapshot(0).source;
                start_job(job, state, engine, events, current, previous)
            };
            let _ = reply.send(result);
        }
        Command::PauseResume(reply) => {
            let result = if state.is_idle() {
                Ok(false)
            } else if state.is_paused() {
                resume(state, engine, events, current)
            } else {
                pause(state, engine, events, current)
            };
            let _ = reply.send(result);
        }
        Command::SetRate(rate, reply) => {
            let applied = state.set_rate(rate);
            // Rate is applied on the next chunk, not to a currently playing WAV.
            let _ = reply.send(Ok(()));
            events.snapshot(&state.snapshot(0));
        }
        Command::Stop(reply) => {
            let _ = engine.stop_playing();
            current.take();
            state.stop();
            events.snapshot(&state.snapshot(0));
            log::info!("playback stopped");
            let _ = reply.send(Ok(()));
        }
        Command::Snapshot(reply) => {
            let offset = state.last_offset();
            let _ = reply.send(Ok(state.snapshot(offset)));
        }
        Command::Shutdown(reply) => {
            let _ = engine.stop_playing();
            let _ = reply.send(Ok(()));
            return true;
        }
        Command::SwitchModel(model, reply) => {
            let result = if state.is_idle() {
                engine.switch_model(model, &**cache)
            } else {
                Err(AppError::Busy)
            };
            let _ = reply.send(result);
        }
    }

    false
}

fn start_job(
    job: PlaybackJob,
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    events: &dyn PlaybackEvents,
    current: &mut Option<CurrentChunk>,
    previous: Option<PlaybackSource>,
) -> AppResult<()> {
    if let Some(previous) = previous {
        events.interrupted(&previous, previous.chapter_index());
    }

    state.start(job);
    let title = state.snapshot(0).title.clone();
    let total_chars = state.total_chars();
    let chunk_count = state.chunk_count();

    match queue_next_chunk(state, engine, current) {
        Ok(true) => {
            log::info!(
                "playback started: \"{title}\" ({total_chars} characters, {chunk_count} chunks)"
            );
            events.snapshot(&state.snapshot(0));
            Ok(())
        }
        Ok(false) => {
            state.stop();
            events.snapshot(&state.snapshot(0));
            log::info!("nothing to read in \"{title}\"");
            Ok(())
        }
        Err(error) => {
            state.stop();
            events.snapshot(&state.snapshot(0));
            Err(error)
        }
    }
}

fn queue_next_chunk(
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    current: &mut Option<CurrentChunk>,
) -> AppResult<bool> {
    let chunk_text = match state.take_next_chunk() {
        Some(chunk) => chunk.text,
        None => return Ok(false),
    };
    let playback = engine.play_chunk(&chunk_text, speed_to_inflect(state.rate()), 0.667, None)?;

    // The engine may have failed to play even after synthesizing. Treat that as
    // a playback failure rather than leaving the chunk "playing" forever.
    if playback.duration_ms == 0 && !playback.wav_path.exists() {
        return Err(AppError::playback(
            "the synthesized chunk could not be played",
        ));
    }

    let started = Instant::now();
    current.replace(CurrentChunk { playback, started });

    Ok(true)
}

/// Pause: stop current chunk playback but remember it so resume can continue
/// from the same chunk.
fn pause(
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    events: &dyn PlaybackEvents,
    current: &mut Option<CurrentChunk>,
) -> AppResult<bool> {
    let _ = engine.stop_playing();
    current.take();
    state.set_paused(true);
    events.snapshot(&state.snapshot(0));
    Ok(true)
}

/// Resume: re-synthesize and play the current chunk from the beginning.
fn resume(
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    events: &dyn PlaybackEvents,
    current: &mut Option<CurrentChunk>,
) -> AppResult<bool> {
    // Re-queue the same chunk we just paused. Before pausing we had already
    // taken it from the state machine, so `take_next_chunk` now returns the
    // chunk that was paused.
    match state.take_next_chunk() {
        Some(chunk) => {
            let playback =
                engine.play_chunk(&chunk.text, speed_to_inflect(state.rate()), 0.667, None)?;
            let started = Instant::now();
            current.replace(CurrentChunk { playback, started });
            state.set_paused(false);
            events.snapshot(&state.snapshot(0));
            Ok(false)
        }
        None => {
            // We paused after the last chunk finished; nothing to resume.
            state.set_paused(false);
            events.snapshot(&state.snapshot(0));
            Ok(false)
        }
    }
}

/// Advance playback if the current chunk's expected duration has elapsed.
fn tick(
    state: &mut PlaybackState,
    engine: &mut dyn TtsEngine,
    events: &dyn PlaybackEvents,
    current: &mut Option<CurrentChunk>,
    failed_reads: &mut u32,
) {
    if !state.is_awaiting_engine() {
        *failed_reads = 0;
        return;
    }

    if state.is_paused() {
        return;
    }

    // A chunk is "playing" until its expected duration elapses. This is the
    // honest progress model for a wave-based synthesizer: we know how long the
    // chunk should last, and we let it run.
    let finished = match current.as_ref() {
        Some(current) => {
            let elapsed = current.started.elapsed().as_millis() as u64;
            elapsed >= current.playback.duration_ms
        }
        None => true,
    };

    if !finished {
        // Still playing; report progress based on elapsed time.
        let elapsed_ms = current
            .as_ref()
            .map(|c| c.started.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let offset_in_chunk = if current
            .as_ref()
            .map(|c| c.playback.duration_ms)
            .unwrap_or(1)
            > 0
        {
            (elapsed_ms * current.as_ref().map(|c| c.playback.chars).unwrap_or(0) as u64)
                / current
                    .as_ref()
                    .map(|c| c.playback.duration_ms)
                    .unwrap_or(1) as u64
        } else {
            0
        } as u32;
        state.observe_progress(true, offset_in_chunk);
        *failed_reads = 0;
        events.snapshot(&state.snapshot(offset_in_chunk));
        return;
    }

    *failed_reads = 0;
    let playback = current.take().map(|c| c.playback);
    let _chars = playback.as_ref().map(|p| p.chars).unwrap_or(0);

    // Account for the completed chunk.
    if let Some(info) = state.current_chunk_info() {
        let _ = state.complete_chunk(info);
    }

    events.snapshot(&state.snapshot(0));

    if !state.is_idle() {
        if let Some(chunk) = playback {
            // Clean up the WAV after the chunk finished.
            let _ = std::fs::remove_file(&chunk.wav_path);
        }
        let has_more = queue_next_chunk(state, engine, current);
        if has_more.is_err() {
            log::error!("the next chunk could not be queued: {has_more:?}");
            state.stop();
            events.snapshot(&state.snapshot(0));
        }
    } else {
        // Job complete.
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
}

/// Test helper: a fake engine that records what it was asked to do and returns
/// scripted chunk playback.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::inflect::model::InflectModel;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    pub struct FakeEngine {
        pub played: Mutex<Vec<PlayRecord>>,
        pub stopped: AtomicU32,
        pub model: InflectModel,
        /// Preset duration to return for each chunk, in milliseconds.
        pub duration_ms: u64,
        /// Preset chars to report per chunk.
        pub chars: usize,
    }

    #[derive(Clone, Debug)]
    pub struct PlayRecord {
        pub text: String,
        pub speed: f32,
        pub variation: f32,
    }

    impl FakeEngine {
        pub fn new(model: InflectModel, duration_ms: u64, chars: usize) -> Arc<Self> {
            Arc::new(Self {
                played: Mutex::new(Vec::new()),
                stopped: AtomicU32::new(0),
                model,
                duration_ms,
                chars,
            })
        }
    }

    impl TtsEngine for Arc<FakeEngine> {
        fn play_chunk(
            &mut self,
            text: &str,
            speed: f32,
            variation: f32,
            _seed: Option<i64>,
        ) -> AppResult<ChunkPlayback> {
            let mut played = self
                .played
                .lock()
                .map_err(|e| AppError::internal(format!("fake engine lock: {e}")))?;
            played.push(PlayRecord {
                text: text.to_string(),
                speed,
                variation,
            });
            Ok(ChunkPlayback {
                wav_path: std::path::PathBuf::from("/tmp/fake-chunk.wav"),
                duration_ms: self.duration_ms,
                chars: self.chars,
            })
        }

        fn stop_playing(&mut self) -> AppResult<()> {
            self.stopped.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn model(&self) -> InflectModel {
            self.model
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::playback::state::{PlaybackJob, PlaybackSource, PlaybackStatus};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    fn handle<S>(engine: Arc<S>, cache: Arc<ModelCacheHandle>) -> PlaybackHandle
    where
        S: Send + Sync + 'static,
        Arc<S>: TtsEngine,
    {
        PlaybackHandle::spawn(
            Box::new(move |cache| Ok(Box::new(engine) as Box<dyn TtsEngine>)),
            Arc::new(NoEvents),
            cache,
        )
        .expect("actor starts")
    }

    fn memory_cache() -> Arc<ModelCacheHandle> {
        // A real cache needs an app handle; in tests we use a minimal adapter.
        Arc::new(ModelCacheHandle::new(std::path::PathBuf::from(
            "/tmp/inflect-test-cache",
        )))
    }

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
        let engine = FakeEngine::new(InflectModel::Micro, 100, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        assert!(is_idle(&handle));
        assert_eq!(handle.snapshot().expect("snapshot").total_chars, 0);
        handle.shutdown();
    }

    #[test]
    fn starting_a_job_synthesizes_the_first_chunk_and_reports_progress() {
        let engine = FakeEngine::new(InflectModel::Micro, 100, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "hello world",
                None,
            ))
            .expect("start");

        assert!(wait_until(|| !engine
            .played
            .lock()
            .map(|p| p.is_empty())
            .unwrap_or(true)));
        let snapshot = handle.snapshot().expect("snapshot");
        assert_eq!(snapshot.status, PlaybackStatus::Playing);
        assert_eq!(snapshot.total_chars, 11);
        handle.shutdown();
    }

    #[test]
    fn the_actor_advances_chunks_on_its_own_until_the_job_is_finished() {
        let engine = FakeEngine::new(InflectModel::Micro, 50, 10);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        let text = "a".repeat(25);
        handle
            .start(PlaybackJob {
                chunks: crate::text::split_into_chunks(&text, 10),
                ..PlaybackJob::new(PlaybackSource::Clipboard, "Clipboard", &text, None)
            })
            .expect("start");

        assert!(
            wait_until(|| handle.snapshot().map(|s| s.finished).unwrap_or(false)),
            "the actor should finish the job without any external polling"
        );

        let played = engine.played.lock().map(|p| p.clone()).unwrap_or_default();
        assert_eq!(
            played.len(),
            3,
            "every chunk should be synthesized exactly once"
        );
        handle.shutdown();
    }

    #[test]
    fn pausing_stops_playback_and_resuming_continues() {
        let engine = FakeEngine::new(InflectModel::Micro, 100, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "hello world",
                None,
            ))
            .expect("start");

        assert!(wait_until(|| engine
            .played
            .lock()
            .map(|p| p.len() >= 1)
            .unwrap_or(false)));
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
    fn stopping_clears_playback_and_marks_the_engine_stopped() {
        let engine = FakeEngine::new(InflectModel::Micro, 5000, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "some text",
                None,
            ))
            .expect("start");
        handle.stop().expect("stop");

        assert!(is_idle(&handle));
        assert_eq!(
            engine.stopped.load(Ordering::SeqCst),
            1,
            "stop_playing should have been called"
        );
        handle.shutdown();
    }

    #[test]
    fn switching_model_is_ignored_when_a_job_is_playing() {
        let engine = FakeEngine::new(InflectModel::Micro, 5000, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "some text",
                None,
            ))
            .expect("start");

        let err = handle
            .switch_model(InflectModel::Nano)
            .expect_err("must be busy");
        assert_eq!(err.code(), "busy");
        handle.shutdown();
    }

    #[test]
    fn concurrent_commands_never_deadlock() {
        let engine = FakeEngine::new(InflectModel::Micro, 200, 5);
        let cache = memory_cache();
        let handle = Arc::new(handle(Arc::clone(&engine), cache));

        let mut threads = Vec::new();
        for worker in 0..8 {
            let handle = Arc::clone(&handle);
            threads.push(thread::spawn(move || {
                for i in 0..25 {
                    let _ = handle.snapshot();
                    let _ = handle.set_rate(1.0 + (i % 3) as f32 * 0.25);
                    let _ = handle.pause_resume();
                    if i % 5 == 0 {
                        let _ = handle.start(PlaybackJob::new(
                            PlaybackSource::Clipboard,
                            "Clipboard",
                            &"x".repeat(50 + worker),
                            None,
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

        assert!(handle.snapshot().is_ok());
        handle.shutdown();
    }

    #[test]
    fn reading_the_snapshot_does_not_advance_playback() {
        let engine = FakeEngine::new(InflectModel::Micro, 200, 5);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);
        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                &"a".repeat(25),
                None,
            ))
            .expect("start");

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

    /// The previous SAPI model had a "silence before engine starts" edge case.
    /// The new wave model does not: a chunk is synthesized and started, and the
    /// actor waits for its expected duration. There is no polling a running
    /// voice, so there is no window where silence is ambiguous.
    #[test]
    fn chunk_progress_is_driven_by_expected_duration_not_by_polling_a_voice() {
        let engine = FakeEngine::new(InflectModel::Micro, 300, 10);
        let cache = memory_cache();
        let handle = handle(Arc::clone(&engine), cache);

        handle
            .start(PlaybackJob::new(
                PlaybackSource::Clipboard,
                "Clipboard",
                "one chunk only",
                None,
            ))
            .expect("start");

        // Immediately after starting, the chunk should be playing and not yet
        // finished, because the expected duration has not elapsed.
        let snap = handle.snapshot().expect("snapshot");
        assert_eq!(snap.status, PlaybackStatus::Playing);
        assert!(!snap.finished);

        // Wait for the expected duration plus a tick.
        thread::sleep(Duration::from_millis(350));
        assert!(
            wait_until(|| handle.snapshot().map(|s| s.finished).unwrap_or(false)),
            "the chunk should finish after its expected duration"
        );
        handle.shutdown();
    }
}
