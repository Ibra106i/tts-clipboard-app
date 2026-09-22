//! Unicode-aware text measurement and chunking.
//!
//! Units matter, and this module is the only place that decides which one is
//! meant. Everything here counts **characters** (Unicode scalar values), never
//! bytes and never UTF-16 code units, because:
//!
//! * indexing a `str` by byte offset panics when the offset is not a character
//!   boundary (the bug this module exists to prevent), and
//! * SAPI reports progress in characters of the queued input.

use std::fmt;

/// Maximum number of characters handed to the speech engine in one call.
///
/// Chunking keeps individual `Speak` calls short so pause/stop stay responsive
/// and so progress can be reported without holding a single enormous request.
pub const CHUNK_CHARS: usize = 2000;

/// Sentence terminators a chunk is allowed to end on, in preference order.
const SENTENCE_END: [char; 5] = ['.', '!', '?', '\n', '\u{2026}'];

/// Number of characters (not bytes) in `text`.
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// A slice of text, measured in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Character offset of this chunk within the original text.
    pub start_char: usize,
    /// Number of characters in this chunk.
    pub char_count: usize,
    pub text: String,
}

impl fmt::Display for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}..+{}]", self.start_char, self.char_count)
    }
}

/// Byte offset of the `n`-th character, or the length of `text` when it is
/// shorter. The returned offset is always a valid character boundary, which is
/// what makes slicing safe.
fn byte_offset_of_char(text: &str, n: usize) -> usize {
    text.char_indices()
        .nth(n)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

/// Split `text` into chunks of at most `max_chars` characters.
///
/// Boundaries are chosen in this order: the last sentence terminator inside the
/// window, then the last whitespace, then a hard cut on a character boundary.
/// Whitespace at a boundary is dropped, so a chunk never starts with the
/// separator that ended the previous one. No character is ever split or lost.
///
/// `max_chars == 0` is treated as "do not split" rather than as an error.
pub fn split_into_chunks(text: &str, max_chars: usize) -> Vec<Chunk> {
    if text.is_empty() {
        return Vec::new();
    }
    if max_chars == 0 || char_count(text) <= max_chars {
        return vec![Chunk {
            start_char: 0,
            char_count: char_count(text),
            text: text.to_string(),
        }];
    }

    let mut chunks = Vec::new();
    // Character offset of `remaining` within the original `text`. Tracked
    // incrementally so the cost stays proportional to the text length instead of
    // re-counting from the start on every chunk.
    let mut consumed_chars = 0usize;
    let mut remaining = text;

    loop {
        if char_count(remaining) <= max_chars {
            push_chunk(&mut chunks, remaining, consumed_chars);
            break;
        }

        // Safe by construction: `byte_offset_of_char` only returns character
        // boundaries, so this slice can never split a character.
        let window_end = byte_offset_of_char(remaining, max_chars);
        let window = &remaining[..window_end];

        let after_sentence = window.rfind(SENTENCE_END).map(|offset| {
            let terminator = window[offset..].chars().next().unwrap_or('.');
            offset + terminator.len_utf8()
        });
        let cut_at = after_sentence
            .or_else(|| window.rfind(char::is_whitespace))
            .unwrap_or(window_end);

        let taken = &remaining[..cut_at];
        let rest = remaining[cut_at..].trim_start();
        let dropped_whitespace = char_count(&remaining[cut_at..]) - char_count(rest);

        push_chunk(&mut chunks, taken, consumed_chars);
        consumed_chars += char_count(taken) + dropped_whitespace;
        remaining = rest;
    }

    chunks
}

fn push_chunk(chunks: &mut Vec<Chunk>, text: &str, start_char: usize) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    chunks.push(Chunk {
        start_char,
        char_count: char_count(trimmed),
        text: trimmed.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_count_counts_characters_not_bytes() {
        assert_eq!(char_count("abc"), 3);
        assert_eq!("héllo".len(), 6);
        assert_eq!(char_count("héllo"), 5);
        assert_eq!(char_count("日本語"), 3);
        assert_eq!(char_count("👍"), 1);
    }

    #[test]
    fn empty_text_produces_no_chunks() {
        assert!(split_into_chunks("", CHUNK_CHARS).is_empty());
    }

    #[test]
    fn short_text_is_one_chunk() {
        let chunks = split_into_chunks("Hello world.", CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello world.");
        assert_eq!(chunks[0].start_char, 0);
        assert_eq!(chunks[0].char_count, 12);
    }

    /// The original implementation sliced by byte offset and panicked with
    /// "byte index 2000 is not a char boundary" on text like this.
    #[test]
    fn multi_byte_text_at_the_old_byte_boundary_no_longer_panics() {
        let text = "日本語のテキストです。".repeat(400); // > 2000 bytes, ~4000 chars
        assert!(text.len() > 2000, "fixture must exceed the old byte limit");

        let chunks = split_into_chunks(&text, CHUNK_CHARS);

        assert!(chunks.len() > 1);
        let rejoined: String = chunks.iter().map(|c| c.text.clone()).collect();
        assert_eq!(char_count(&rejoined), char_count(&text));
    }

    #[test]
    fn emoji_are_never_split_across_chunks() {
        let text = "👍".repeat(CHUNK_CHARS + 500);
        let chunks = split_into_chunks(&text, CHUNK_CHARS);

        let total: usize = chunks.iter().map(|c| c.char_count).sum();
        assert_eq!(total, CHUNK_CHARS + 500);
        for chunk in &chunks {
            assert_eq!(char_count(&chunk.text), chunk.char_count);
        }
    }

    #[test]
    fn text_just_over_the_limit_is_split_into_two_chunks() {
        let text = format!("{}extra", "a ".repeat(CHUNK_CHARS));
        let chunks = split_into_chunks(&text, CHUNK_CHARS);
        assert!(chunks.len() >= 2, "expected a split, got {}", chunks.len());
    }

    #[test]
    fn text_exactly_at_the_limit_is_not_split() {
        let text = "a".repeat(CHUNK_CHARS);
        assert_eq!(split_into_chunks(&text, CHUNK_CHARS).len(), 1);
    }

    #[test]
    fn a_zero_limit_means_do_not_split() {
        let chunks = split_into_chunks("some text", 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "some text");
    }
}
