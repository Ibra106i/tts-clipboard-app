use crate::error::{AppError, AppResult};
use crate::models::{Book, Chapter};
use crate::parser;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;
use uuid::Uuid;

// ── Path-pure core ──────────────────────────────────────────────────
// These functions take explicit paths instead of a Tauri handle so the
// persistence rules can be unit tested directly (and later reused by the
// backup/atomic-write layer).

/// Read the library file. A missing file is an empty library, not an error.
pub fn read_library_file(path: &Path) -> AppResult<Vec<Book>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(path)
        .map_err(|e| AppError::storage("read", format!("{}: {e}", path.display())))?;
    serde_json::from_str(&data)
        .map_err(|e| AppError::storage("parse", format!("{}: {e}", path.display())))
}

pub fn write_library_file(path: &Path, books: &[Book]) -> AppResult<()> {
    let data = serde_json::to_string_pretty(books)
        .map_err(|e| AppError::storage("serialize", e.to_string()))?;
    fs::write(path, data)
        .map_err(|e| AppError::storage("write", format!("{}: {e}", path.display())))
}

/// Look up a book by id, or report a structured not-found error.
pub fn find_book(books: &[Book], book_id: &str) -> AppResult<Book> {
    books
        .iter()
        .find(|b| b.id == book_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("Book", book_id))
}

fn index_of(books: &[Book], book_id: &str) -> AppResult<usize> {
    books
        .iter()
        .position(|b| b.id == book_id)
        .ok_or_else(|| AppError::not_found("Book", book_id))
}

// ── Tauri-scoped helpers ────────────────────────────────────────────

fn library_dir(app_handle: &tauri::AppHandle) -> AppResult<PathBuf> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| AppError::storage("locate", format!("app data directory: {e}")))?;
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::storage("create", format!("data directory: {e}")))?;
    Ok(dir)
}

fn library_json_path(app_handle: &tauri::AppHandle) -> AppResult<PathBuf> {
    Ok(library_dir(app_handle)?.join("library.json"))
}

fn books_dir(app_handle: &tauri::AppHandle) -> AppResult<PathBuf> {
    let dir = library_dir(app_handle)?.join("books");
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::storage("create", format!("books directory: {e}")))?;
    Ok(dir)
}

fn read_library(app_handle: &tauri::AppHandle) -> AppResult<Vec<Book>> {
    read_library_file(&library_json_path(app_handle)?)
}

fn write_library(app_handle: &tauri::AppHandle, books: &[Book]) -> AppResult<()> {
    write_library_file(&library_json_path(app_handle)?, books)
}

// ── Commands' backing functions ─────────────────────────────────────

pub fn import_book(file_path: String, app_handle: &tauri::AppHandle) -> AppResult<Book> {
    let src = PathBuf::from(&file_path);
    if !src.exists() {
        return Err(AppError::not_found("File", file_path));
    }

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext != "pdf" && ext != "epub" {
        return Err(AppError::invalid_input(
            "Unsupported file type. Only PDF and EPUB are supported.",
        ));
    }

    let file_name = src
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
        .to_string();

    let dest_dir = books_dir(app_handle)?;
    let id = Uuid::new_v4().to_string();
    let stored_name = format!("{}.{}", id, ext);
    let dest = dest_dir.join(&stored_name);
    fs::copy(&src, &dest)
        .map_err(|e| AppError::import(&file_name, format!("the file could not be copied ({e})")))?;

    let dest_str = dest.to_str().unwrap_or_default();
    let chapters: Vec<Chapter> = match ext.as_str() {
        "pdf" => parser::extract_pdf_text(dest_str)?,
        "epub" => parser::extract_epub_text(dest_str)?,
        _ => unreachable!("extension was validated above"),
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
        chapters: Some(chapters),
    };

    let mut books = read_library(app_handle)?;
    books.push(book.clone());
    write_library(app_handle, &books)?;

    Ok(book)
}

pub fn get_library(app_handle: &tauri::AppHandle) -> AppResult<Vec<Book>> {
    read_library(app_handle)
}

