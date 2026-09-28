use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub file_path: String,
    pub file_type: String,
    pub imported_at: NaiveDateTime,
    pub current_chapter: usize,
    pub current_position: usize,
    pub total_chapters: usize,
    #[serde(default)]
    pub chapters: Option<Vec<Chapter>>,
    /// SHA-256 of the stored file; absent for books imported before this
    /// field existed. Used to reject duplicate imports.
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    pub content: String,
}

/// A span of chapter text to read, addressed in Unicode characters rather than
/// bytes or UTF-16 units — the same unit `text::Chunk::start_char` and
/// `PlaybackSnapshot::spoken_chars` are measured in.
///
/// The two ways the reader can start mid-chapter differ only in these fields,
/// which is why they share one type and one command instead of two:
///
/// * Clicking a paragraph sends `end: None` with `align_to_sentence: true`, so
///   reading begins at the first sentence at or after the click.
/// * Reading a selection sends a real `end` with `align_to_sentence: false`, so
///   the chosen characters are read exactly as they are.
///
/// Offsets are resolved against the chapter's own text, so a stale offset from a
/// previous edit is clamped rather than rejected: pointing at nothing is better
/// than refusing to read.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    /// First character to read. Clamped to the chapter length.
    pub start: usize,
    /// One past the last character to read. `None` reads to the end of the
    /// chapter.
    #[serde(default)]
    pub end: Option<usize>,
    /// Whether `start` should move forward to the next sentence boundary.
    #[serde(default)]
    pub align_to_sentence: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_date_time() -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 1, 2)
            .and_then(|date| date.and_hms_opt(3, 4, 5))
            .expect("valid fixed date")
    }

    fn sample_book() -> Book {
        Book {
            id: "book-1".to_string(),
            title: "A Book".to_string(),
            file_path: "C:/data/books/book-1.epub".to_string(),
            file_type: "epub".to_string(),
            imported_at: sample_date_time(),
            current_chapter: 2,
            current_position: 0,
            total_chapters: 3,
            chapters: Some(vec![Chapter {
                index: 0,
                title: "Chapter 1".to_string(),
                content: "Body".to_string(),
            }]),
            fingerprint: None,
        }
    }

    #[test]
    fn book_survives_a_serde_json_round_trip() {
        let book = sample_book();
        let json = serde_json::to_string(&book).expect("serialize");
        let restored: Book = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.id, book.id);
        assert_eq!(restored.title, book.title);
        assert_eq!(restored.imported_at, book.imported_at);
        assert_eq!(restored.current_chapter, book.current_chapter);
        assert_eq!(restored.total_chapters, book.total_chapters);
        assert_eq!(restored.chapters.map(|c| c.len()), Some(1));
    }

    #[test]
    fn chapters_are_optional_for_legacy_records() {
        let json = r#"{
            "id": "book-1",
            "title": "A Book",
            "file_path": "C:/data/books/book-1.pdf",
            "file_type": "pdf",
            "imported_at": "2026-01-02T03:04:05",
            "current_chapter": 0,
            "current_position": 0,
            "total_chapters": 0
        }"#;

        let book: Book = serde_json::from_str(json).expect("legacy record without chapters");
        assert!(book.chapters.is_none());
    }

    #[test]
    fn a_text_range_survives_a_serde_json_round_trip() {
        let range = TextRange {
            start: 4_231,
            end: Some(5_904),
            align_to_sentence: false,
        };
        let json = serde_json::to_string(&range).expect("serialize");
        let restored: TextRange = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, range);
    }

    #[test]
    fn a_text_range_from_the_frontend_keeps_snake_case_wire_names() {
        let json = r#"{"start": 12, "end": 40, "align_to_sentence": true}"#;
        let range: TextRange = serde_json::from_str(json).expect("deserialize");
        assert_eq!(range.start, 12);
        assert_eq!(range.end, Some(40));
        assert!(range.align_to_sentence);

        // Tauri maps camelCase command arguments onto snake_case fields, so the
        // names the webview sends must be the ones on the struct.
        let value = serde_json::to_value(range).expect("to value");
        let object = value.as_object().expect("an object");
        assert!(object.contains_key("align_to_sentence"), "{object:?}");
    }

    #[test]
    fn a_text_range_from_a_reading_gesture_may_omit_the_optional_fields() {
        // A click sends only what it knows; the defaults must describe reading
        // from the click to the end of the chapter, snapped to a sentence.
        let range: TextRange = serde_json::from_str(r#"{"start": 900}"#).expect("deserialize");
        assert_eq!(range.start, 900);
        assert_eq!(range.end, None);
        assert!(!range.align_to_sentence);
    }
}
