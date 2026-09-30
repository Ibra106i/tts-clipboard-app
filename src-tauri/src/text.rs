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
pub(crate) fn byte_offset_of_char(text: &str, n: usize) -> usize {
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
    // Only a debug build reports this. It exists because the cost of splitting
    // a chapter is a real part of the cost of starting one, and "the reader felt
    // slow" is not something to argue with either way.
    #[cfg(debug_assertions)]
    let measured = {
        let started = std::time::Instant::now();
        let chunks = split_into_chunks_inner(text, max_chars);
        log::debug!(
            "chunked {} characters into {} chunks in {:?}",
            text.chars().count(),
            chunks.len(),
            started.elapsed()
        );
        chunks
    };
    #[cfg(debug_assertions)]
    return measured;
    #[cfg(not(debug_assertions))]
    split_into_chunks_inner(text, max_chars)
}

fn split_into_chunks_inner(text: &str, max_chars: usize) -> Vec<Chunk> {
    // Counting up front also covers the empty case, so there is no separate
    // guard for it: an empty string is zero characters, which never exceeds
    // `max_chars`, and `push_chunk` drops the resulting empty chunk.
    let total_chars = char_count(text);
    if max_chars == 0 || total_chars <= max_chars {
        let mut single = Vec::new();
        push_chunk(&mut single, text, 0);
        return single;
    }

    let mut chunks = Vec::new();
    // Character offset of `remaining` within the original `text`, and how many
    // characters are left in it. Both are tracked incrementally.
    //
    // The previous version counted the entire remaining suffix again on every
    // iteration while looking for the next boundary, which made chunking
    // quadratic in the chapter length: N^2/2C character decodings, so a
    // 500,000-character chapter decoded 62.5 million characters - on the
    // caller's thread, before the first word was spoken. Boundary search is
    // now bounded by `max_chars` and the whitespace run it trims, so the whole
    // pass is linear in the text.
    let mut consumed_chars = 0usize;
    let mut remaining_chars = total_chars;
    let mut remaining = text;

    loop {
        if remaining_chars <= max_chars {
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
        let boundary = &remaining[cut_at..];
        let rest = boundary.trim_start();
        // Only the whitespace `trim_start` removed is counted, and that is a
        // short run at the front of `boundary`. Counting `boundary` and `rest`
        // in full - as this did - would put the whole remaining chapter back
        // into every iteration.
        //
        // `trim_start` drops whole characters, so this offset is always a
        // character boundary.
        let dropped_whitespace = char_count(&boundary[..boundary.len() - rest.len()]);

        push_chunk(&mut chunks, taken, consumed_chars);
        let advanced = char_count(taken) + dropped_whitespace;
        consumed_chars += advanced;
        remaining_chars -= advanced;
        remaining = rest;
    }

    chunks
}

/// Character offset of the start of the first sentence that begins at or after
/// `char_offset`.
///
/// Clicking into a chapter should begin reading at a sentence, not part-way
/// through one: the speech engine has no way to start mid-utterance, so a chunk
/// that opens on a clause is spoken with broken prosody. The set of terminators
/// is the same [`SENTENCE_END`] the chunker already prefers, so a snapped
/// offset can never land inside a chunk boundary the chunker chose.
///
/// A sentence opens at the start of the text, or immediately after a
/// terminator and any run of whitespace that follows it. A click already sitting
/// on one of those positions is left alone; a click anywhere else moves
/// *forward* to the next sentence, so no text is ever skipped twice. When no
/// terminator follows — the click is in the last sentence — the offset is kept
/// exactly as given. Offsets past the end of the text are clamped, so the
/// result is always a valid index into `text`.
///
/// The returned offset always points at a non-whitespace character, matching the
/// convention [`split_into_chunks`] uses for `Chunk::start_char`.
pub fn snap_to_sentence_start(text: &str, char_offset: usize) -> usize {
    let target = char_offset.min(char_count(text));
    if target == 0 {
        return 0;
    }

    if opens_a_sentence(text, target) {
        return skip_whitespace(text, target);
    }

    // The click is inside a sentence, so move to the one that follows it. The
    // search starts at the click itself, which is past the terminator of the
    // sentence being read.
    let from = byte_offset_of_char(text, target);
    match text[from..].find(SENTENCE_END) {
        Some(byte_offset) => {
            let terminator_end = from
                + byte_offset
                + text[from + byte_offset..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
            skip_whitespace(text, char_count(&text[..terminator_end]))
        }
        None => target,
    }
}

/// The character at `char_offset - 1`, or `None` at the start of the text.
fn char_before(text: &str, char_offset: usize) -> Option<char> {
    if char_offset == 0 {
        return None;
    }
    text.char_indices().nth(char_offset - 1).map(|(_, ch)| ch)
}

/// Whether a sentence opens at `char_offset`: either the start of the text, or
/// a position reached by walking back over whitespace to a terminator.
fn opens_a_sentence(text: &str, char_offset: usize) -> bool {
    let mut probe = char_offset;
    while probe > 0 {
        match char_before(text, probe) {
            Some(ch) if ch.is_whitespace() => probe -= 1,
            Some(ch) => return SENTENCE_END.contains(&ch),
            None => return false,
        }
    }
    true
}

/// First non-whitespace character at or after `from`, or the end of the text.
fn skip_whitespace(text: &str, from: usize) -> usize {
    let mut at = from;
    while let Some(ch) = text.chars().nth(at) {
        if !ch.is_whitespace() {
            break;
        }
        at += 1;
    }
    at
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
    fn chunking_a_very_long_chapter_is_linear_not_quadratic() {
        // The cost this guards is character decoding, so the input is scaled
        // well past any real chapter. Chunking must keep up with the input
        // rather than falling away behind it: doubling the length should not
        // quadruple the time. The budget is deliberately loose - it is there to
        // catch a return to the quadratic walk, not to measure this machine.
        let short = "The quick brown fox jumps over the lazy dog. ".repeat(2_000);
        let long = "The quick brown fox jumps over the lazy dog. ".repeat(8_000);

        let start = std::time::Instant::now();
        let short_chunks = split_into_chunks(&short, CHUNK_CHARS);
        let short_elapsed = start.elapsed();
        assert!(!short_chunks.is_empty(), "the control input should chunk");

        let start = std::time::Instant::now();
        let long_chunks = split_into_chunks(&long, CHUNK_CHARS);
        let long_elapsed = start.elapsed();

        assert_invariants(&long, CHUNK_CHARS, &long_chunks);
        // 4x the input must not cost more than ~16x the time, with slack for a
        // loaded machine. The old implementation re-counted the remaining
        // suffix per chunk and blew straight through this.
        assert!(
            long_elapsed < short_elapsed * 24,
            "4x input took {:?} against {:?} for 1x - chunking is no longer linear",
            long_elapsed,
            short_elapsed
        );
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

    #[test]
    fn a_click_mid_sentence_snaps_forward_to_the_next_sentence() {
        let text = "First one. Second one. Third one.";
        // Character 14 is inside "Second one." (which spans 11..=21), so the
        // click must skip the rest of that sentence and land on "Third one.".
        let snapped = snap_to_sentence_start(text, 14);
        assert_eq!(text.chars().skip(snapped).collect::<String>(), "Third one.");
    }

    #[test]
    fn a_click_already_on_a_sentence_start_snaps_to_that_same_sentence() {
        let text = "First one. Second one.";
        // Character 0 opens the first sentence, so the offset must not move.
        assert_eq!(snap_to_sentence_start(text, 0), 0);
        // Character 11 opens the second sentence. The terminator at 10 must not
        // push it past the start of the sentence it already points at.
        assert_eq!(snap_to_sentence_start(text, 11), 11);
    }

    #[test]
    fn a_click_inside_the_final_sentence_stays_where_it_is() {
        let text = "First one. The last sentence has no terminator";
        let snapped = snap_to_sentence_start(text, 20);
        assert_eq!(snapped, 20);
    }

    #[test]
    fn snapping_past_the_end_of_the_text_clamps_to_the_text_length() {
        let text = "Only one sentence.";
        assert_eq!(snap_to_sentence_start(text, 5_000), char_count(text));
    }

    #[test]
    fn snapping_never_splits_a_multi_byte_character() {
        let text = "Emoji lead 👍 here. After the emoji.";
        // Character 4 is "e" in "lead"; the byte offset of the emoji's leading
        // surrogate must never be produced, so a naive byte-slice implementation
        // would panic here.
        for offset in 0..=char_count(text) {
            let snapped = snap_to_sentence_start(text, offset);
            assert!(
                text.is_char_boundary(byte_offset_of_char(text, snapped)),
                "snap for offset {offset} produced a non-boundary at {snapped}"
            );
        }
        assert_eq!(
            text.chars()
                .skip(snap_to_sentence_start(text, 5))
                .collect::<String>(),
            "After the emoji."
        );
    }

    #[test]
    fn snapping_an_empty_text_stays_at_zero() {
        assert_eq!(snap_to_sentence_start("", 0), 0);
        assert_eq!(snap_to_sentence_start("", 12), 0);
    }

    #[test]
    fn a_newline_counts_as_a_sentence_boundary() {
        // `\n` is in `SENTENCE_END`, so a click inside one block snaps to the
        // start of the next one rather than staying mid-block.
        let text = "First block here.\nSecond block here.\nThird block here.";
        let snapped = snap_to_sentence_start(text, 5);
        assert_eq!(snapped, 18);
        assert_eq!(
            text.chars().skip(snapped).collect::<String>(),
            "Second block here.\nThird block here."
        );
    }
}
