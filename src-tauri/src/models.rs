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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    pub content: String,
}
