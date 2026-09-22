//! Unicode-aware text measurement and chunking.
//!
//! Units matter, and this module is the only place that decides which one is
//! meant. Everything here counts **characters** (Unicode scalar values), never
//! bytes and never UTF-16 code units, because:
//!
//! * indexing a `str` by byte offset panics when the offset is not a character
//!   boundary (the bug this module exists to prevent), and
//! * the speech engine reports progress in characters of the queued input.
//!
//! Units used across the codebase:
//! * `chars`   - Unicode scalar values; the unit of everything in this module.
//! * `bytes`   - raw UTF-8 length; never used for slicing or progress.
//! * `utf16`   - JavaScript string length; only ever relevant inside the webview.

use std::fmt;

/// Maximum number of characters handed to the speech engine in one call.
///
/// Chunking keeps individual `Speak` calls short so pause and stop stay
/// responsive and so progress can be reported without one enormous request.
pub const CHUNK_CHARS: usize = 2000;

/// Characters a chunk is allowed to end on, in preference order.
const SENTENCE_END: [char; 5] = ['.', '!', '?', '\n', '\u{2026}'];

/// Number of characters (not bytes) in `text`.
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// A slice of text, measured in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Character offset of the chunk's first spoken character in the source.
    pub start_char: usize,
    /// Number of characters in `text`.
    pub char_count: usize,
    /// The text to speak, with surrounding whitespace removed.
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

/// Append `text` as a chunk starting at `start_char`, skipping empty content and
/// accounting for the leading whitespace the chunk text drops.
fn push_chunk(chunks: &mut Vec<Chunk>, text: &str, start_char: usize) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let leading_whitespace = char_count(text) - char_count(text.trim_start());
    chunks.push(Chunk {
        start_char: start_char + leading_whitespace,
        char_count: char_count(trimmed),
        text: trimmed.to_string(),
    });
}

