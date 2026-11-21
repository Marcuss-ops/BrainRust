//! Explicit ages attached only to an unambiguous nearby PERSON mention.

use semantic_core::{NumericFact, PersonAgeFact, SentenceSpan, TemporalFact};
use visualner::VisualEntity;

pub fn extract(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    numbers: &[NumericFact],
    dates: &[TemporalFact],
) -> Vec<PersonAgeFact> {
    if entities.len() != entity_ids.len() {
        return Vec::new();
    }
    let language = language.split('-').next().unwrap_or(language);
    let mut output = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        for number in numbers.iter().filter(|number| {
            number.kind == "NUMBER"
                && number
                    .value
                    .is_some_and(|value| value.fract() == 0.0 && (0.0..=120.0).contains(&value))
                && number.start_byte >= sentence.start_byte
                && number.end_byte <= sentence.end_byte
        }) {
            let Some(age) = number.value.map(|value| value as u8) else {
                continue;
            };
            let local_start = number.start_byte - sentence.start_byte;
            let local_end = number.end_byte - sentence.start_byte;
            if !has_age_marker(sentence_text, local_start, local_end, language) {
                continue;
            }
            let mut people: Vec<(usize, usize)> = entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| {
                    entity.r#type == "PERSON"
                        && entity.start >= sentence.start_byte
                        && entity.end <= sentence.end_byte
                })
                .filter_map(|(index, entity)| {
                    entity_ids.get(index)?;
                    let distance = if entity.end <= number.start_byte {
                        number.start_byte - entity.end
                    } else if number.end_byte <= entity.start {
                        entity.start.saturating_sub(number.end_byte)
                    } else {
                        0
                    };
                    (distance <= 80).then_some((index, distance))
                })
                .collect();
            people.sort_by_key(|(index, distance)| (*distance, entities[*index].start));
            if people.is_empty() || people.get(1).is_some_and(|(_, d)| *d == people[0].1) {
                continue;
            }
            if people.len() > 1 && collective_age_marker(sentence_text, local_start, language) {
                continue;
            }
            let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
                continue;
            };
            let as_of = dates
                .iter()
                .filter(|date| {
                    date.start_byte >= sentence.start_byte && date.end_byte <= number.start_byte
                })
                .filter_map(|date| date.normalized.as_deref())
                .find(|normalized| {
                    normalized.len() >= 4 && normalized[..4].bytes().all(|b| b.is_ascii_digit())
                })
                .map(str::to_string);
            let dated_context = as_of.as_ref().is_some_and(|_| {
                let earlier_date_end = dates
                    .iter()
                    .filter(|date| {
                        date.start_byte >= sentence.start_byte
                            && date.end_byte <= number.start_byte
                            && date.normalized == as_of
                    })
                    .map(|date| date.end_byte)
                    .max();
                earlier_date_end
                    .and_then(|end| source.get(end..number.start_byte))
                    .is_some_and(|between| date_age_link(language, between))
            });
            let derived_birth_year = if dated_context {
                as_of
                    .as_ref()
                    .and_then(|date| date.get(..4))
                    .and_then(|year| year.parse::<i32>().ok())
                    .map(|year| year - i32::from(age))
            } else {
                None
            };
            output.push(PersonAgeFact {
                person_id: entity_ids[people[0].0].clone(),
                age,
                as_of: as_of.filter(|_| dated_context),
                derived_birth_year,
                start_byte: number.start_byte,
                end_byte: number.end_byte,
                sentence_index,
                evidence: evidence.to_string(),
            });
        }
    }
    output.sort_by_key(|fact| (fact.start_byte, fact.person_id.clone()));
    output
}

fn date_age_link(language: &str, between: &str) -> bool {
    let folded = between.to_lowercase();
    let cues: &[&str] = match language {
        "en" => &[" at ", " aged ", " age ", " was ", " were "],
        "it" => &[" all'età di ", " all’eta di ", " a ", " aveva "],
        "es" => &[" tenía ", " tenia ", " cumplió ", " cumplio "],
        "fr" => &[" âgé de ", " âgée de ", " avait "],
        "de" => &[" im alter von ", " war ", " wurde "],
        "pt" => &[" com ", " tinha "],
        "nl" => &[" op ", " was ", " werd "],
        "pl" => &[" w wieku ", " miała ", " miał ", " miało "],
        "ru" => &[" в возрасте ", " было ", " исполнилось "],
        "tr" => &[
            " yılında ",
            " yaşında ",
            " yasinda ",
            " yaşındaydı ",
            " yasindaydi ",
        ],
        "id" => &[" berusia ", " berumur "],
        _ => &[],
    };
    cues.iter().any(|cue| folded.contains(cue))
}

