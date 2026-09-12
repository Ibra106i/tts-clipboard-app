use crate::models::Chapter;
use lopdf::Object;
use lopdf::Document;
use scraper::{Html, Selector};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const FONT_SIZE_RATIO_THRESHOLD: f32 = 0.75;
const BOTTOM_POSITION_THRESHOLD: f32 = 0.2;

struct TextRun {
    text: String,
    font_size: f32,
    y_position: f32,
}

struct PageState {
    font_size: f32,
    x: f32,
    y: f32,
    page_height: f32,
}

fn extract_text_runs(doc: &Document, page_id: lopdf::ObjectId) -> Option<Vec<TextRun>> {
    let content = doc.get_and_decode_page_content(page_id).ok()?;

    let page_height = get_page_height(doc, page_id).unwrap_or(842.0);

    let mut state = PageState {
        font_size: 12.0,
        x: 0.0,
        y: page_height,
        page_height,
    };

    let mut runs = Vec::new();
    let mut in_text = false;

    for operation in &content.operations {
        match operation.operator.as_str() {
            "BT" => {
                in_text = true;
            }
            "ET" => {
                in_text = false;
            }
            "Tf" => {
                if in_text && operation.operands.len() >= 2 {
                    state.font_size = operand_as_f32(&operation.operands[1]).unwrap_or(state.font_size);
                }
            }
            "Td" | "TD" => {
                if in_text && operation.operands.len() >= 2 {
                    let tx = operand_as_f32(&operation.operands[0]).unwrap_or(0.0);
                    let ty = operand_as_f32(&operation.operands[1]).unwrap_or(0.0);
                    state.x += tx;
                    state.y += ty;
                }
            }
            "Tm" => {
                if in_text && operation.operands.len() >= 6 {
                    state.x = operand_as_f32(&operation.operands[4]).unwrap_or(state.x);
                    state.y = operand_as_f32(&operation.operands[5]).unwrap_or(state.y);
                }
            }
            "T*" => {
                if in_text {
                    state.y -= state.font_size * 1.2;
                }
            }
            "Tj" => {
                if in_text {
                    if let Some(text) = extract_string_operand(&operation.operands) {
                        let normalized_y = (state.y / state.page_height).clamp(0.0, 1.0);
                        runs.push(TextRun {
                            text,
                            font_size: state.font_size,
                            y_position: normalized_y,
                        });
                        state.x += state.font_size * runs.last().map(|r| r.text.len() as f32 * 0.5).unwrap_or(0.0);
                    }
                }
            }
            "TJ" => {
                if in_text {
                    if let Some(text) = extract_tj_text(&operation.operands) {
                        let normalized_y = (state.y / state.page_height).clamp(0.0, 1.0);
                        runs.push(TextRun {
                            text,
                            font_size: state.font_size,
                            y_position: normalized_y,
                        });
                        state.x += state.font_size * runs.last().map(|r| r.text.len() as f32 * 0.5).unwrap_or(0.0);
                    }
                }
            }
            "'" => {
                if in_text {
                    state.y -= state.font_size * 1.2;
                    if let Some(text) = extract_string_operand(&operation.operands) {
                        let normalized_y = (state.y / state.page_height).clamp(0.0, 1.0);
                        runs.push(TextRun {
                            text,
                            font_size: state.font_size,
                            y_position: normalized_y,
                        });
                        state.x += state.font_size * runs.last().map(|r| r.text.len() as f32 * 0.5).unwrap_or(0.0);
                    }
                }
            }
            "\"" => {
                if in_text && operation.operands.len() >= 3 {
                    state.y -= state.font_size * 1.2;
                    if let Some(text) = extract_string_operand_index(&operation.operands, 2) {
                        let normalized_y = (state.y / state.page_height).clamp(0.0, 1.0);
                        runs.push(TextRun {
                            text,
                            font_size: state.font_size,
                            y_position: normalized_y,
                        });
                        state.x += state.font_size * runs.last().map(|r| r.text.len() as f32 * 0.5).unwrap_or(0.0);
                    }
                }
            }
            _ => {}
        }
    }

    Some(runs)
}

