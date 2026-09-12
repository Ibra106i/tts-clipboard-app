use crate::models::Chapter;
use lopdf::Document;
use scraper::{Html, Selector};

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
        let text = extract_epub_html_filtered(&html);

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

fn lower_class_split(class_attr: &str) -> Vec<String> {
    class_attr
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .collect()
}

fn is_excluded_element(element: scraper::ElementRef) -> bool {
    if let Some(epub_type) = element.value().attr("epub:type") {
        let lower = epub_type.to_lowercase();
        if lower.contains("footnote") || lower.contains("endnote") {
            return true;
        }
    }

    if element.value().name() == "aside" {
        return true;
    }

    let exact_targets = ["fn"];
    let substring_targets = ["footnote", "endnote", "note"];
    let false_positives = ["noteworthy", "notebook", "notepad"];

    if let Some(class_attr) = element.value().attr("class") {
        for token in lower_class_split(class_attr) {
            if false_positives.contains(&token.as_str()) {
                continue;
            }
            if exact_targets.contains(&token.as_str()) {
                return true;
            }
            if substring_targets.iter().any(|t| token.contains(t)) {
                return true;
            }
        }
    }

    false
}

fn is_descendant_of_excluded(element: scraper::ElementRef) -> bool {
    let mut current = element.parent();
    while let Some(parent) = current {
        if let Some(parent_elem) = scraper::ElementRef::wrap(parent) {
            if is_excluded_element(parent_elem) {
                return true;
            }
            current = parent_elem.parent();
        } else {
            break;
        }
    }
    false
}

fn extract_epub_html_filtered(html: &str) -> String {
    let document = Html::parse_document(html);

    let epub_type_selector = Selector::parse("[epub\\:type]").unwrap();
    let has_epub_type_markers = document.select(&epub_type_selector).any(|elem| {
        if let Some(epub_type) = elem.value().attr("epub:type") {
            let lower = epub_type.to_lowercase();
            lower.contains("footnote") || lower.contains("endnote")
        } else {
            false
        }
    });

    let aside_selector = Selector::parse("aside").unwrap();
    let has_aside = document.select(&aside_selector).next().is_some();

    let mut has_class_markers = false;
    let class_selector = Selector::parse("[class]").unwrap();
    for elem in document.select(&class_selector) {
        if is_excluded_element(elem) {
            has_class_markers = true;
            break;
        }
    }

    if !has_epub_type_markers && !has_aside && !has_class_markers {
        return strip_html(html);
    }

    let body_selector = Selector::parse("body").unwrap();
    if let Some(body) = document.select(&body_selector).next() {
        extract_text_filtered(body)
    } else {
        let root = document.root_element();
        let mut result = String::new();
        for child in root.children() {
            if let Some(child_elem) = scraper::ElementRef::wrap(child) {
                if child_elem.value().name() == "head" {
                    continue;
                }
                let child_text = extract_text_filtered(child_elem);
                result.push_str(&child_text);
            }
        }
        result
    }
}

fn extract_text_filtered(element: scraper::ElementRef) -> String {
    let mut result = String::new();
    let block_tags = ["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "pre", "br"];

    for child in element.children() {
        match child.value() {
            scraper::Node::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    result.push_str(trimmed);
                    result.push(' ');
                }
            }
            scraper::Node::Element(elem) => {
                if let Some(child_elem) = scraper::ElementRef::wrap(child) {
                    if is_excluded_element(child_elem) || is_descendant_of_excluded(child_elem) {
                        continue;
                    }

                    let tag = elem.name().to_lowercase();
                    let is_block = block_tags.contains(&tag.as_str());

                    if is_block && !result.is_empty() && !result.ends_with("\n\n") {
                        result.push_str("\n\n");
                    }

                    let child_text = extract_text_filtered(child_elem);
                    result.push_str(&child_text);

                    if is_block && !result.ends_with("\n\n") {
                        result.push_str("\n\n");
                    }
                }
            }
            _ => {}
        }
    }

    result
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
