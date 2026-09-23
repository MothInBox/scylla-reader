//! Snippet generation for AI search chapter hits.
//!
//! A snippet is a ~120-char window of the matching chunk's text, centered on
//! the best-matching span (the BM25-matched term for keyword hits, or the start
//! of the text for semantic hits). Truncation snaps to word boundaries and
//! marks the cut with "…".

/// Maximum snippet length in characters.
pub const SNIPPET_LEN: usize = 120;

/// Builds a ~`max_len`-char snippet from `text`, centered on the first
/// case-insensitive occurrence of `center` when given, otherwise on the start
/// of the text. Truncates at word boundaries and adds "…" on truncated sides.
/// Short texts are returned unchanged.
pub fn snippet(text: &str, center: Option<&str>, max_len: usize) -> String {
    let total = text.chars().count();
    if total <= max_len {
        return text.to_string();
    }
    // Char offset of the center (first case-insensitive match), else start.
    // Counted in the LOWERCASED string: lowercasing can expand byte length
    // (e.g. "İ" → "i̇"), so byte offsets into `lower` must never slice `text`.
    let center_char = center
        .filter(|c| !c.is_empty())
        .and_then(|c| {
            let lower = text.to_lowercase();
            lower
                .find(&c.to_lowercase())
                .map(|byte| lower[..byte].chars().count())
        })
        .unwrap_or(0);
    let half = max_len / 2;
    let mut start = center_char.saturating_sub(half);
    let mut end = (start + max_len).min(total);
    start = end.saturating_sub(max_len);

    let chars: Vec<char> = text.chars().collect();
    // Snap to word boundaries: don't start or end mid-word. Each snap is
    // guarded so it can't collapse the window to empty.
    if start > 0 && !chars[start].is_whitespace() {
        let mut s = start;
        while s < end && !chars[s].is_whitespace() {
            s += 1;
        }
        if s < end {
            start = s;
        }
    }
    if end < total && !chars[end - 1].is_whitespace() {
        let mut e = end;
        while e > start && !chars[e - 1].is_whitespace() {
            e -= 1;
        }
        if e > start {
            end = e;
        }
    }

    let mut out = String::new();
    if start > 0 {
        out.push('…');
        out.push(' ');
    }
    out.extend(&chars[start..end]);
    if end < total {
        out.push(' ');
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ~200 chars, "dragon" sits in the middle.
    const LONG: &str = "The ancient dragon slept beneath the mountain for a thousand years while the kingdom prospered above and the knights told tales of its fiery breath and gleaming scales and the treasure it guarded in the deep caverns below the keep";

    #[test]
    fn test_short_text_returned_unchanged() {
        assert_eq!(snippet("short text", None, 120), "short text");
        assert_eq!(snippet("", None, 120), "");
    }

    #[test]
    fn test_centers_on_matched_term() {
        // "treasure" sits deep in the text, so a 40-char window is truncated on
        // both sides.
        let s = snippet(LONG, Some("treasure"), 40);
        assert!(s.contains("treasure"), "snippet: {s}");
        // Truncated on both sides → both ellipses present.
        assert!(s.starts_with('…'));
        assert!(s.ends_with('…'));
        // Word-boundary clean: the trimmed body starts and ends with a whole
        // word (no partial word at the cut edges).
        let body = s.trim_matches('…').trim();
        let words: Vec<&str> = body.split_whitespace().collect();
        assert!(words.len() >= 2, "snippet body: {body}");
        assert!(body.contains("treasure"));
    }

    #[test]
    fn test_semantic_fallback_centers_on_start() {
        let s = snippet(LONG, None, 40);
        // No center → window starts at the beginning (no leading ellipsis).
        assert!(!s.starts_with('…'));
        assert!(s.starts_with("The ancient"));
        // Truncated at the end → trailing ellipsis.
        assert!(s.ends_with('…'));
    }

    #[test]
    fn test_center_not_found_falls_back_to_start() {
        let s = snippet(LONG, Some("zzz"), 30);
        assert!(!s.starts_with('…'));
        assert!(s.starts_with("The ancient"));
    }

    #[test]
    fn test_truncation_respects_max_len() {
        let text = "word ".repeat(100);
        let s = snippet(&text, None, 50);
        // 50 chars + up to 2 ellipses + 2 spaces.
        assert!(
            s.chars().count() <= 50 + 4,
            "snippet too long: {}",
            s.chars().count()
        );
    }

    #[test]
    fn test_no_panic_on_lowercase_expanding_chars() {
        // "İ" (U+0130) lowercases to "i̇" (3 bytes) — byte offsets into the
        // lowercased string exceed the original's byte length. Slicing `text`
        // with those offsets used to panic; the center must be counted in
        // lowercase space instead.
        let text = format!("{} dragon", "İ".repeat(100));
        let s = snippet(&text, Some("dragon"), 40);
        assert!(s.contains("dragon"), "snippet: {s}");
    }
}
