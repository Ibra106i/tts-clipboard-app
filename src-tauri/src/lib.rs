mod error;
mod inflect;
mod library;
mod models;
mod parser;
mod playback;
mod redact;
mod text;

use crate::error::{AppError, AppResult};
use crate::inflect::commands::ModelState;
use crate::inflect::model::{InflectModel, ModelCacheHandle};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const OVERLAY_LABEL: &str = "overlay";
const MAIN_LABEL: &str = "main";
/// Longest clipboard payload that will be read aloud, in characters.
const MAX_CLIPBOARD_CHARS: usize = 5000;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A panic on a background thread - the playback actor especially - is
    // otherwise only visible on stderr, which a developer console supplies and
    // a packaged app does not. Routing it through `log` puts the message and
    // its location in the same rotating file as everything else, so a crash
    // report is a log the user can send rather than a blank window.
    std::panic::set_hook(Box::new(|info| {
        let where_ = info
            .location()
            .map_or_else(|| "unknown location".to_string(), |l| l.to_string());
        log::error!("PANIC at {where_}: {info}");
        log::error!("backtrace:\n{}", std::backtrace::Backtrace::force_capture());
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // Logging is always on, in debug and release. A release build with
            // no logs turned every recoverable failure into an unexplained
            // symptom for the user.
            //
            // The global floor is Info, and only this app's own crate is opened
            // up to Debug. A global Debug floor looks harmless but is not: a
            // single PDF import emitted 1202 `lopdf` lines, and the windowing
            // layer chattered about event-loop redraws on every frame. Each of
            // those is two synchronous writes - one to the log file, one to
            // stdout - landing on the thread that owns the window. 95% of a
            // recent log was `lopdf`. Dependencies are held at Info so the app's
            // own diagnostics stay available without a third-party crate being
            // able to spend the UI thread's time.
            let mut logging = tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("tts-clipboard-app".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3));
            if cfg!(debug_assertions) {
                logging = logging.level_for("app_lib", log::LevelFilter::Debug);
            }
            app.handle().plugin(logging.build())?;
            log::info!(
                "starting {} v{} ({})",
                app.package_info().name,
                app.package_info().version,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                }
            );
            match app.path().app_log_dir() {
                Ok(dir) => log::info!("log directory: {}", redact::path(&dir)),
                Err(e) => log::warn!("log directory unavailable: {e}"),
            }

            // Start the playback thread. It creates the local Inflect engine
            // inside itself and owns the playback state machine for the lifetime
            // of the process.
            {
                let events = std::sync::Arc::new(PlaybackEventsSink {
                    app: app.handle().clone(),
                });
                let cache = Arc::new(ModelCacheHandle::new(inflect::model_cache_dir(
                    &app.handle(),
                )?));
                match playback::PlaybackHandle::spawn(
                    Box::new(inflect::engine_factory(cache.clone())),
                    events,
                    cache.clone(),
                ) {
                    Ok(handle) => {
                        app.manage(handle);
                        app.manage(cache);
                        app.manage(ModelState::new(InflectModel::Micro));
                    }
                    Err(error) => log::error!("Playback is unavailable: {error}"),
                }
            }

            // Create overlay window (hidden by default)
            {
                let overlay_url = if cfg!(debug_assertions) {
                    "http://localhost:5173".to_string()
                } else {
                    "index.html".to_string()
                };

                let monitor = app
                    .primary_monitor()
                    .ok()
                    .flatten()
                    .or_else(|| app.available_monitors().ok()?.into_iter().next());

                let (pos_x, pos_y) = if let Some(m) = monitor {
                    let size = m.size();
                    let scale = m.scale_factor();
                    let w = (size.width as f64 / scale) as i32;
                    let h = (size.height as f64 / scale) as i32;
                    (w - 460, h - 290)
                } else {
                    (800, 600)
                };

                use tauri::WebviewUrl;
                use tauri::WebviewWindowBuilder;

                match WebviewWindowBuilder::new(
                    app.handle(),
                    OVERLAY_LABEL,
                    WebviewUrl::App(overlay_url.into()),
                )
                .title("TTS Overlay")
                .inner_size(450.0, 280.0)
                .position(pos_x as f64, pos_y as f64)
                .resizable(false)
                .decorations(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .transparent(true)
                .visible(false)
                .build()
                {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("Failed to create overlay window: {e}");
                    }
                }
            }

            // Register global hotkey
            {
                let handle = app.handle().clone();
                let shortcut = app.global_shortcut();
                if let Err(e) =
                    shortcut.on_shortcut("Ctrl+Shift+Space", move |_app, _shortcut, event| {
                        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                            let h = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = handle_hotkey(&h).await {
                                    log::error!("Hotkey handler error: {e}");
                                }
                            });
                        }
                    })
                {
                    log::error!("Failed to register global shortcut: {e}");
                }
            }

            // System tray
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};

                let show_item = match MenuItemBuilder::with_id("show", "Show").build(app) {
                    Ok(item) => item,
                    Err(e) => {
                        log::error!("Failed to create show menu item: {e}");
                        return Ok(());
                    }
                };
                let logs_item = match MenuItemBuilder::with_id("logs", "Open log folder").build(app)
                {
                    Ok(item) => item,
                    Err(e) => {
                        log::error!("Failed to create logs menu item: {e}");
                        return Ok(());
                    }
                };
                let quit_item = match MenuItemBuilder::with_id("quit", "Quit").build(app) {
                    Ok(item) => item,
                    Err(e) => {
                        log::error!("Failed to create quit menu item: {e}");
                        return Ok(());
                    }
                };
                let menu = match MenuBuilder::new(app)
                    .item(&show_item)
                    .item(&logs_item)
                    .item(&quit_item)
                    .build()
                {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to create tray menu: {e}");
                        return Ok(());
                    }
                };

                let handle = app.handle().clone();
                let tray = app.tray_by_id("main-tray");
                if let Some(tray) = tray {
                    let _ = tray.set_menu(Some(menu));
                    let _ = tray.set_tooltip(Some("TTS Library"));
                    tray.on_menu_event(move |_app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = handle.get_webview_window(MAIN_LABEL) {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "logs" => {
                            if let Err(e) = open_log_folder(&handle) {
                                log::error!("Failed to open the log folder: {e}");
                            }
                        }
                        "quit" => {
                            log::info!("shutting down");
                            // Stop the speech engine and join the playback
                            // thread before the process goes away.
                            if let Some(playback) = handle.try_state::<playback::PlaybackHandle>() {
                                playback.shutdown();
                            }
                            handle.exit(0);
                        }
                        _ => {}
                    });
                }
            }

            // Intercept main window close → hide instead of quit
            {
                let handle = app.handle().clone();
                if let Some(window) = app.get_webview_window(MAIN_LABEL) {
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            if let Some(w) = handle.get_webview_window(MAIN_LABEL) {
                                let _ = w.hide();
                            }
                        }
                    });
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            playback::commands::speak_text,
            playback::commands::speak_book_chapter,
            playback::commands::pause_resume_tts,
            playback::commands::set_tts_rate,
            playback::commands::playback_get_state,
            playback::commands::stop_tts,
            inflect::commands::tts_model_info,
            inflect::commands::tts_switch_model,
            cmd_open_file_dialog,
            cmd_import_book,
            cmd_get_library,
            cmd_delete_book,
            cmd_get_book_chapters,
            cmd_save_reading_position,
            cmd_open_logs_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn handle_hotkey(app: &tauri::AppHandle) -> AppResult<()> {
    // No pre-flight busy check: `start` itself returns `Busy`, and asking
    // twice was how a second, racing async task got launched in the first
    // place. One decision point, one owner.
    let text = playback::read_clipboard()?;

    if text.trim().is_empty() {
        log::info!("hotkey pressed with an empty clipboard");
        let _ = app.emit("clipboard-empty", ());
        return Ok(());
    }

    // The clipboard content itself is never logged: it is the most private data
    // this application touches. Only its size is recorded.
    log::info!(
        "hotkey pressed with {} characters on the clipboard",
        text.chars().count()
    );

    // Characters, not bytes, on both sides of the comparison: the limit is a
    // reader-facing length, and the old byte comparison truncated non-ASCII
    // text at a different point than it reported.
    let display_text = if text.chars().count() > MAX_CLIPBOARD_CHARS {
        let truncated: String = text.chars().take(MAX_CLIPBOARD_CHARS).collect();
        format!("{truncated}... (text truncated)")
    } else {
        text
    };

    // Exactly one owner starts playback: this handler. It used to start
    // playback *and* emit a `speak-trigger` event that made the overlay call
    // `speak_text` again, so one of the two calls always lost the race and the
    // user saw a spurious "busy" toast with a progress bar that never moved.
    // The overlay is now a pure view of the `playback-state` events the actor
    // emits, and never initiates speech itself.
    let handle = app.state::<playback::PlaybackHandle>();
    match handle.start(playback::PlaybackJob::new(
        playback::PlaybackSource::Clipboard,
        "Clipboard",
        &display_text,
        None,
    )) {
        Ok(()) => {
            if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        }
        Err(error) if error.is_busy() => {
            let _ = app.emit("tts-busy", ());
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Bridges actor events to the webview.
///
/// The actor emits through this instead of holding an `AppHandle`, which keeps
/// the playback thread independent of Tauri and makes it testable.
struct PlaybackEventsSink {
    app: tauri::AppHandle,
}

impl playback::PlaybackEvents for PlaybackEventsSink {
    fn snapshot(&self, snapshot: &playback::PlaybackSnapshot) {
        // Fired on every state change and a few times a second while speaking.
        // This is what replaced the 100 ms polling loop in the frontend.
        // Fired on every state change and a few times a second while speaking.
        // This is what replaced the 100 ms polling loop in the frontend.
        //
        // There is deliberately only this one event. A second channel,
        // `playback-progress`, used to be emitted alongside it carrying
        // `spoken_chars` and `total_chars` - four times a second, forever, to no
        // listener anywhere in the app. It cost a `serde_json::Value`
        // allocation, a serialisation and a webview message dispatch per tick to
        // deliver nothing. Anything that wants progress reads it off this
        // payload, which already carries both fields.
        let _ = self.app.emit("playback-state", snapshot);
    }

    fn interrupted(&self, previous: &playback::PlaybackSource, chapter_index: Option<usize>) {
        let _ = self.app.emit(
            "tts-interrupted",
            serde_json::json!({
                "previous_source": match previous {
                    playback::PlaybackSource::Clipboard => "clipboard".to_string(),
                    playback::PlaybackSource::Book { book_id, .. } => format!("book:{book_id}"),
                },
                "previous_chapter_index": chapter_index,
            }),
        );
    }

    fn chapter_finished(&self, book_id: &str, chapter_index: usize, total_chapters: usize) {
        // Replaces the window CustomEvent that used to carry this as a string.
        let _ = self.app.emit(
            "chapter-finished",
            serde_json::json!({
                "book_id": book_id,
                "chapter_index": chapter_index,
                "total_chapters": total_chapters,
            }),
        );
    }

    fn engine_lost(&self, message: &str) {
        // The window needs both halves: the state change that has already been
        // emitted stops the playhead, and this says why it stopped. Without a
        // reason, silence is indistinguishable from a hang.
        let _ = self.app.emit(
            "playback-engine-lost",
            serde_json::json!({ "message": message }),
        );
    }
}

// ── Library Commands ──────────────────────────────────────────────

/// Log a command's outcome in one place, so every backend failure is recorded
/// with its diagnostic detail regardless of where it originated.
fn log_result<T>(operation: &str, result: &AppResult<T>) {
    match result {
        Ok(_) => log::debug!("{operation}: ok"),
        Err(error) => log::error!(
            "{operation}: failed [{}] {} (detail: {:?})",
            error.code(),
            error.user_message(),
            error.detail()
        ),
    }
}

/// Where the running application writes its rotated log files.
fn log_directory(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_log_dir()
        .map_err(|e| AppError::storage("locate", format!("log directory: {e}")))
}

#[cfg(windows)]
fn open_log_folder(app: &tauri::AppHandle) -> AppResult<()> {
    let dir = log_directory(app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::storage("create", format!("log directory: {e}")))?;
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| AppError::storage("open", format!("log directory: {e}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn open_log_folder(_app: &tauri::AppHandle) -> AppResult<()> {
    Err(AppError::unsupported_platform("Opening the log folder"))
}

#[tauri::command]
async fn cmd_open_logs_folder(app: tauri::AppHandle) -> AppResult<()> {
    open_log_folder(&app)
}

#[tauri::command]
async fn cmd_open_file_dialog(app: tauri::AppHandle) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;

    // The dialog API is callback-based, so bridge it to async. The blocking
    // wait runs on the blocking pool: waiting inline used to park an async
    // worker thread for the whole time the dialog was open, stalling every
    // other command (including playback polling).
    tauri::async_runtime::spawn_blocking(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        app.dialog()
            .file()
            .add_filter("PDF & EPUB", &["pdf", "epub"])
            .pick_file(move |path| {
                let _ = tx.send(path);
            });

        let result: Option<tauri_plugin_dialog::FilePath> = rx
            .recv()
            .map_err(|_| AppError::internal("file dialog closed without a result"))?;

        match result {
            Some(path) => Ok(Some(path.to_string())),
            None => Ok(None),
        }
    })
    .await
    .map_err(|e| AppError::internal(format!("file dialog task aborted: {e}")))?
}

#[tauri::command]
async fn cmd_import_book(file_path: String, app: tauri::AppHandle) -> AppResult<models::Book> {
    // Parsing a large PDF can take seconds; run it on the blocking pool so
    // the async runtime keeps serving other commands meanwhile.
    let result =
        tauri::async_runtime::spawn_blocking(move || library::import_book(file_path, &app))
            .await
            .map_err(|e| AppError::internal(format!("import task aborted: {e}")))?;

    match &result {
        Ok(book) => log::info!(
            "imported \"{}\" ({} chapters, {})",
            book.title,
            book.total_chapters,
            book.file_type
        ),
        Err(error) => log::error!(
            "import failed [{}] {} (detail: {:?})",
            error.code(),
            error.user_message(),
            error.detail()
        ),
    }
    result
}

#[tauri::command]
async fn cmd_get_library(app: tauri::AppHandle) -> AppResult<Vec<models::Book>> {
    // Blocking file read, and the library can be large. Off the runtime worker.
    let result = tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let result = library::get_library(&app);
        log::debug!("get library took {:?}", started.elapsed());
        result
    })
    .await
    .map_err(|_| AppError::internal("the library read could not be queued"))?;
    log_result("get library", &result);
    result
}

#[tauri::command]
async fn cmd_delete_book(book_id: String, app: tauri::AppHandle) -> AppResult<()> {
    let log_id = book_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || library::delete_book(&book_id, &app))
        .await
        .map_err(|_| AppError::internal("the delete could not be queued"))?;
    match &result {
        Ok(()) => log::info!("deleted book {log_id}"),
        Err(error) => log::error!(
            "delete failed [{}] {} (detail: {:?})",
            error.code(),
            error.user_message(),
            error.detail()
        ),
    }
    result
}

#[tauri::command]
async fn cmd_get_book_chapters(
    book_id: String,
    app: tauri::AppHandle,
) -> AppResult<Vec<models::Chapter>> {
    // Reads the library file and, on a cache miss, re-parses the whole source
    // book. Both are blocking, and this used to run inline on a runtime worker.
    let label = format!("get chapters for {book_id}");
    let result = tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let result = library::get_book_chapters(&book_id, &app);
        log::debug!("get book chapters took {:?}", started.elapsed());
        result
    })
    .await
    .map_err(|_| AppError::internal("the chapter request could not be queued"))?;
    log_result(&label, &result);
    result
}

#[tauri::command]
async fn cmd_save_reading_position(
    book_id: String,
    chapter: usize,
    position: usize,
    app: tauri::AppHandle,
) -> AppResult<()> {
    // Read-modify-write of the library file. Blocking, and previously inline.
    let label = format!("save position {book_id}/{chapter}");
    let result = tauri::async_runtime::spawn_blocking(move || {
        library::save_reading_position(&book_id, chapter, position, &app)
    })
    .await
    .map_err(|_| AppError::internal("the position save could not be queued"))?;
    log_result(&label, &result);
    result
}
