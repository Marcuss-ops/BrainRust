//! Exact, conservative links from grounded LOCATION mentions to the packaged
//! country/capital index. Ambiguous aliases never receive coordinates.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use semantic_core::{Place, SentenceSpan};
use serde::Deserialize;
use visualner::VisualEntity;

const INDEX_JSON: &str = include_str!("../../lexicon/place_index_v1.json");
const ROLES_JSON: &str = include_str!("../../lexicon/place_roles.json");
const KIND_CUES_JSON: &str = include_str!("../../lexicon/place_kind_cues.json");

#[derive(Debug, Deserialize)]
struct IndexFile {
    entries: Vec<IndexEntry>,
}

#[derive(Debug, Deserialize)]
struct PlaceRoleFile {
    languages: BTreeMap<String, RoleCues>,
}

#[derive(Debug, Deserialize)]
struct PlaceKindCueFile {
    languages: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

fn place_kind_cue(prefix: &str, language: &str) -> Option<String> {
    static CUES: OnceLock<PlaceKindCueFile> = OnceLock::new();
    let file = CUES.get_or_init(|| {
        serde_json::from_str(KIND_CUES_JSON).expect("embedded place kind cues must be valid JSON")
    });
    let language = language.split('-').next().unwrap_or(language);
    let prefix = prefix.trim_end().to_lowercase();
    file.languages
        .get(language)?
        .iter()
        .find_map(|(kind, cues)| {
            cues.iter()
                .any(|cue| {
                    let cue = cue.to_lowercase();
                    prefix.strip_suffix(&cue).is_some_and(|before| {
                        before.is_empty()
                            || before
                                .chars()
                                .next_back()
                                .is_some_and(|ch| ch.is_whitespace() || ch == '\'' || ch == '’')
                    })
                })
                .then(|| kind.clone())
        })
}

#[derive(Debug, Deserialize)]
struct RoleCues {
    origin: Vec<String>,
    destination: Vec<String>,
    setting: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    id: String,
    name: String,
    kind: String,
    lat: f64,
    lon: f64,
    parent_ids: Vec<String>,
    aliases: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    context_terms: Vec<String>,
    #[serde(default)]
    counter_terms: Vec<String>,
}

fn index() -> &'static [IndexEntry] {
    static ENTRIES: OnceLock<Vec<IndexEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        serde_json::from_str::<IndexFile>(INDEX_JSON)
            .expect("embedded place index must be valid JSON")
            .entries
    })
}

fn role_cues(language: &str) -> Option<&'static RoleCues> {
    static CUES: OnceLock<PlaceRoleFile> = OnceLock::new();
    CUES.get_or_init(|| {
        serde_json::from_str(ROLES_JSON).expect("embedded place role cues must be valid JSON")
    })
    .languages
    .get(language.split('-').next().unwrap_or(language))
}

fn place_role(sentence_prefix: &str, language: &str) -> &'static str {
    let Some(cues) = role_cues(language) else {
        return "mention";
    };
    let prefix = sentence_prefix.trim_end().to_lowercase();
    for (role, terms) in [
        ("origin", &cues.origin),
        ("destination", &cues.destination),
        ("setting", &cues.setting),
    ] {
        if terms.iter().any(|term| {
            let term = term.to_lowercase();
            prefix.strip_suffix(&term).is_some_and(|before| {
                before.is_empty()
                    || before
                        .chars()
                        .next_back()
                        .is_some_and(|ch| ch.is_whitespace() || ch == '\'')
            })
        }) {
            return role;
        }
    }
    "mention"
}

fn key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn term_hits(text: &str, terms: &[String]) -> usize {
    let lowered = text.to_lowercase();
    terms
        .iter()
        .filter(|term| {
            let needle = term.to_lowercase();
            !needle.is_empty()
                && lowered.match_indices(&needle).any(|(start, matched)| {
                    let end = start + matched.len();
                    let before = lowered[..start].chars().next_back();
                    let after = lowered[end..].chars().next();
                    !before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric)
                })
        })
        .count()
}

fn candidates(surface: &str, language: &str) -> Vec<&'static IndexEntry> {
    let lookup = key(surface);
    let canonical = semantic_core::lexicon::lexicon().canonical_place(surface, language);
    let language = language.split('-').next().unwrap_or(language);
    index()
        .iter()
        .filter(|entry| {
            key(&entry.name) == lookup
                || entry
                    .aliases
                    .values()
                    .flatten()
                    .any(|alias| key(alias) == lookup)
                || canonical.is_some_and(|name| {
                    entry
                        .aliases
                        .get(language)
                        .is_some_and(|aliases| aliases.iter().any(|alias| key(alias) == key(name)))
                })
        })
        .collect()
}