pub fn delete_book(book_id: &str, app_handle: &tauri::AppHandle) -> AppResult<()> {
    let mut books = read_library(app_handle)?;
    let book = find_book(&books, book_id)?;

    let path = PathBuf::from(&book.file_path);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| AppError::storage("delete", format!("{}: {e}", path.display())))?;
    }

    books.retain(|b| b.id != book_id);
    write_library(app_handle, &books)
}

pub fn get_book_chapters(book_id: &str, app_handle: &tauri::AppHandle) -> AppResult<Vec<Chapter>> {
    let mut books = read_library(app_handle)?;
    let index = index_of(&books, book_id)?;

    if let Some(chapters) = books[index].chapters.as_ref() {
        if !chapters.is_empty() {
            return Ok(chapters.clone());
        }
    }

    let book = &books[index];
    let path = PathBuf::from(&book.file_path);
    if !path.exists() {
        return Err(AppError::document(
            book.title.clone(),
            "the stored copy of this book is missing from disk",
        ));
    }

    let chapters = match book.file_type.as_str() {
        "pdf" => parser::extract_pdf_text(path.to_str().unwrap_or_default())?,
        "epub" => parser::extract_epub_text(path.to_str().unwrap_or_default())?,
        other => {
            return Err(AppError::internal(format!(
                "book {} has unsupported file type {other}",
                book.id
            )))
        }
    };

    books[index].chapters = Some(chapters.clone());
    books[index].total_chapters = chapters.len();
    write_library(app_handle, &books)?;

    Ok(chapters)
}

pub fn save_reading_position(
    book_id: &str,
    chapter: usize,
    position: usize,
    app_handle: &tauri::AppHandle,
) -> AppResult<()> {
    let mut books = read_library(app_handle)?;
    let index = index_of(&books, book_id)?;

    books[index].current_chapter = chapter;
    books[index].current_position = position;
    write_library(app_handle, &books)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("tts-clipboard-test-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn book(id: &str) -> Book {
        Book {
            id: id.to_string(),
            title: format!("Book {id}"),
            file_path: format!("C:/books/{id}.epub"),
            file_type: "epub".to_string(),
            imported_at: Utc::now().naive_utc(),
            current_chapter: 3,
            current_position: 0,
            total_chapters: 12,
            chapters: None,
        }
    }

    #[test]
    fn a_missing_library_file_is_an_empty_library_not_an_error() {
        let dir = TempDir::new("missing");
        let books = read_library_file(&dir.file("library.json")).expect("missing file is fine");
        assert!(books.is_empty());
    }

    #[test]
    fn library_written_then_read_preserves_metadata() {
        let dir = TempDir::new("roundtrip");
        let path = dir.file("library.json");
        let original = vec![book("a"), book("b")];

        write_library_file(&path, &original).expect("write");
        let restored = read_library_file(&path).expect("read");

        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].id, "a");
        assert_eq!(restored[1].current_chapter, 3);
        assert_eq!(restored[1].total_chapters, 12);
    }

    #[test]
    fn a_corrupt_library_reports_a_storage_error_instead_of_silently_resetting() {
        let dir = TempDir::new("corrupt");
        let path = dir.file("library.json");
        fs::write(&path, b"{ this is not json").expect("write fixture");

        let error = read_library_file(&path).expect_err("corrupt file must fail");
        assert_eq!(error.code(), "storage_failed");
        assert!(error.user_message().contains("library.json"));
    }

    #[test]
    fn find_book_and_index_of_agree_on_identity() {
        let books = vec![book("a"), book("b")];

        assert_eq!(find_book(&books, "a").expect("found").title, "Book a");
        assert_eq!(index_of(&books, "b").expect("found"), 1);

        assert_eq!(
            index_of(&books, "zz").expect_err("absent").code(),
            "not_found"
        );
    }

    #[test]
    fn writing_to_an_unwritable_location_reports_a_storage_error() {
        let dir = TempDir::new("unwritable");
        // A directory can never be replaced by a file write, so this exercises
        // the IO error path deterministically.
        let error = write_library_file(&dir.0, &[book("a")]).expect_err("write must fail");
        assert_eq!(error.code(), "storage_failed");
        assert!(error.detail().is_some());
    }
}
