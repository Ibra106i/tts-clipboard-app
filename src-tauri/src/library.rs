use crate::models::{Book, Chapter};
use crate::parser;
use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use uuid::Uuid;

fn library_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create data dir: {e}"))?;
    Ok(dir)
}

fn library_json_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(library_dir(app_handle)?.join("library.json"))
}

fn books_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = library_dir(app_handle)?.join("books");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create books dir: {e}"))?;
    Ok(dir)
}

fn read_library(app_handle: &tauri::AppHandle) -> Result<Vec<Book>, String> {
    let path = library_json_path(app_handle)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read library.json: {e}"))?;
    serde_json::from_str(&data).map_err(|e| format!("Failed to parse library.json: {e}"))
}

fn write_library(app_handle: &tauri::AppHandle, books: &[Book]) -> Result<(), String> {
    let path = library_json_path(app_handle)?;
    let data = serde_json::to_string_pretty(books)
        .map_err(|e| format!("Failed to serialize library: {e}"))?;
    fs::write(&path, data).map_err(|e| format!("Failed to write library.json: {e}"))
}

pub fn import_book(file_path: String, app_handle: &tauri::AppHandle) -> Result<Book, String> {
    let src = PathBuf::from(&file_path);
    if !src.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext != "pdf" && ext != "epub" {
        return Err("Unsupported file type. Only PDF and EPUB are supported.".to_string());
    }

    let dest_dir = books_dir(app_handle)?;
    let id = Uuid::new_v4().to_string();
    let file_name = format!("{}.{}", id, ext);
    let dest = dest_dir.join(&file_name);
    fs::copy(&src, &dest).map_err(|e| format!("Failed to copy file: {e}"))?;

    let chapters: Vec<Chapter> = match ext.as_str() {
        "pdf" => parser::extract_pdf_text(dest.to_str().unwrap_or_default())?,
        "epub" => parser::extract_epub_text(dest.to_str().unwrap_or_default())?,
        _ => unreachable!(),
    };

    let title = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let book = Book {
        id,
        title,
        file_path: dest.to_string_lossy().to_string(),
        file_type: ext,
        imported_at: Utc::now().naive_utc(),
        current_chapter: 0,
        current_position: 0,
        total_chapters: chapters.len(),
    };

    let mut books = read_library(app_handle)?;
    books.push(book.clone());
    write_library(app_handle, &books)?;

    Ok(book)
}

pub fn get_library(app_handle: &tauri::AppHandle) -> Result<Vec<Book>, String> {
    read_library(app_handle)
}

pub fn delete_book(book_id: &str, app_handle: &tauri::AppHandle) -> Result<(), String> {
    let mut books = read_library(app_handle)?;
    let book = books
        .iter()
        .find(|b| b.id == book_id)
        .cloned()
        .ok_or_else(|| format!("Book not found: {book_id}"))?;

    let path = PathBuf::from(&book.file_path);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Failed to delete file: {e}"))?;
    }

    books.retain(|b| b.id != book_id);
    write_library(app_handle, &books)
}

pub fn get_book_chapters(
    book_id: &str,
    app_handle: &tauri::AppHandle,
) -> Result<Vec<Chapter>, String> {
    let books = read_library(app_handle)?;
    let book = books
        .iter()
        .find(|b| b.id == book_id)
        .ok_or_else(|| format!("Book not found: {book_id}"))?;

    let path = PathBuf::from(&book.file_path);
    if !path.exists() {
        return Err("Book file not found on disk. It may have been moved or deleted.".to_string());
    }

    match book.file_type.as_str() {
        "pdf" => parser::extract_pdf_text(path.to_str().unwrap_or_default()),
        "epub" => parser::extract_epub_text(path.to_str().unwrap_or_default()),
        _ => Err("Unsupported file type".to_string()),
    }
}

pub fn save_reading_position(
    book_id: &str,
    chapter: usize,
    position: usize,
    app_handle: &tauri::AppHandle,
) -> Result<(), String> {
    let mut books = read_library(app_handle)?;
    let book = books
        .iter_mut()
        .find(|b| b.id == book_id)
        .ok_or_else(|| format!("Book not found: {book_id}"))?;

    book.current_chapter = chapter;
    book.current_position = position;
    write_library(app_handle, &books)
}
