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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    pub content: String,
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
}