/// Link exact grounded place surfaces. A collision is resolved only if a
/// same-sentence, uniquely linked place is an indexed parent of one candidate.
pub fn extract(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
) -> Vec<Place> {
    if entities.len() != entity_ids.len() {
        return Vec::new();
    }
    let mut matches: Vec<Option<Vec<&IndexEntry>>> = vec![None; entities.len()];
    let mut sentence_indices = vec![usize::MAX; entities.len()];
    for (i, entity) in entities.iter().enumerate() {
        if entity.r#type != "LOCATION"
            || source.get(entity.start..entity.end) != Some(entity.text.as_str())
        {
            continue;
        }
        let Some((sentence_index, _)) = sentences
            .iter()
            .enumerate()
            .find(|(_, span)| entity.start >= span.start_byte && entity.end <= span.end_byte)
        else {
            continue;
        };
        sentence_indices[i] = sentence_index;
        let sentence = &sentences[sentence_index];
        let prefix = source.get(sentence.start_byte..entity.start).unwrap_or("");
        let options = candidates(&entity.text, language);
        let explicit_kind = place_kind_cue(prefix, language);
        matches[i] = Some(match explicit_kind {
            Some(kind) => options
                .into_iter()
                .filter(|entry| entry.kind == kind)
                .collect(),
            None => options,
        });
    }

    let mut output = Vec::new();
    for (i, entity) in entities.iter().enumerate() {
        let Some(options) = matches[i].as_ref() else {
            continue;
        };
        let sentence_index = sentence_indices[i];
        let sentence = &sentences[sentence_index];
        let before = source.get(sentence.start_byte..entity.start).unwrap_or("");
        let after = source.get(entity.end..sentence.end_byte).unwrap_or("");
        let local_context = format!("{before} {after}");
        let context_ids: Vec<&str> = matches
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i && sentence_indices[*j] == sentence_index)
            .filter_map(|(_, found)| found.as_ref())
            .filter(|found| found.len() == 1)
            .filter_map(|found| found.first().map(|entry| entry.id.as_str()))
            .collect();

        let mut ranked: Vec<(&IndexEntry, usize, usize)> = options
            .iter()
            .filter(|candidate| term_hits(&local_context, &candidate.counter_terms) == 0)
            .map(|candidate| {
                let parent_coherence = candidate
                    .parent_ids
                    .iter()
                    .filter(|parent| context_ids.contains(&parent.as_str()))
                    .count();
                let context_overlap = term_hits(&local_context, &candidate.context_terms);
                (*candidate, parent_coherence, context_overlap)
            })
            .collect();
        ranked.sort_by_key(|(_, parent, context)| std::cmp::Reverse((*parent, *context)));
        let linked = match ranked.as_slice() {
            [(entry, parent, context), (_, next_parent, next_context)]
                if (*parent, *context) > (*next_parent, *next_context)
                    && (*parent > 0 || *context > 0) =>
            {
                Some(*entry)
            }
            [(entry, _, _)] => Some(*entry),
            _ => None,
        };

        let Some(evidence) = source.get(entity.start..entity.end) else {
            continue;
        };
        let sentence = &sentences[sentence_index];
        let prefix = source.get(sentence.start_byte..entity.start).unwrap_or("");
        let (kb_id, kind, lat, lon, parent_ids, link_status) = match linked {
            Some(entry) => (
                Some(entry.id.clone()),
                entry.kind.clone(),
                Some(entry.lat),
                Some(entry.lon),
                entry.parent_ids.clone(),
                "linked".to_string(),
            ),
            None if options.is_empty() => (
                None,
                place_kind_cue(prefix, language).unwrap_or_else(|| "unknown".to_string()),
                None,
                None,
                Vec::new(),
                "unlinked".to_string(),
            ),
            None => (
                None,
                place_kind_cue(prefix, language).unwrap_or_else(|| "unknown".to_string()),
                None,
                None,
                Vec::new(),
                "ambiguous".to_string(),
            ),
        };
        output.push(Place {
            id: format!("pl_{:04}", output.len() + 1),
            entity_local_id: entity_ids[i].clone(),
            kb_id,
            link_status,
            kind,
            name: evidence.to_string(),
            lat,
            lon,
            parent_ids,
            role: place_role(prefix, language).to_string(),
            start_byte: entity.start,
            end_byte: entity.end,
            sentence_index,
            evidence: evidence.to_string(),
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(source: &str, surface: &str, kind: &str) -> VisualEntity {
        let start = source.find(surface).unwrap();
        VisualEntity {
            text: surface.to_string(),
            r#type: kind.to_string(),
            score: 0.9,
            start,
            end: start + surface.len(),
            evidence: surface.to_string(),
        }
    }

    #[test]
    fn unique_alias_links_with_coordinates_and_exact_span() {
        let source = "France has a coastline.";
        let entities = vec![entity(source, "France", "LOCATION")];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["loc_1".into()],
        );
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].kb_id.as_deref(), Some("wd:Q142"));
        assert_eq!(places[0].kind, "country");
        assert!(places[0].lat.is_some() && places[0].lon.is_some());
        assert_eq!(&source[places[0].start_byte..places[0].end_byte], "France");
    }

    #[test]
    fn localized_wikidata_labels_link_in_all_supported_languages() {
        let examples = [
            ("en", "Germany is in Europe.", "Germany"),
            ("it", "La Germania è in Europa.", "Germania"),
            ("de", "Deutschland liegt in Europa.", "Deutschland"),
            ("es", "Alemania está en Europa.", "Alemania"),
            ("fr", "L'Allemagne est en Europe.", "Allemagne"),
            ("id", "Jerman berada di Eropa.", "Jerman"),
            ("nl", "Duitsland ligt in Europa.", "Duitsland"),
            ("pl", "Niemcy leżą w Europie.", "Niemcy"),
            ("pt", "A Alemanha fica na Europa.", "Alemanha"),
            ("ru", "Германия находится в Европе.", "Германия"),
            ("tr", "Almanya Avrupa'dadır.", "Almanya"),
        ];

        for (language, source, surface) in examples {
            let entities = vec![entity(source, surface, "LOCATION")];
            let places = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &entities,
                &["loc_germany".into()],
            );
            assert_eq!(places.len(), 1, "{language}: {places:?}");
            assert_eq!(places[0].link_status, "linked", "{language}: {places:?}");
            assert_eq!(places[0].kb_id.as_deref(), Some("wd:Q183"));
            assert_eq!(places[0].kind, "country");
            assert_eq!(&source[places[0].start_byte..places[0].end_byte], surface);
        }
    }

    #[test]
    fn inflected_polish_russian_and_turkish_places_link_with_source_spans() {
        let examples = [
            (
                "pl",
                "Dziennikarz pracuje w Warszawie.",
                "Warszawie",
                "wd:Q270",
            ),
            ("ru", "Журналист работает в Москве.", "Москве", "wd:Q649"),
            ("tr", "Gazeteci Ankara'da yaşıyor.", "Ankara'da", "wd:Q3640"),
        ];

        for (language, source, surface, expected_id) in examples {
            let entities = vec![entity(source, surface, "LOCATION")];
            let places = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &entities,
                &["loc_1".into()],
            );
            assert_eq!(places.len(), 1, "{language}: {places:?}");
            assert_eq!(places[0].link_status, "linked", "{language}: {places:?}");
            assert_eq!(places[0].kb_id.as_deref(), Some(expected_id));
            assert!(places[0].lat.is_some() && places[0].lon.is_some());
            assert_eq!(&source[places[0].start_byte..places[0].end_byte], surface);
        }
    }

    #[test]
    fn ambiguous_alias_never_receives_coordinates() {
        let source = "Luxembourg is mentioned.";
        let entities = vec![entity(source, "Luxembourg", "LOCATION")];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["loc_1".into()],
        );
        assert_eq!(places[0].link_status, "ambiguous");
        assert_eq!(places[0].kind, "unknown");
        assert!(places[0].kb_id.is_none());
        assert!(places[0].lat.is_none() && places[0].lon.is_none());
    }

    #[test]
    fn explicit_city_and_country_cues_resolve_indexed_alias_collisions() {
        for (source, surface, expected_id, expected_kind) in [
            (
                "The city of Luxembourg is near France.",
                "Luxembourg",
                "wd:Q1842",
                "city",
            ),
            (
                "The country of Luxembourg borders France.",
                "Luxembourg",
                "wd:Q32",
                "country",
            ),
            (
                "Luxembourg (LU) is shown on the map.",
                "Luxembourg",
                "wd:Q32",
                "country",
            ),
        ] {
            let entities = vec![entity(source, surface, "LOCATION")];
            let places = extract(
                source,
                "en",
                &semantic_core::split_sentences(source, "en", None),
                &entities,
                &["loc_luxembourg".into()],
            );
            assert_eq!(places.len(), 1, "{source}: {places:?}");
            assert_eq!(places[0].link_status, "linked", "{source}: {places:?}");
            assert_eq!(places[0].kb_id.as_deref(), Some(expected_id));
            assert_eq!(places[0].kind, expected_kind);
            assert!(places[0].lat.is_some() && places[0].lon.is_some());
        }

        let source = "The country of Paris is fictional.";
        let entities = vec![entity(source, "Paris", "LOCATION")];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["loc_paris".into()],
        );
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].link_status, "unlinked");
        assert_eq!(places[0].kind, "country");
        assert!(places[0].kb_id.is_none());
        assert!(places[0].lat.is_none() && places[0].lon.is_none());
    }

    #[test]
    fn repeated_collision_alias_without_parent_context_stays_ambiguous() {
        let source = "Panama is in Panama.";
        let entities = vec![
            entity(source, "Panama", "LOCATION"),
            VisualEntity {
                text: "Panama".to_string(),
                r#type: "LOCATION".to_string(),
                score: 0.9,
                start: source.rfind("Panama").unwrap(),
                end: source.rfind("Panama").unwrap() + "Panama".len(),
                evidence: "Panama".to_string(),
            },
        ];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["loc_country".into(), "loc_city".into()],
        );
        // With identical aliases and no independently grounded parent, neither
        // mention is allowed to manufacture a country/city distinction.
        assert!(places.iter().all(|place| place.link_status == "ambiguous"));
        assert!(places.iter().all(|place| place.lat.is_none()));
    }

    #[test]
    fn non_location_or_bad_source_span_is_not_projected() {
        let source = "France has a coastline.";
        let mut bad = entity(source, "France", "LOCATION");
        bad.text = "Paris".to_string();
        let entities = vec![entity(source, "France", "PERSON"), bad];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["person_1".into(), "bad_1".into()],
        );
        assert!(places.is_empty());
    }

    #[test]
    fn explicit_class_cue_types_an_unlinked_place_without_coordinates() {
        let source = "The river Zambezi flooded nearby.";
        let entities = vec![entity(source, "Zambezi", "LOCATION")];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["loc_zambezi".into()],
        );
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].kind, "river");
        assert_eq!(places[0].link_status, "unlinked");
        assert!(places[0].kb_id.is_none());
        assert!(places[0].lat.is_none() && places[0].lon.is_none());
    }

    #[test]
    fn explicit_local_prepositions_assign_place_roles() {
        let source = "They drove from Paris to Rome.";
        let entities = vec![
            entity(source, "Paris", "LOCATION"),
            entity(source, "Rome", "LOCATION"),
        ];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["paris".into(), "rome".into()],
        );
        assert_eq!(
            places
                .iter()
                .map(|place| place.role.as_str())
                .collect::<Vec<_>>(),
            ["origin", "destination"]
        );
    }

    #[test]
    fn italian_origin_and_destination_cues_use_localized_forms() {
        let source = "Da Roma verso Parigi.";
        let entities = vec![
            entity(source, "Roma", "LOCATION"),
            entity(source, "Parigi", "LOCATION"),
        ];
        let places = extract(
            source,
            "it",
            &semantic_core::split_sentences(source, "it", None),
            &entities,
            &["roma".into(), "parigi".into()],
        );
        assert_eq!(places[0].role, "origin");
        assert_eq!(places[1].role, "destination");
    }

    #[test]
    fn unmarked_place_keeps_mention_role() {
        let source = "Paris is famous.";
        let entities = vec![entity(source, "Paris", "LOCATION")];
        let places = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["paris".into()],
        );
        assert_eq!(places[0].role, "mention");
    }

    #[test]
    fn role_cues_cover_every_supported_language_and_each_role() {
        for language in [
            "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
        ] {
            let cues = role_cues(language).expect("every language must have a role cue pack");
            assert!(
                !cues.origin.is_empty(),
                "missing origin cues for {language}"
            );
            assert!(
                !cues.destination.is_empty(),
                "missing destination cues for {language}"
            );
            assert!(
                !cues.setting.is_empty(),
                "missing setting cues for {language}"
            );
        }
    }
}
