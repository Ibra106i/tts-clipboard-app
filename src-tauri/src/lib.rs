mod library;
mod models;
mod parser;
mod tts;

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
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Initialize TTS voice
            {
                let state = app.handle().state::<tts::TtsState>();
                match tts::init_voice() {
                    Ok(voice) => {
                        let mut guard = state.inner().voice.lock().unwrap();
                        *guard = Some(voice);
                        log::info!("TTS voice initialized");
                    }
                    Err(e) => {
                        log::error!("Failed to initialize TTS voice: {e}");
                    }
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

                WebviewWindowBuilder::new(
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
                .build()?;
            }

            // Register global hotkey
            {
                let handle = app.handle().clone();
                let shortcut = app.global_shortcut();
                shortcut.on_shortcut("Ctrl+Shift+Space", move |_app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let h = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_hotkey(&h).await {
                                log::error!("Hotkey handler error: {e}");
                            }
                        });
                    }
                })?;
            }

            // System tray
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};

                let show_item = MenuItemBuilder::with_id("show", "Show").build(app)?;
                let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
                let menu = MenuBuilder::new(app)
                    .item(&show_item)
                    .item(&quit_item)
                    .build()?;

                let handle = app.handle().clone();
                let tray = app.tray_by_id("main-tray");
                if let Some(tray) = tray {
                    tray.set_menu(Some(menu))?;
                    tray.set_tooltip(Some("TTS Library"))?;
                    tray.on_menu_event(move |_app, event| {
                        match event.id().as_ref() {
                            "show" => {
                                if let Some(window) = handle.get_webview_window(MAIN_LABEL) {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                            "quit" => {
                                handle.exit(0);
                            }
                            _ => {}
                        }
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn handle_hotkey(app: &tauri::AppHandle) -> Result<(), String> {
    // Check if TTS is busy
    {
        let state = app.state::<tts::TtsState>();
        let mode = state.inner().mode.lock().map_err(|e| e.to_string())?;
        if *mode != tts::TtsMode::Idle {
            let _ = app.emit("tts-busy", ());
            return Ok(());
        }
    }

    let text = tts::read_clipboard()?;

    if text.trim().is_empty() {
        let _ = app.emit("clipboard-empty", ());
        return Ok(());
    }

    let display_text = if text.len() > MAX_CLIPBOARD_LEN {
        let truncated: String = text.chars().take(MAX_CLIPBOARD_LEN).collect();
        format!("{truncated}... (text truncated)")
    } else {
        text.clone()
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

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

#[tauri::command]
async fn cmd_open_file_dialog(app: tauri::AppHandle) -> Result<Option<String>, String> {
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
async fn cmd_import_book(
    file_path: String,
    app: tauri::AppHandle,
) -> Result<models::Book, String> {
    library::import_book(file_path, &app)
}

#[tauri::command]
async fn cmd_get_library(app: tauri::AppHandle) -> Result<Vec<models::Book>, String> {
    library::get_library(&app)
}

#[tauri::command]
async fn cmd_delete_book(book_id: String, app: tauri::AppHandle) -> Result<(), String> {
    library::delete_book(&book_id, &app)
}

#[tauri::command]
async fn cmd_get_book_chapters(
    book_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<models::Chapter>, String> {
    library::get_book_chapters(&book_id, &app)
}

#[tauri::command]
async fn cmd_save_reading_position(
    book_id: String,
    chapter: usize,
    position: usize,
    app: tauri::AppHandle,
) -> Result<(), String> {
    library::save_reading_position(&book_id, chapter, position, &app)
}
