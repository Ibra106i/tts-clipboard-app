use crate::error::{AppError, AppResult};
use crate::models::Chapter;
use lopdf::Document;
use scraper::{Html, Selector};

/// Short, user-facing name for a path: the file name when there is one.
fn file_label(file_path: &str) -> String {
    std::path::Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(file_path)
        .to_string()
}

pub fn extract_pdf_text(file_path: &str) -> AppResult<Vec<Chapter>> {
    let label = file_label(file_path);
    let doc = Document::load(file_path)
        .map_err(|e| AppError::document(&label, format!("the PDF could not be opened ({e})")))?;

    let pages = doc.get_pages();
    let mut chapters = Vec::new();

    for page_num in pages.keys() {
        let page_num_u32 = *page_num;
        let text = doc.extract_text(&[page_num_u32]).map_err(|e| {
            AppError::document(&label, format!("page {page_num} could not be read ({e})"))
        })?;

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
        return Err(AppError::document(
            &label,
            "no text layer was found, so this looks like a scanned PDF",
        ));
    }

    Ok(chapters)
}

pub fn extract_epub_text(file_path: &str) -> AppResult<Vec<Chapter>> {
    let label = file_label(file_path);
    let mut archive = epub::doc::EpubDoc::new(file_path)
        .map_err(|e| AppError::document(&label, format!("the EPUB could not be opened ({e})")))?;

    let spine = archive.spine.clone();
    let mut chapters = Vec::new();

    for (i, spine_item) in spine.iter().enumerate() {
        let Some(res_id) = spine_item.id.clone() else {
            continue;
        };
        let Some((data, _mime)) = archive.get_resource(&res_id) else {
            continue;
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
        return Err(AppError::document(
            &label,
            "no readable content was found in this EPUB",
        ));
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

    let epub_type_selector = Selector::parse("[epub\\:type]").expect("static selector");
    let has_epub_type_markers = document.select(&epub_type_selector).any(|elem| {
        if let Some(epub_type) = elem.value().attr("epub:type") {
            let lower = epub_type.to_lowercase();
            lower.contains("footnote") || lower.contains("endnote")
        } else {
            false
        }
    });

    let aside_selector = Selector::parse("aside").expect("static selector");
    let has_aside = document.select(&aside_selector).next().is_some();

    let mut has_class_markers = false;
    let class_selector = Selector::parse("[class]").expect("static selector");
    for elem in document.select(&class_selector) {
        if is_excluded_element(elem) {
            has_class_markers = true;
            break;
        }
    }

    if !has_epub_type_markers && !has_aside && !has_class_markers {
        return strip_html(html);
    }

    let body_selector = Selector::parse("body").expect("static selector");
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
    let block_tags = [
        "p",
        "div",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "li",
        "blockquote",
        "pre",
        "br",
    ];

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

/// Tags that start a new paragraph when stripped; anything else is inline
/// and must not break the surrounding sentence.
fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "tr"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "br"
            | "hr"
            | "table"
            | "ul"
            | "ol"
    )
}

/// Decode the HTML entities a hand-rolled scanner can meet, including the
/// numeric forms. Without this, TTS literally reads "amp semicolon" aloud.
fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let semi = rest.find(';').filter(|&i| i > 1 && i <= 12);
        match semi {
            Some(i) => {
                let entity = &rest[1..i];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{00a0}'),
                    "mdash" => Some('—'),
                    "ndash" => Some('–'),
                    "hellip" => Some('…'),
                    "lsquo" | "rsquo" => Some('’'),
                    "ldquo" | "rdquo" => Some('"'),
                    other => {
                        // &#123; or &#x1F600;
                        let code = if let Some(hex) = other
                            .strip_prefix("#x")
                            .or_else(|| other.strip_prefix("#X"))
                        {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            other.strip_prefix('#').and_then(|d| d.parse().ok())
                        };
                        code.and_then(char::from_u32)
                    }
                };
                match decoded {
                    Some(ch) => {
                        out.push(ch);
                        rest = &rest[i + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Fallback stripper for documents without footnote markers.
///
/// The previous version pushed a newline for *every* tag, so
/// `some <em>italic</em> text` came out as three lines; it also left HTML
/// entities undecoded for the synthesiser to read aloud character by
/// character. Both are fixed here.
fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut skip_content = false;

    let mut chars = html.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag_name = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' || next.is_whitespace() {
                    break;
                }
                tag_name.push(next);
                chars.next();
            }
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == '>' {
                    break;
                }
            }

            let lower = tag_name.to_lowercase();
            if lower == "script" || lower == "style" {
                skip_content = true;
            } else if lower == "/script" || lower == "/style" {
                skip_content = false;
            } else if !skip_content {
                // Opening *and* closing block tags break, so paragraphs end
                // up separated by a blank line; inline tags add nothing.
                let name = lower.strip_prefix('/').unwrap_or(&lower);
                if is_block_tag(name) {
                    result.push('\n');
                }
            }
            continue;
        }

        if skip_content {
            continue;
        }

        result.push(ch);
    }

    // Collapse runs of blank lines to a single paragraph break.
    let mut cleaned = String::with_capacity(result.len());
    let mut pending_newlines = 0usize;
    for ch in decode_entities(&result).chars() {
        if ch == '\n' {
            pending_newlines = (pending_newlines + 1).min(2);
        } else {
            if pending_newlines > 0 {
                for _ in 0..pending_newlines.min(2) {
                    cleaned.push('\n');
                }
                pending_newlines = 0;
            }
            cleaned.push(ch);
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_label_falls_back_to_the_full_path() {
        assert_eq!(file_label("C:/books/novel.pdf"), "novel.pdf");
        assert_eq!(file_label("novel.epub"), "novel.epub");
    }

    #[test]
    fn missing_pdf_reports_the_file_name_not_the_full_path() {
        let error = extract_pdf_text("C:/definitely/missing/novel.pdf").expect_err("missing file");
        assert_eq!(error.code(), "document_parse_failed");
        assert!(
            error.user_message().contains("novel.pdf"),
            "unexpected message: {}",
            error.user_message()
        );
    }

    #[test]
    fn missing_epub_reports_a_parse_failure() {
        let error =
            extract_epub_text("C:/definitely/missing/novel.epub").expect_err("missing file");
        assert_eq!(error.code(), "document_parse_failed");
    }

    #[test]
    fn entities_are_decoded_not_read_aloud() {
        let out = strip_html("<p>Tom&amp;Jerry&#8217;s &nbsp;book&mdash;yes</p>");
        assert!(
            out.contains("Tom&Jerry\u{2019}s \u{00a0}book\u{2014}yes"),
            "entities must decode, got {out:?}"
        );
        assert!(!out.contains("amp;"), "raw entity text remains: {out:?}");
        assert!(!out.contains("&#"), "numeric entity remains: {out:?}");
    }

    #[test]
    fn inline_tags_do_not_break_sentences() {
        let out = strip_html("<p>some <em>italic</em> words</p>");
        assert_eq!(
            out.trim().lines().count(),
            1,
            "inline tags must not split: {out:?}"
        );
        assert_eq!(out.trim(), "some italic words");
    }

    #[test]
    fn block_tags_still_start_new_paragraphs() {
        let out = strip_html("<p>first</p><p>second</p><h2>Head</h2>");
        assert!(out.contains("first\n\nsecond"), "got {out:?}");
        assert!(out.contains("second\n\nHead"), "got {out:?}");
    }

    #[test]
    fn script_and_style_bodies_are_dropped() {
        let out = strip_html("<style>p{color:red}</style><p>keep</p><script>alert(1)</script>");
        assert_eq!(out.trim(), "keep");
    }

    #[test]
    fn unknown_entities_pass_through_unchanged() {
        let out = strip_html("<p>a &weird; b</p>");
        assert!(out.contains("&weird;"), "got {out:?}");
    }
}
