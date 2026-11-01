//! Splits document text into overlapping windows for embedding

/// Default window size in characters (roughly 400 tokens of prose)
pub const DEFAULT_CHUNK_CHARS: usize = 1500;

/// Default overlap between consecutive windows in characters
pub const DEFAULT_OVERLAP_CHARS: usize = 200;

/// Split `text` into windows of at most `max_chars` characters.
///
/// Windows end at whitespace when one is available in the last quarter of the
/// window, and each window starts `overlap` characters before the previous one
/// ended so a sentence cut at a boundary still appears whole in one chunk.
pub fn chunk_text(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let overlap = if overlap >= max_chars {
        max_chars / 4
    } else {
        overlap
    };

    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let mut end = (start + max_chars).min(chars.len());
        if end < chars.len() {
            let floor = start + max_chars - max_chars / 4;
            if let Some(pos) = (floor..end).rev().find(|&i| chars[i].is_whitespace()) {
                end = pos + 1;
            }
        }

        let chunk: String = chars[start..end].iter().collect();
        let chunk = chunk.trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }

        if end >= chars.len() {
            break;
        }
        start = end.saturating_sub(overlap).max(start + 1);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_yield_no_chunks() {
        assert!(chunk_text("", 100, 10).is_empty());
        assert!(chunk_text("  \n\t ", 100, 10).is_empty());
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk_text("hello world", 100, 10), vec!["hello world"]);
    }

    #[test]
    fn long_text_is_windowed_with_overlap() {
        let text: String = (0..400).map(|i| format!("word{i} ")).collect();
        let chunks = chunk_text(&text, 200, 40);

        assert!(chunks.len() > 5);
        assert!(chunks.iter().all(|c| c.chars().count() <= 200));

        for pair in chunks.windows(2) {
            let tail = pair[0].split_whitespace().last().unwrap();
            assert!(
                pair[1].contains(tail),
                "chunk should repeat the previous chunk's last word {tail}"
            );
        }

        for i in 0..400 {
            let word = format!("word{i}");
            assert!(chunks
                .iter()
                .any(|c| c.split_whitespace().any(|w| w == word)));
        }
    }

    #[test]
    fn unbroken_text_still_terminates() {
        let text = "x".repeat(1000);
        let chunks = chunk_text(&text, 100, 100);
        assert!(chunks.len() >= 10);
        assert!(chunks.iter().all(|c| c.len() <= 100));
    }

    #[test]
    fn multibyte_text_is_not_split_inside_a_character() {
        let text = "äöü ".repeat(200);
        let chunks = chunk_text(&text, 50, 10);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.chars().count() <= 50));
    }
}