fn collective_age_marker(text: &str, number_start: usize, language: &str) -> bool {
    let prefix = text.get(..number_start).unwrap_or("").to_lowercase();
    let cues: &[&str] = match language {
        "en" => &["both ", "all ", "they are "],
        "it" => &["entrambi ", "entrambe ", "tutti e due "],
        "es" => &["ambos ", "ambas ", "los dos "],
        "fr" => &["tous deux ", "toutes deux ", "les deux "],
        "de" => &["beide ", "beiden "],
        "pt" => &["ambos ", "ambas ", "os dois "],
        "nl" => &["beiden ", "allebei "],
        "pl" => &["obaj ", "obie ", "oboje "],
        "ru" => &["оба ", "обе ", "оба они "],
        "tr" => &["ikisi de "],
        "id" => &["keduanya "],
        _ => &[],
    };
    cues.iter().any(|cue| prefix.contains(cue))
}

fn has_age_marker(text: &str, start: usize, end: usize, language: &str) -> bool {
    let before = text.get(..start).unwrap_or("").to_lowercase();
    let after = text.get(end..).unwrap_or("").to_lowercase();
    let number_phrase = text.get(start..end).unwrap_or("").to_lowercase();
    let before_markers: &[&str] = match language {
        "en" => &["aged ", "age "],
        // A bare `di N` / `a N` also matches ordinary counts (`due filoni`,
        // `cinque consulenze`). Require an age unit after the number or an
        // explicitly age-specific phrase before it.
        "it" => &["all'età di ", "all’eta di "],
        "es" => &["de ", "con "],
        "fr" => &["de ", "âgé de ", "age de "],
        "de" => &["im alter von ", "mit "],
        "pt" => &["de ", "com "],
        "nl" => &["van ", "op "],
        "pl" => &["w wieku ", "ma "],
        "ru" => &["в возрасте ", "лет "],
        "tr" => &["yaşında ", "yasinda "],
        "id" => &["berusia ", "umur "],
        _ => &[],
    };
    let after_markers: &[&str] = match language {
        "en" => &[" years old", " year old", "-year-old", "-years-old"],
        "it" => &[" anni", "enne"],
        "es" => &[" años", " anos"],
        "fr" => &[" ans"],
        "de" => &[" jahre", " jahren"],
        "pt" => &[" anos"],
        "nl" => &[" jaar"],
        "pl" => &[" lat"],
        "ru" => &[" лет", " года", " год"],
        "tr" => &[" yaşında", " yasinda"],
        "id" => &[" tahun"],
        _ => &[],
    };
    before_markers.iter().any(|marker| before.ends_with(marker))
        || after_markers.iter().any(|marker| after.starts_with(marker))
        || embedded_age_unit(&number_phrase, &before, &after, language)
}