fn get_page_height(doc: &Document, page_id: lopdf::ObjectId) -> Option<f32> {
    let page = doc.get_dictionary(page_id).ok()?;
    let mediabox = page.get(b"MediaBox").ok()?;
    if let Object::Array(arr) = mediabox {
        if arr.len() >= 4 {
            let y0 = operand_as_f32(&arr[1]).unwrap_or(0.0);
            let y1 = operand_as_f32(&arr[3]).unwrap_or(842.0);
            return Some(y1 - y0);
        }
    }
    Some(842.0)
}

fn operand_as_f32(operand: &Object) -> Option<f32> {
    match operand {
        Object::Integer(n) => Some(*n as f32),
        Object::Real(n) => Some(*n),
        _ => None,
    }
}

fn extract_string_operand(operands: &[Object]) -> Option<String> {
    match &operands[0] {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).to_string()),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        _ => None,
    }
}

fn extract_string_operand_index(operands: &[Object], index: usize) -> Option<String> {
    operands.get(index).and_then(|op| match op {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).to_string()),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        _ => None,
    })
}

fn extract_tj_text(operands: &[Object]) -> Option<String> {
    if let Object::Array(arr) = &operands[0] {
        let mut text = String::new();
        for item in arr {
            match item {
                Object::String(bytes, _) => {
                    text.push_str(&String::from_utf8_lossy(bytes));
                }
                Object::Integer(_n) => {
                    // Negative integers are kerning adjustments — skip
                }
                _ => {}
            }
        }
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

fn compute_dominant_font_size(runs: &[TextRun]) -> f32 {
    let mut size_chars: HashMap<i32, usize> = HashMap::new();
    for run in runs {
        let key = (run.font_size * 10.0) as i32;
        *size_chars.entry(key).or_insert(0) += run.text.len();
    }
    size_chars
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(key, _)| key as f32 / 10.0)
        .unwrap_or(12.0)
}

fn is_footnote_candidate(run: &TextRun, dominant_size: f32) -> bool {
    let size_ratio = run.font_size / dominant_size;
    size_ratio < FONT_SIZE_RATIO_THRESHOLD && run.y_position < BOTTOM_POSITION_THRESHOLD
}

fn write_audit_log(
    file_path: &str,
    page_num: u32,
    excluded: &[&TextRun],
) {
    if excluded.is_empty() {
        return;
    }

    let log_path = audit_log_path(file_path);
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut content = String::new();
    for run in excluded {
        content.push_str(&format!(
            "page {} | size {:.1} | y {:.2} | {}\n",
            page_num, run.font_size, run.y_position, run.text
        ));
    }

    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(content.as_bytes())
        });
}

fn audit_log_path(file_path: &str) -> PathBuf {
    let stem = PathBuf::from(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    PathBuf::from(format!("{file_path}.{stem}_footnotes.txt"))
}

pub fn extract_pdf_text(file_path: &str) -> Result<Vec<Chapter>, String> {
    let doc = Document::load(file_path).map_err(|e| format!("Failed to load PDF: {e}"))?;

    let pages = doc.get_pages();
    let mut chapters = Vec::new();

    for (page_num, page_id) in &pages {
        let page_num_u32 = *page_num;

        let page_text = match extract_text_runs(&doc, *page_id) {
            Some(runs) if !runs.is_empty() => {
                let dominant_size = compute_dominant_font_size(&runs);

                let has_variation = runs.iter().any(|r| {
                    let ratio = r.font_size / dominant_size;
                    (ratio - 1.0).abs() > 0.1
                });

                if !has_variation {
                    runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ")
                } else {
                    let mut included = Vec::new();
                    let mut excluded = Vec::new();
                    for run in &runs {
                        if is_footnote_candidate(run, dominant_size) {
                            excluded.push(run);
                        } else {
                            included.push(run);
                        }
                    }
                    write_audit_log(file_path, page_num_u32, &excluded);
                    included.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ")
                }
            }
            _ => {
                doc.extract_text(&[page_num_u32])
                    .unwrap_or_default()
            }
        };

        let trimmed = page_text.trim().to_string();
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