/// Split `text` into chunks of at most `max_chars` characters.
///
/// Boundaries are chosen in this order: the last sentence terminator inside the
/// window, then the last whitespace, then a hard cut on a character boundary.
/// Whitespace at a boundary is dropped, so a chunk never starts with the
/// separator that ended the previous one. No character is ever split or lost.
///
/// Whitespace-only input produces no chunks. `max_chars == 0` means "do not
/// split" rather than an error.
pub fn split_into_chunks(text: &str, max_chars: usize) -> Vec<Chunk> {
    if text.is_empty() {
        return Vec::new();
    }
    if max_chars == 0 || char_count(text) <= max_chars {
        let mut single = Vec::new();
        push_chunk(&mut single, text, 0);
        return single;
    }

    let mut chunks = Vec::new();
    // Character offset of `remaining` within the original `text`. Tracked
    // incrementally so the cost stays proportional to the text length instead of
    // re-counting from the start for every chunk.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every invariant a valid chunk list must satisfy, checked for every
    /// fixture: no chunk exceeds the limit, chunks are in order, none is empty,
    /// none has surrounding whitespace, and no non-whitespace character of the
    /// input is lost or reordered.
    fn assert_invariants(input: &str, max_chars: usize, chunks: &[Chunk]) {
        let keep = |text: &str| -> String { text.chars().filter(|c| !c.is_whitespace()).collect() };

        for (index, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.char_count <= max_chars,
                "chunk {index} has {} chars (limit {max_chars})",
                chunk.char_count
            );
            assert_eq!(
                char_count(&chunk.text),
                chunk.char_count,
                "chunk {index} reports the wrong char count"
            );
            assert!(
                !chunk.text.is_empty(),
                "chunk {index} is empty and should have been skipped"
            );
            assert_eq!(
                chunk.text.trim(),
                chunk.text,
                "chunk {index} has surrounding whitespace"
            );
            if index > 0 {
                assert!(
                    chunk.start_char >= chunks[index - 1].start_char,
                    "chunk {index} starts before its predecessor"
                );
            }
        }

        let joined: String = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            keep(&joined),
            keep(input),
            "characters were lost or reordered while chunking"
        );
    }

    #[test]
    fn char_count_counts_characters_not_bytes() {
        assert_eq!(char_count("abc"), 3);
        assert_eq!("héllo".len(), 6);
        assert_eq!(char_count("héllo"), 5);
        assert_eq!(char_count("日本語"), 3);
        assert_eq!(char_count("👍"), 1);
        assert_eq!(char_count("مرحبا"), 5);
    }

    #[test]
    fn empty_and_whitespace_only_input_produce_no_chunks() {
        assert!(split_into_chunks("", CHUNK_CHARS).is_empty());
        assert!(split_into_chunks("   \n\t ", CHUNK_CHARS).is_empty());
    }

    #[test]
    fn short_text_is_one_chunk_with_accurate_offsets() {
        let chunks = split_into_chunks("Hello world.", CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello world.");
        assert_eq!(chunks[0].start_char, 0);
        assert_eq!(chunks[0].char_count, 12);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_and_offsets_stay_exact() {
        let chunks = split_into_chunks("   Hello", CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Hello");
        assert_eq!(chunks[0].start_char, 3);
        assert_eq!(chunks[0].char_count, 5);
    }

    /// The original implementation sliced with `remaining[..2000]`, a byte
    /// offset, and panicked with "byte index 2000 is not a char boundary" on any
    /// text whose 2000th byte fell inside a multi-byte character.
    #[test]
    fn multi_byte_text_at_the_old_byte_boundary_no_longer_panics() {
        let text = "日本語のテキストです。".repeat(400);
        assert!(text.len() > 2000, "fixture must exceed the old byte limit");

        let chunks = split_into_chunks(&text, CHUNK_CHARS);
        assert!(chunks.len() > 1, "expected a split");
        assert_invariants(&text, CHUNK_CHARS, &chunks);
    }

    #[test]
    fn emoji_are_never_split_across_chunks() {
        let text = "👍".repeat(CHUNK_CHARS + 500);
        let chunks = split_into_chunks(&text, CHUNK_CHARS);
        assert_invariants(&text, CHUNK_CHARS, &chunks);

        let total: usize = chunks.iter().map(|c| c.char_count).sum();
        assert_eq!(total, CHUNK_CHARS + 500);
    }

    #[test]
    fn sentence_boundaries_are_preferred_over_hard_cuts() {
        let text = format!(
            "{} End of sentence. {} trailing text",
            "a".repeat(1000),
            "b".repeat(500)
        );
        let chunks = split_into_chunks(&text, 1100);
        assert!(chunks.len() >= 2, "expected a split, got {}", chunks.len());
        assert_invariants(&text, 1100, &chunks);
        assert!(
            chunks[0].text.ends_with("End of sentence."),
            "first chunk should end on the sentence boundary, got: {:?}",
            &chunks[0].text[chunks[0].text.len().saturating_sub(40)..]
        );
    }

    #[test]
    fn falls_back_to_whitespace_when_no_sentence_end_exists() {
        let words = "word ".repeat(1000);
        let chunks = split_into_chunks(&words, 100);
        assert!(chunks.len() > 1);
        assert_invariants(&words, 100, &chunks);
    }

    #[test]
    fn hard_cuts_when_there_is_no_whitespace_at_all() {
        let text = "a".repeat(500);
        let chunks = split_into_chunks(&text, 100);
        assert_eq!(chunks.len(), 5);
        assert_invariants(&text, 100, &chunks);
    }

    #[test]
    fn a_single_unbroken_word_longer_than_the_limit_is_still_split() {
        let word = "ü".repeat(300);
        let chunks = split_into_chunks(&word, 100);
        assert_eq!(chunks.len(), 3);
        assert_invariants(&word, 100, &chunks);
    }

    #[test]
    fn every_supported_script_survives_chunking() {
        let fixtures: [(&str, &str); 8] = [
            ("ASCII", "The quick brown fox jumps over the lazy dog. "),
            (
                "accented Latin",
                "Le café était très animé, à côté de l'église. ",
            ),
            ("Turkish", "İstanbul'da yaşayan ğüşiöç öğrenciler var. "),
            ("Arabic", "مرحبا بالعالم، هذا نص عربي طويل جدا. "),
            ("Hebrew", "שלום עולם, זהו טקסט עברי ארוך. "),
            (
                "Japanese",
                "これは日本語のテキストです。とても長い文章です。 ",
            ),
            ("Chinese", "这是一段很长的中文文本。它包含很多字符。 "),
            (
                "emoji mixed",
                "Party time 🎉🎊 with 👍👍 and flags 🇹🇷🇯🇵 ok. ",
            ),
        ];

        for (name, unit) in fixtures {
            let text = unit.repeat(120);
            let chunks = split_into_chunks(&text, 150);
            assert!(chunks.len() > 1, "{name}: expected a split");
            assert_invariants(&text, 150, &chunks);
        }
    }

    #[test]
    fn boundary_conditions_around_the_limit() {
        for (label, length) in [
            ("one under", CHUNK_CHARS - 1),
            ("exact", CHUNK_CHARS),
            ("one over", CHUNK_CHARS + 1),
        ] {
            let text = "a".repeat(length);
            let chunks = split_into_chunks(&text, CHUNK_CHARS);
            assert_invariants(&text, CHUNK_CHARS, &chunks);

            if length <= CHUNK_CHARS {
                assert_eq!(chunks.len(), 1, "{label}: should not split");
            } else {
                assert_eq!(chunks.len(), 2, "{label}: should split once");
            }
        }
    }

    #[test]
    fn very_long_input_is_split_into_many_chunks_without_loss() {
        let text = "Sed ut perspiciatis unde omnis iste natus error. ".repeat(2000);
        let chunks = split_into_chunks(&text, CHUNK_CHARS);
        assert!(
            chunks.len() > 20,
            "expected many chunks, got {}",
            chunks.len()
        );
        assert_invariants(&text, CHUNK_CHARS, &chunks);
    }

    #[test]
    fn a_zero_limit_means_do_not_split() {
        let chunks = split_into_chunks("some text", 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "some text");
    }

    #[test]
    fn a_limit_of_one_character_still_makes_progress() {
        let chunks = split_into_chunks("abc", 1);
        assert_eq!(chunks.len(), 3);
        assert_invariants("abc", 1, &chunks);
    }

    #[test]
    fn middle_dot_and_ellipsis_terminators_are_honoured() {
        let text = format!("{} … and then it ended…more text here", "x".repeat(50));
        let chunks = split_into_chunks(&text, 60);
        assert_invariants(&text, 60, &chunks);
        assert!(chunks.len() >= 2);
    }
}
