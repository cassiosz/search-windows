//! Typos in the address field: a word with a letter wrong, missing or swapped
//! still finds what it meant. Only a fallback, ranked below exact matches.
//! Port of upstream Sources/Search/Fuzzy.swift (#762).
//!
//! Works on bytes because it runs over the whole history on every key.
//! Public so the app's other lists (Ctrl+K's tabs, the bookmarks) forgive
//! the same typos the same way.

/// Letters and digits, and anything outside ASCII (Fuzzy.swift `inWord`).
fn in_word(byte: u8) -> bool {
    byte >= 0x80 || byte.is_ascii_digit() || byte.is_ascii_lowercase()
}

/// Whether `word` is in `text` (both already lowercased).
pub fn contains(text: &str, word: &str) -> bool {
    text.contains(word)
}

/// Whether `word` starts a word of `text`: "api" in "team/api", not "rapid".
/// A word that doesn't start with a letter or digit counts anywhere.
pub fn starts_word(word: &str, text: &str) -> bool {
    let (word, text) = (word.as_bytes(), text.as_bytes());
    if word.is_empty() || text.len() < word.len() {
        return false;
    }
    let anywhere = !in_word(word[0]);
    let mut from = 0;
    while from + word.len() <= text.len() {
        let Some(at) = find(&text[from..], word) else { return false };
        let at = from + at;
        if anywhere || at == 0 || !in_word(text[at - 1]) {
            return true;
        }
        from = at + 1;
    }
    false
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let first = needle[0];
    let last_start = hay.len().checked_sub(needle.len())?;
    (0..=last_start).find(|&i| hay[i] == first && hay[i..i + needle.len()] == *needle)
}

/// 1 if every word is in `text` as typed, 0 if one needed a typo forgiven,
/// None if one isn't there. Both are lowercased.
pub fn matches(words: &[&str], text: &str) -> Option<u8> {
    let mut slipped = false;
    for word in words {
        if !contains(text, word) {
            if !nearly(word.as_bytes(), text.as_bytes()) {
                return None;
            }
            slipped = true;
        }
    }
    Some(if slipped { 0 } else { 1 })
}

/// Whether a word in `text` starts with `typed`, allowing one typo, or two in
/// long words. None under four letters, where a guess is noise.
pub fn nearly(typed: &[u8], text: &[u8]) -> bool {
    let allowed = if typed.len() < 4 { 0 } else if typed.len() < 8 { 1 } else { 2 };
    if allowed == 0 {
        return false;
    }
    // Edit distance to the start of each word, where swapping two letters
    // counts as one. Rows are reused across words.
    let width = typed.len() + allowed + 1;
    // Made at the first word worth measuring: most texts have none, and this
    // runs for every page in the history on every key.
    let mut before: Vec<usize> = Vec::new();
    let mut above: Vec<usize> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut start = 0;
    while start < text.len() {
        if !in_word(text[start]) {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < text.len() && in_word(text[end]) {
            end += 1;
        }
        let word = &text[start..end];
        start = end;
        // People rarely get the first letter wrong, so require it (or a swap
        // with the second). This skips most words cheaply.
        if word.len() < 2
            || word.len() + allowed < typed.len()
            || !(word[0] == typed[0] || (word[0] == typed[1] && word[1] == typed[0]))
        {
            continue;
        }
        if row.is_empty() {
            before = vec![0; width];
            above = before.clone();
            row = before.clone();
        }
        let columns = word.len().min(width - 1);
        for (j, cell) in above.iter_mut().enumerate().take(columns + 1) {
            *cell = j;
        }
        let mut far = false;
        for i in 1..=typed.len() {
            row[0] = i;
            let mut least = i;
            for j in 1..=columns {
                let cost = usize::from(typed[i - 1] != word[j - 1]);
                let mut best = (above[j] + 1).min(row[j - 1] + 1).min(above[j - 1] + cost);
                if i > 1 && j > 1 && typed[i - 1] == word[j - 2] && typed[i - 2] == word[j - 1] {
                    best = best.min(before[j - 2] + 1);
                }
                row[j] = best;
                least = least.min(best);
            }
            // Already too many typos.
            if least > allowed {
                far = true;
                break;
            }
            // Rotate: before <- above <- row <- old before.
            std::mem::swap(&mut before, &mut above);
            std::mem::swap(&mut above, &mut row);
        }
        if !far && above[1..=columns].iter().any(|&d| d <= allowed) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_word_needs_a_word_start() {
        assert!(starts_word("api", "team/api"));
        assert!(!starts_word("api", "rapid"));
        assert!(starts_word("api", "api.example"));
        assert!(starts_word(".com", "github.com")); // not a letter: anywhere
    }

    #[test]
    fn one_slip_is_forgiven_from_four_letters() {
        assert!(nearly(b"githb", b"github.com")); // missing letter
        assert!(nearly(b"gihtub", b"github.com")); // swapped
        assert!(nearly(b"gitgub", b"github.com")); // wrong letter
        assert!(!nearly(b"gti", b"git hub")); // under four: noise
        assert!(!nearly(b"zzzz", b"github.com"));
    }

    #[test]
    fn two_slips_only_in_long_words() {
        assert!(!nearly(b"gthbu", b"github.com"));
        assert!(nearly(b"stackovrflw", b"stackoverflow.com"));
    }

    #[test]
    fn matches_scores() {
        assert_eq!(matches(&["git"], "github git"), Some(1));
        assert_eq!(matches(&["githb"], "github"), Some(0));
        assert_eq!(matches(&["git", "zzzzz"], "github"), None);
    }
}
