//! Discourse-cue matching shared by every consumer of `discourse_cues.json`.
//!
//! A cue is a sequence of words, never a substring of the raw text: `"vs"` is
//! a cue, `"versus"` inside `"universus"` is not. The vocabulary stays data
//! (`lexicon().discourse.comparison_signals`); this module only locates it and
//! reports byte offsets, so a caller can attach a span to the source instead
//! of re-deriving one by searching a lowercased copy (which shifts offsets).

/// One word of the source text and its byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    /// Lowercased surface, used only for matching.
    pub lowered: String,
}

/// Splits `text` into word spans on every byte that is not alphanumeric or an
/// apostrophe. Offsets stay byte-exact against the original slice, so a caller
/// can ground a cue without a second tokenizer.
pub fn word_spans(text: &str) -> Vec<WordSpan> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, character) in text.char_indices() {
        if character.is_alphanumeric() || matches!(character, '\'' | '\u{2019}') {
            start.get_or_insert(offset);
            continue;
        }
        if let Some(begin) = start.take() {
            words.push(word_span(text, begin, offset));
        }
    }
    if let Some(begin) = start {
        words.push(word_span(text, begin, text.len()));
    }
    words
}

fn word_span(text: &str, start: usize, end: usize) -> WordSpan {
    WordSpan {
        start,
        end,
        lowered: text[start..end]
            .trim_matches(['\'', '\u{2019}'])
            .to_lowercase(),
    }
}

/// Byte span of the first comparison cue in `text`, if the lexicon lists one.
/// Multi-word cues are matched as a consecutive word sequence, and the first
/// (earliest) match wins.
pub fn comparison_cue(text: &str) -> Option<(usize, usize)> {
    let words = word_spans(text);
    if words.is_empty() {
        return None;
    }
    let cues = &lexicon().discourse.comparison_signals;
    let mut best: Option<(usize, usize)> = None;
    for (index, _) in words.iter().enumerate() {
        for cue in cues {
            if cue.is_empty() || index + cue.len() > words.len() {
                continue;
            }
            if !cue_matches(&words, index, cue) {
                continue;
            }
            let end = words[index + cue.len() - 1].end;
            let candidate = (words[index].start, end);
            if best.is_none_or(|current| candidate.0 < current.0) {
                best = Some(candidate);
            }
        }
    }
    best
}

/// True when `cue` matches the words starting at `index`.
///
/// A cue's final word may be a preposition that the text fuses with an article:
/// the vocabulary says `rispetto a` while Italian writes `rispetto allo`,
/// `rispetto alla`, `rispetto ai`. Fusing is grammar, not vocabulary, so the
/// last word also matches an articulated form from the shared stopword table.
fn cue_matches(words: &[WordSpan], index: usize, cue: &[String]) -> bool {
    if cue.len() > 1
        && cue[..cue.len() - 1]
            .iter()
            .enumerate()
            .all(|(offset, word)| words[index + offset].lowered == *word)
        && is_articulated_form(&words[index + cue.len() - 1].lowered, &cue[cue.len() - 1])
    {
        return true;
    }
    cue.iter()
        .enumerate()
        .all(|(offset, word)| words[index + offset].lowered == *word)
}

/// `token` is the cue's preposition alone ("a") or that preposition carrying
/// an article ("allo", "all'"). The article table is the shared lexicon's
/// `articulated_prepositions`, so no language knowledge lives here.
fn is_articulated_form(token: &str, preposition: &str) -> bool {
    if token == preposition {
        return true;
    }
    let articulated = &lexicon().stopwords.articulated_prepositions;
    let carries_article = articulated.iter().any(|form| form == token);
    carries_article && preposition.chars().count() == 1 && token.starts_with(preposition)
}

/// Number of words fully inside `[start, end)`. A caller uses it to reject a
/// "comparison" whose subjects are a whole clause away from the cue.
pub fn words_between(words: &[WordSpan], start: usize, end: usize) -> usize {
    words
        .iter()
        .filter(|word| word.start >= start && word.end <= end)
        .count()
}

fn lexicon() -> &'static crate::lexicon::Lexicon {
    crate::lexicon::lexicon()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_is_matched_on_word_boundaries_with_exact_offsets() {
        let text = "Il 42% degli uomini contro il 18% delle donne.";
        let (start, end) = comparison_cue(text).expect("contro is a comparison cue");
        assert_eq!(&text[start..end], "contro");
        // A cue inside a longer word is not a cue.
        assert_eq!(comparison_cue("Universus non è una parola italiana."), None);
    }

    #[test]
    fn cue_preposition_may_carry_an_article() {
        let text = "Lo Stadio Aurora è più capiente rispetto allo Stadio Olimpico.";
        let (start, end) = comparison_cue(text).expect("rispetto allo is rispetto a");
        assert_eq!(&text[start..end], "rispetto allo");
    }

    #[test]
    fn multilingual_comparison_cues_are_recognized() {
        for (text, expected) in [
            ("Aurora consiguió 41% frente al 15% de Lumen.", "frente al"),
            ("Aurora apresentou 41% contra 15% do grupo Lumen.", "contra"),
            ("Aurora obtient 41% contre 15% pour Lumen.", "contre"),
            ("Aurora behaalde 41% tegenover 15% voor Lumen.", "tegenover"),
            ("Aurora uzyskała 41% wobec 15% grupy Lumen.", "wobec"),
            ("Аврора получила 41% против 15% у группы Люмен.", "против"),
            (
                "Aurora mencatat 41% dibandingkan dengan 15% pada kelompok Lumen.",
                "dibandingkan dengan",
            ),
        ] {
            let (start, end) =
                comparison_cue(text).unwrap_or_else(|| panic!("missing cue in {text}"));
            assert_eq!(&text[start..end], expected);
        }
    }

    #[test]
    fn multi_word_cue_keeps_its_whole_span() {
        let text = "Sales compared with last year rose.";
        let (start, end) = comparison_cue(text).expect("compared with is a comparison cue");
        assert_eq!(&text[start..end], "compared with");
        assert_eq!(words_between(&word_spans(text), end, text.len()), 3);
    }
}
