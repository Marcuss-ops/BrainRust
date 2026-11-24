//! Delimited and cued work-title candidates.

use super::super::location::starts_uppercase;
use super::super::tokenizer::Token;
use super::super::values::Candidate;

/// Delimited editorial titles are strong WORK evidence. Extract the text
/// inside italics/guillemets before ordinary noun phrases can split title
/// words into people, places, or visual concepts.
pub(crate) fn work_title_candidates(
    tokens: &[Token],
    source: &str,
    language: &str,
) -> Vec<Candidate> {
    let base_language = language.split(['-', '_']).next().unwrap_or(language);
    if !base_language.eq_ignore_ascii_case("it") {
        return Vec::new();
    }
    const CUE_HEADS: &[&str] = &["inchiesta", "libro", "saggio", "viaggio"];
    let mut out = Vec::new();
    for index in 0..tokens.len().saturating_sub(2) {
        let head = &tokens[index];
        if !starts_uppercase(&head.text)
            || !CUE_HEADS
                .iter()
                .any(|cue| head.text.eq_ignore_ascii_case(cue))
            || !tokens[index + 1].text.eq_ignore_ascii_case("su")
        {
            continue;
        }
        let mut last = index + 1;
        let mut title_words = 0usize;
        let mut cursor = index + 2;
        while cursor < tokens.len() {
            let token = &tokens[cursor];
            let gap = &source[tokens[cursor - 1].end..token.start];
            if gap
                .chars()
                .any(|ch| matches!(ch, '.' | '!' | '?' | ';' | ':' | '\n'))
            {
                break;
            }
            // Keep title-case words and the lowercase connectors that hold a
            // title together. A lowercase verb/preposition such as "nel" or
            // "pubblicò" ends the title instead of absorbing the next clause.
            let connector = matches!(
                token.text.to_lowercase().as_str(),
                "di" | "del"
                    | "dello"
                    | "della"
                    | "dei"
                    | "degli"
                    | "delle"
                    | "da"
                    | "dal"
                    | "dalla"
                    | "e"
                    | "ed"
                    | "la"
                    | "il"
                    | "lo"
                    | "l'"
                    | "i"
                    | "gli"
                    | "le"
            );
            if !starts_uppercase(&token.text) && !connector {
                break;
            }
            last = cursor;
            title_words += 1;
            cursor += 1;
        }
        if title_words == 0 {
            continue;
        }
        let start = head.start;
        let end = tokens[last].end;
        let Some(text) = source.get(start..end) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        out.push(Candidate {
            text: text.to_string(),
            start,
            end,
            entity_type: Some("WORK".to_string()),
        });
    }
    out
}

pub(crate) fn delimited_work_candidates(source: &str) -> Vec<Candidate> {
    let mut spans = Vec::new();
    for (open, close) in [('*', '*'), ('«', '»'), ('“', '”')] {
        let mut opening = None;
        for (offset, ch) in source.char_indices() {
            if ch == open {
                if opening.is_none() {
                    opening = Some(offset + ch.len_utf8());
                } else if open == close {
                    if let Some(start) = opening.take() {
                        spans.push((start, offset));
                    }
                }
            } else if ch == close && open != close {
                if let Some(start) = opening.take() {
                    spans.push((start, offset));
                }
            }
        }
    }
    spans
        .into_iter()
        .filter_map(|(start, end)| {
            let delimited = source.get(start..end)?;
            let leading_bytes = delimited.len() - delimited.trim_start().len();
            let raw = delimited.trim();
            let text_start = start + leading_bytes;
            let text_end = text_start + raw.len();
            if raw.is_empty() || !raw.chars().next().is_some_and(char::is_uppercase) {
                return None;
            }
            Some(Candidate {
                text: raw.to_string(),
                start: text_start,
                end: text_end,
                entity_type: Some("WORK".to_string()),
            })
        })
        .collect()
}

#[cfg(test)]
mod editorial_title_tests {
    use super::delimited_work_candidates;

    #[test]
    fn italic_and_guillemet_delimiters_protect_work_titles() {
        let source = "*Quel treno da Vienna* e «La torre di Babele».";
        let works = delimited_work_candidates(source);
        assert_eq!(works.len(), 2, "{works:?}");
        for title in ["Quel treno da Vienna", "La torre di Babele"] {
            let start = source.find(title).unwrap();
            assert!(works.iter().any(|work| {
                work.text == title
                    && work.entity_type.as_deref() == Some("WORK")
                    && work.start == start
                    && work.end == start + title.len()
            }));
        }
    }
}
