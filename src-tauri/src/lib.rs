mod error;
mod library;
mod models;
mod parser;
mod text;

// The playback module is the only Windows-specific part of the crate. On other
// platforms a stub with the same surface reports `unsupported_platform`.
#[cfg(windows)]
mod tts;
#[cfg(not(windows))]
#[path = "tts_stub.rs"]
mod tts;

use crate::error::{AppError, AppResult};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const OVERLAY_LABEL: &str = "overlay";
const MAIN_LABEL: &str = "main";
const MAX_CLIPBOARD_LEN: usize = 5000;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tts::init_com();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(tts::TtsState::new())
        .setup(|app| {
            // Logging is always on, in debug and release. A release build with
            // no logs turned every recoverable failure into an unexplained
            // symptom for the user.
            let level = if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(level)
                    .targets([
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                            file_name: Some("tts-clipboard-app".into()),
                        }),
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    ])
                    .max_file_size(2_000_000)
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                    .build(),
            )?;
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
                Ok(dir) => log::info!("log directory: {}", dir.display()),
                Err(e) => log::warn!("log directory unavailable: {e}"),
            }

            // Initialize the speech voice. All locking happens inside the
            // playback module, so this never touches its internals and never
            // panics on a poisoned mutex.
            {
                let state = app.handle().state::<tts::TtsState>();
                if let Err(error) = tts::init_voice(&state) {
                    log::error!("Speech voice unavailable: {error}");
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
            tts::speak_text,
            tts::speak_book_chapter,
            tts::pause_resume_tts,
            tts::set_tts_rate,
            tts::get_speech_position,
            tts::stop_tts,
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
    // Check if TTS is busy
    {
        let state = app.state::<tts::TtsState>();
        if !tts::is_idle(&state)? {
            let _ = app.emit("tts-busy", ());
            return Ok(());
        }
    }

    let text = tts::read_clipboard()?;

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

    let display_text = if text.len() > MAX_CLIPBOARD_LEN {
        let truncated: String = text.chars().take(MAX_CLIPBOARD_LEN).collect();
        format!("{truncated}... (text truncated)")
    } else {
        text
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();

    let _ = app.emit(
        "speak-trigger",
        serde_json::json!({
            "text": display_text,
            "timestamp": timestamp,
        }),
    );

    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    let state = app.state::<tts::TtsState>();
    tts::speak_text(display_text, state)?;

    Ok(())
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

    let result = app
        .dialog()
        .file()
        .add_filter("PDF & EPUB", &["pdf", "epub"])
        .blocking_pick_file();

    match result {
        Some(path) => Ok(Some(path.to_string())),
        None => Ok(None),
    }
}

#[tauri::command]
async fn cmd_import_book(file_path: String, app: tauri::AppHandle) -> AppResult<models::Book> {
    let result = library::import_book(file_path, &app);
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
    let result = library::get_library(&app);
    log_result("get library", &result);
    result
}

#[tauri::command]
async fn cmd_delete_book(book_id: String, app: tauri::AppHandle) -> AppResult<()> {
    let result = library::delete_book(&book_id, &app);
    match &result {
        Ok(()) => log::info!("deleted book {book_id}"),
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
    let result = library::get_book_chapters(&book_id, &app);
    log_result(&format!("get chapters for {book_id}"), &result);
    result
}

#[tauri::command]
async fn cmd_save_reading_position(
    book_id: String,
    chapter: usize,
    position: usize,
    app: tauri::AppHandle,
) -> AppResult<()> {
    let result = library::save_reading_position(&book_id, chapter, position, &app);
    log_result(&format!("save position {book_id}/{chapter}"), &result);
    result
}
