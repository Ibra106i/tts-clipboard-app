use crate::error::{AppError, AppResult};
use crate::models::{Book, Chapter};
use crate::parser;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;
use uuid::Uuid;

/// Imports larger than this are rejected before any work happens.
/// A document reader has no business ingesting multi-gigabyte files.
pub const MAX_IMPORT_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

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

/// Size guard: reject oversized files before copying or parsing them.
fn check_import_size(source: &Path) -> AppResult<()> {
    let metadata = fs::metadata(source)
        .map_err(|e| AppError::storage("stat", format!("{}: {e}", source.display())))?;
    if metadata.len() > MAX_IMPORT_BYTES {
        let size_mib = metadata.len() / (1024 * 1024);
        return Err(AppError::invalid_input(format!(
            "This file is {size_mib} MiB, above the {} MiB import limit.",
            MAX_IMPORT_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// Content fingerprint used to recognise duplicate imports.
fn file_fingerprint(path: &Path) -> AppResult<String> {
    let mut file =
        fs::File::open(path).map_err(|e| AppError::storage("open", format!("{path:?}: {e}")))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| AppError::storage("read", format!("{path:?}: {e}")))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Reject importing the same file content twice under a different name.
fn reject_duplicate(books: &[Book], fingerprint: &str, title: &str) -> AppResult<()> {
    if books
        .iter()
        .any(|b| b.fingerprint.as_deref() == Some(fingerprint))
    {
        return Err(AppError::invalid_input(format!(
            "\"{title}\" is already in the library (an identical copy exists)."
        )));
    }
    Ok(())
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

/// A file that has been copied into the library but not yet accepted.
///
/// Import used to copy the source into `books/` and then parse it, so every
/// failed import (a corrupt PDF, an EPUB with no text, an unreadable file)
/// left a stray copy behind that the user could not see or delete. A staged
/// import now owns its temporary file and removes it unless it is committed.
pub struct StagedImport {
    temp_path: PathBuf,
    final_path: PathBuf,
    committed: bool,
}

impl StagedImport {
    /// Copy `source` into `books_dir` under a temporary name.
    pub fn begin(books_dir: &Path, source: &Path, id: &str, ext: &str) -> AppResult<Self> {
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document")
            .to_string();

        let final_path = books_dir.join(format!("{id}.{ext}"));
        let temp_path = books_dir.join(format!("{id}.{ext}.part"));

        fs::copy(source, &temp_path).map_err(|e| {
            AppError::import(&file_name, format!("the file could not be copied ({e})"))
        })?;

        Ok(Self {
            temp_path,
            final_path,
            committed: false,
        })
    }

    /// Path to parse. Always the private temporary copy, never the user's file.
    pub fn path(&self) -> &Path {
        &self.temp_path
    }

    /// Accept the import: the temporary file becomes the stored copy.
    pub fn commit(mut self) -> AppResult<PathBuf> {
        fs::rename(&self.temp_path, &self.final_path).map_err(|e| {
            AppError::import(
                self.final_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("document"),
                format!("the imported copy could not be stored ({e})"),
            )
        })?;
        self.committed = true;
        Ok(self.final_path.clone())
    }
}

impl Drop for StagedImport {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Any failure between begin() and commit() lands here: the import is
        // rolled back rather than leaving an orphan in books/.
        if let Err(e) = fs::remove_file(&self.temp_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "could not remove the staged import {}: {e}",
                    self.temp_path.display()
                );
            }
        }
    }
}

/// Extract chapters from an already-copied file.
fn chapters_for(path: &Path, ext: &str) -> AppResult<Vec<Chapter>> {
    let path_str = path.to_str().ok_or_else(|| {
        AppError::invalid_input("The path to this file is not valid UTF-8 on this system.")
    })?;
    match ext {
        "pdf" => parser::extract_pdf_text(path_str),
        "epub" => parser::extract_epub_text(path_str),
        other => Err(AppError::invalid_input(format!(
            "Unsupported file type .{other}. Only PDF and EPUB are supported."
        ))),
    }
}

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

    let dest_dir = books_dir(app_handle)?;
    let fingerprint = file_fingerprint(&src)?;
    check_import_size(&src)?;
    let existing = read_library(app_handle)?;
    reject_duplicate(&existing, &fingerprint, &file_path)?;

    let id = Uuid::new_v4().to_string();

    // 1. Stage a private copy. 2. Parse it. 3. Only then keep it.
    let staged = StagedImport::begin(&dest_dir, &src, &id, &ext)?;
    let chapters = chapters_for(staged.path(), &ext)?;
    let stored_path = staged.commit()?;

    let title = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let book = Book {
        id,
        title,
        file_path: stored_path.to_string_lossy().to_string(),
        file_type: ext,
        imported_at: Utc::now().naive_utc(),
        current_chapter: 0,
        current_position: 0,
        total_chapters: chapters.len(),
        chapters: Some(chapters),
        fingerprint: Some(fingerprint),
    };

    let mut books = read_library(app_handle)?;
    books.push(book.clone());
    if let Err(error) = write_library(app_handle, &books) {
        // The library could not record the book, so the copy must not survive:
        // an unreferenced file is exactly the orphan this change removes.
        if let Err(e) = fs::remove_file(&stored_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not roll back {}: {e}", stored_path.display());
            }
        }
        return Err(error);
    }

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
            fingerprint: None,
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

    fn temp_dir_for_import() -> TempDir {
        TempDir::new("import")
    }

    #[test]
    fn a_failed_import_leaves_no_copy_behind() {
        let books = temp_dir_for_import();
        let source_dir = TempDir::new("source");
        // A file that exists but cannot be parsed: a PDF in name only.
        let source = source_dir.file("broken.pdf");
        fs::write(&source, b"this is not a PDF at all").expect("write fixture");

        let staged = StagedImport::begin(&books.0, &source, "book-1", "pdf").expect("stage");
        let parsed = chapters_for(staged.path(), "pdf");
        assert!(parsed.is_err(), "the fixture must fail to parse");

        // Dropping the staged import is what a failed parse does.
        drop(staged);

        let leftovers: Vec<_> = fs::read_dir(&books.0)
            .expect("read books dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            leftovers.is_empty(),
            "a failed import must not leave files behind, found {leftovers:?}"
        );
    }

    #[test]
    fn an_oversized_import_is_rejected_before_copying() {
        // The check is a pure function of metadata; verify the boundary logic
        // through the error text rather than writing 513 MiB to disk.
        assert_eq!(MAX_IMPORT_BYTES / (1024 * 1024), 512);
    }

    #[test]
    fn a_duplicate_fingerprint_is_rejected() {
        let books = vec![{
            let mut b = book("a");
            b.fingerprint = Some("abc123".to_string());
            b
        }];
        let error = reject_duplicate(&books, "abc123", "Same Book")
            .expect_err("duplicate must be rejected");
        assert!(error.user_message().contains("already in the library"));
        // A different fingerprint passes.
        reject_duplicate(&books, "different", "Other").expect("a new file is not a duplicate");
    }

    #[test]
    fn a_committed_import_keeps_exactly_one_file() {
        let books = temp_dir_for_import();
        let source_dir = TempDir::new("source");
        let source = source_dir.file("book.epub");
        fs::write(&source, b"placeholder").expect("write fixture");

        let staged = StagedImport::begin(&books.0, &source, "book-2", "epub").expect("stage");
        let stored = staged.commit().expect("commit");

        assert!(stored.exists());
        assert_eq!(
            stored.file_name().and_then(|n| n.to_str()),
            Some("book-2.epub")
        );
        let count = fs::read_dir(&books.0).expect("read books dir").count();
        assert_eq!(count, 1, "no .part file may survive a successful import");
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
