use crate::models::Chapter;
use lopdf::Document;

pub fn extract_pdf_text(file_path: &str) -> Result<Vec<Chapter>, String> {
    let doc = Document::load(file_path).map_err(|e| format!("Failed to load PDF: {e}"))?;

    let pages = doc.get_pages();
    let mut chapters = Vec::new();

    for page_num in pages.keys() {
        let page_num_u32 = *page_num as u32;
        let text = doc
            .extract_text(&[page_num_u32])
            .map_err(|e| format!("Failed to extract page {page_num}: {e}"))?;

        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            chapters.push(Chapter {
                index: chapters.len(),
                title: format!("Page {}", page_num + 1),
                content: trimmed,
            });
        }
    }

    if chapters.is_empty() {
        return Err("Cannot extract text. This might be a scanned PDF.".to_string());
    }

    Ok(chapters)
}

pub fn extract_epub_text(file_path: &str) -> Result<Vec<Chapter>, String> {
    let mut archive =
        epub::doc::EpubDoc::new(file_path).map_err(|e| format!("Failed to load EPUB: {e}"))?;

    let spine = archive.spine.clone();
    let mut chapters = Vec::new();

    for (i, spine_item) in spine.iter().enumerate() {
        let res_id = match &spine_item.id {
            Some(id) => id.clone(),
            None => continue,
        };
        let data = match archive.get_resource(&res_id) {
            Some((bytes, _mime)) => bytes,
            None => continue,
        };

        let html = String::from_utf8_lossy(&data).to_string();
        let text = strip_html(&html);

        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            let title = format!("Chapter {}", i + 1);
            chapters.push(Chapter {
                index: chapters.len(),
                title,
                content: trimmed,
            });
        }
    }

    if chapters.is_empty() {
        return Err("No readable content found in EPUB.".to_string());
    }

    Ok(chapters)
}

fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut skip_content = false;

    let mut chars = html.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag_name = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' || next == ' ' {
                    break;
                }
                tag_name.push(next);
                chars.next();
            }
            // Skip to end of tag
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                chars.next();
            }

            let lower = tag_name.to_lowercase();
            if lower == "script" || lower == "style" {
                skip_content = true;
            } else if lower == "/script" || lower == "/style" {
                skip_content = false;
            } else if !skip_content {
                result.push('\n');
            }
            continue;
        }

        if skip_content {
            continue;
        }

        result.push(ch);
    }

    // Collapse multiple newlines
    let mut cleaned = String::new();
    let mut prev_was_newline = false;
    for ch in result.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                cleaned.push(ch);
            }
            prev_was_newline = true;
        } else {
            cleaned.push(ch);
            prev_was_newline = false;
        }
    }

    cleaned
}

pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_chars {
            chunks.push(remaining.to_string());
            break;
        }

        let cut_at = remaining[..max_chars]
            .rfind(|c: char| c == '.' || c == '!' || c == '?')
            .map(|i| i + 1)
            .unwrap_or_else(|| remaining[..max_chars].rfind(' ').unwrap_or(max_chars));

        chunks.push(remaining[..cut_at].to_string());
        remaining = remaining[cut_at..].trim_start();
    }

    chunks
}