/// Spellout extraction may include the following open-class age unit in the
/// numeric span (`thirty years`, `trente ans`). In that case the ordinary
/// after-span marker check cannot see it. Require a language-specific age
/// predicate before the number or an age-specific suffix after it; a bare
/// duration such as `worked thirty years` remains non-age data.
fn embedded_age_unit(number: &str, before: &str, after: &str, language: &str) -> bool {
    let (units, predicates, suffixes): (&[&str], &[&str], &[&str]) = match language {
        "en" => (
            &[" year", " years"],
            &[" aged ", " age ", " was ", " were "],
            &[" old"],
        ),
        "it" => (
            &[" anno", " anni"],
            &[" aveva ", " ha ", " all'età di ", " all’eta di "],
            &[],
        ),
        "es" => (
            &[" año", " años", " ano", " anos"],
            &[" tenía ", " tenia ", " cumplió ", " cumplio "],
            &[],
        ),
        "fr" => (
            &[" an", " ans"],
            &[" avait ", " âgé de ", " âgée de ", " age de "],
            &[],
        ),
        "pt" => (&[" ano", " anos"], &[" tinha ", " com "], &[]),
        "de" => (
            &[" jahr", " jahre", " jahren"],
            &[" war ", " im alter von "],
            &[" alt"],
        ),
        "nl" => (&[" jaar", " jaren"], &[" was ", " op ", " werd "], &[]),
        "pl" => (
            &[" rok", " lata", " lat"],
            &[" miała ", " miał ", " miało ", " w wieku "],
            &[],
        ),
        "ru" => (
            &[" год", " года", " лет"],
            &[" было ", " в возрасте ", " исполнилось "],
            &[],
        ),
        "tr" => (&[" yaşında", " yasinda"], &[" yaşında ", " yasinda "], &[]),
        "id" => (&[" tahun"], &[" berusia ", " berumur "], &[]),
        _ => return false,
    };
    let has_unit = units.iter().any(|unit| number.ends_with(unit));
    let has_predicate = predicates.iter().any(|cue| before.ends_with(cue));
    let has_age_suffix =
        suffixes.iter().any(|cue| after.starts_with(cue)) || (language == "tr" && has_unit);
    has_unit && (has_predicate || has_age_suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(source: &str, surface: &str, start_at: usize) -> VisualEntity {
        let start = source[start_at..].find(surface).unwrap() + start_at;
        VisualEntity {
            text: surface.into(),
            r#type: "PERSON".into(),
            score: 1.0,
            start,
            end: start + surface.len(),
            evidence: surface.into(),
        }
    }

    fn number(source: &str, surface: &str, value: f64) -> NumericFact {
        let start_byte = source.find(surface).unwrap();
        NumericFact {
            surface: surface.into(),
            kind: "NUMBER".into(),
            value: Some(value),
            start_byte,
            end_byte: start_byte + surface.len(),
            evidence: surface.into(),
            form: None,
        }
    }

    #[test]
    fn extracts_explicit_age_and_derives_birth_year_only_for_dated_age_context() {
        let source = "In 1998, Maya Chen was aged 30.";
        let span = SentenceSpan {
            start_byte: 0,
            end_byte: source.len(),
        };
        let entities = vec![person(source, "Maya Chen", 0)];
        let age = number(source, "30", 30.0);
        let date_start = source.find("1998").unwrap();
        let dates = vec![TemporalFact {
            surface: "1998".into(),
            kind: "DATE".into(),
            normalized: Some("1998".into()),
            precision: "year".into(),
            start_byte: date_start,
            end_byte: date_start + 4,
            evidence: "1998".into(),
            era: None,
            ambiguous: false,
            relative: None,
            derived: false,
        }];
        let facts = extract(
            source,
            "en",
            &[span],
            &entities,
            &["per_1".into()],
            &[age],
            &dates,
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].person_id, "per_1");
        assert_eq!(facts[0].age, 30);
        assert_eq!(facts[0].as_of.as_deref(), Some("1998"));
        assert_eq!(facts[0].derived_birth_year, Some(1968));
        assert_eq!(&source[facts[0].start_byte..facts[0].end_byte], "30");
    }

    #[test]
    fn dated_age_constructions_are_localized_across_all_supported_languages() {
        let examples = [
            ("en", "In 1998, Maya Chen was 30 years old."),
            ("it", "Nel 1998 Maya aveva 30 anni."),
            ("es", "En 1998, Maya tenía 30 años."),
            ("fr", "En 1998, Maya avait 30 ans."),
            ("de", "Im Jahr 1998 war Maya 30 Jahre alt."),
            ("pt", "Em 1998, Maya tinha 30 anos."),
            ("nl", "In 1998 was Maya 30 jaar oud."),
            ("pl", "W 1998 Maya miała 30 lat."),
            ("ru", "В 1998 году Maya было 30 лет."),
            ("tr", "1998 yılında Maya 30 yaşındaydı."),
            ("id", "Pada 1998, Maya berusia 30 tahun."),
        ];
        for (language, source) in examples {
            let year = number(source, "1998", 1998.0);
            let age = number(source, "30", 30.0);
            let date = TemporalFact {
                surface: "1998".into(),
                kind: "DATE".into(),
                normalized: Some("1998".into()),
                precision: "year".into(),
                start_byte: year.start_byte,
                end_byte: year.end_byte,
                evidence: "1998".into(),
                era: None,
                ambiguous: false,
                relative: None,
                derived: false,
            };
            let people = vec![person(source, "Maya", 0)];
            let facts = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &people,
                &["person:maya".into()],
                &[age],
                &[date],
            );
            assert_eq!(facts.len(), 1, "{language}: {source}");
            assert_eq!(facts[0].as_of.as_deref(), Some("1998"), "{language}");
            assert_eq!(facts[0].derived_birth_year, Some(1968), "{language}");
        }
    }

    #[test]
    fn collective_age_is_not_attributed_to_only_the_nearest_person() {
        let source = "Maya Chen and Alex Kim, both aged 30.";
        let spans = [SentenceSpan {
            start_byte: 0,
            end_byte: source.len(),
        }];
        let entities = vec![
            person(source, "Maya Chen", 0),
            person(source, "Alex Kim", 0),
        ];
        let facts = extract(
            source,
            "en",
            &spans,
            &entities,
            &["per_1".into(), "per_2".into()],
            &[number(source, "30", 30.0)],
            &[],
        );
        assert!(facts.is_empty());
    }

    #[test]
    fn italian_counts_after_di_are_not_person_ages() {
        for (source, numeral, value, person_name) in [
            (
                "Andrea Sempio, poiché si tratta di due filoni separati.",
                "due",
                2.0,
                "Andrea Sempio",
            ),
            (
                "Andrea Sempio ha parlato del deposito di cinque consulenze.",
                "cinque",
                5.0,
                "Andrea Sempio",
            ),
        ] {
            let spans = [SentenceSpan {
                start_byte: 0,
                end_byte: source.len(),
            }];
            let entities = vec![person(source, person_name, 0)];
            let facts = extract(
                source,
                "it",
                &spans,
                &entities,
                &["person:andrea".into()],
                &[number(source, numeral, value)],
                &[],
            );
            assert!(facts.is_empty(), "false age from {source}: {facts:?}");
        }
    }

    #[test]
    fn a_spelled_duration_is_not_misclassified_as_a_person_age() {
        let source = "Maya Chen worked thirty years of experience.";
        let spans = [SentenceSpan {
            start_byte: 0,
            end_byte: source.len(),
        }];
        let entities = vec![person(source, "Maya Chen", 0)];
        let facts = extract(
            source,
            "en",
            &spans,
            &entities,
            &["per_1".into()],
            &[number(source, "thirty years", 30.0)],
            &[],
        );
        assert!(facts.is_empty());
    }
}
