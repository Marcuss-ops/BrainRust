//! Scale and unit suffix attachment for spelled-number candidates.

pub(super) fn quantity_suffix(source: &str, end: usize) -> Option<(&'static str, &'static str)> {
    let tail = source.get(end..)?.trim_start();
    let lower = tail.to_lowercase();
    let suffixes = [
        ("percent", "PERCENT"),
        ("per cent", "PERCENT"),
        ("per cento", "PERCENT"),
        ("pour cent", "PERCENT"),
        ("por cento", "PERCENT"),
        ("percento", "PERCENT"),
        ("prozent", "PERCENT"),
        ("процентов", "PERCENT"),
        ("процента", "PERCENT"),
        ("процент", "PERCENT"),
        ("dollars", "MONEY"),
        ("dollar", "MONEY"),
        ("euros", "MONEY"),
        ("euro", "MONEY"),
        ("pounds", "MONEY"),
        ("pound", "MONEY"),
        ("yen", "MONEY"),
        ("рублей", "MONEY"),
        ("рубля", "MONEY"),
        ("million", "MONEY"),
        ("billion", "MONEY"),
        ("trillion", "MONEY"),
        ("mille", "NUMBER"),
        ("milioni", "NUMBER"),
        ("millions", "NUMBER"),
        ("orders", "NUMBER"),
        ("people", "NUMBER"),
        ("years", "TIME"),
        ("year", "TIME"),
        ("days", "TIME"),
        ("day", "TIME"),
        ("hours", "TIME"),
        ("hour", "TIME"),
        ("minutes", "TIME"),
        ("minute", "TIME"),
        ("votes", "NUMBER"),
        ("ordini", "NUMBER"),
        ("persone", "NUMBER"),
        ("ans", "NUMBER"),
        ("jahre", "NUMBER"),
        ("bestellungen", "NUMBER"),
        ("años", "NUMBER"),
        ("personas", "NUMBER"),
        ("pessoas", "NUMBER"),
        ("personnes", "NUMBER"),
        ("menschen", "NUMBER"),
        ("mensen", "NUMBER"),
        ("osób", "NUMBER"),
        ("человек", "NUMBER"),
        ("kişi", "NUMBER"),
        ("insan", "NUMBER"),
        ("orang", "NUMBER"),
    ];
    suffixes.into_iter().find(|(suffix, _)| {
        lower.starts_with(suffix)
            && lower
                .chars()
                .nth(suffix.chars().count())
                .is_none_or(|next| !next.is_alphanumeric())
    })
}

/// True when the token immediately after `end` is an open-class word, which
/// makes it a unit for the preceding spelled quantity. A preposition or
/// article from the language pack means the number is a bare count ("quaranta
/// dei tecnici"), not a measurement. Without a pack the check fails closed:
/// an unprovable unit never enters the numeric projection.
pub(super) fn next_token_is_open_class(source: &str, end: usize, language: &str) -> bool {
    let Some(tail) = source.get(end..) else {
        return false;
    };
    let tail = tail.trim_start();
    let Some(first) = tail.chars().next() else {
        return false;
    };
    if !first.is_alphanumeric() {
        return false;
    }
    let word_end = tail
        .char_indices()
        .find(|(_, character)| !character.is_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(tail.len());
    let word = tail[..word_end].to_lowercase();
    semantic_core::language_packs::get(language)
        .map(|pack| !pack.is_stopword(&word))
        .unwrap_or(false)
}

/// Scale word among the tokens consumed by [`extend_quantity`]. The table is
/// `semantic_core::scale_word_factor`, so a new scale spelling is a data edit
/// in `lexicon/scale_words.json` (and its factor table), never a second copy.
pub(super) fn suffix_scale_factor(source: &str, end: usize, candidate_end: usize) -> Option<f64> {
    source
        .get(end..candidate_end)?
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .find_map(semantic_core::scale_word_factor)
}

pub(super) fn extend_quantity(
    source: &str,
    end: usize,
    default_type: &'static str,
) -> (usize, &'static str) {
    let Some((suffix, kind)) = quantity_suffix(source, end) else {
        return (end, default_type);
    };
    let suffix_start = end + source[end..].len() - source[end..].trim_start().len();
    (
        suffix_start + suffix.len(),
        if kind == "TIME" { "NUMBER" } else { kind },
    )
}
