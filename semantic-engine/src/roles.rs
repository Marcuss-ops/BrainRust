//! Conservative person-role projection from explicit title lexicons.
//!
//! A role is emitted only when its complete surface form is grounded and a
//! PERSON entity is nearby in the same sentence with an appositive/copular
//! connection. Scope is attached only from a grounded entity in that sentence.

use semantic_core::{PersonRole, RoleRank, RoleStatus, RoleTense, SentenceSpan, TemporalFact};
use serde::Deserialize;
use visualner::VisualEntity;

const EN: &str = include_str!("../../lexicon/roles/en.json");
const IT: &str = include_str!("../../lexicon/roles/it.json");
const ES: &str = include_str!("../../lexicon/roles/es.json");
const DE: &str = include_str!("../../lexicon/roles/de.json");
const FR: &str = include_str!("../../lexicon/roles/fr.json");
const ID: &str = include_str!("../../lexicon/roles/id.json");
const NL: &str = include_str!("../../lexicon/roles/nl.json");
const PL: &str = include_str!("../../lexicon/roles/pl.json");
const PT: &str = include_str!("../../lexicon/roles/pt.json");
const RU: &str = include_str!("../../lexicon/roles/ru.json");
const TR: &str = include_str!("../../lexicon/roles/tr.json");

#[derive(Debug, Deserialize)]
struct RoleFile {
    language: String,
    /// Data-only copular/appositive connectors between a person and a role
    /// surface (` is `, ` est `, ` jest `, ` adalah `, ...). They live in the
    /// language pack instead of a Rust list so a new language is data, and an
    /// unknown language degrades to punctuation-only attachment.
    #[serde(default)]
    attach_markers: Vec<String>,
    roles: Vec<RoleEntry>,
    tense_markers: TenseMarkers,
}

#[derive(Debug, Deserialize)]
struct RoleEntry {
    role_code: String,
    scope: String,
    surfaces: Vec<RoleSurface>,
}

#[derive(Debug, Deserialize)]
struct RoleSurface {
    text: String,
    #[serde(default)]
    gender: String,
}

#[derive(Debug, Deserialize)]
struct TenseMarkers {
    former: Vec<String>,
    acting: Vec<String>,
    appointed: Vec<String>,
    resigned: Vec<String>,
}

fn role_file(language: &str) -> Option<RoleFile> {
    let source = match language.split('-').next().unwrap_or(language) {
        "en" => EN,
        "it" => IT,
        "es" => ES,
        "de" => DE,
        "fr" => FR,
        "id" => ID,
        "nl" => NL,
        "pl" => PL,
        "pt" => PT,
        "ru" => RU,
        "tr" => TR,
        _ => return None,
    };
    serde_json::from_str(source).ok()
}

pub fn extract(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    temporal_facts: &[TemporalFact],
) -> Vec<PersonRole> {
    let Some(file) = role_file(language) else {
        return Vec::new();
    };
    if entities.len() != entity_ids.len()
        || file.language != language.split('-').next().unwrap_or(language)
    {
        return Vec::new();
    }

    let mut output = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        for role in &file.roles {
            for surface in &role.surfaces {
                for (local_start, local_end) in whole_word_spans(sentence_text, &surface.text) {
                    let start = sentence.start_byte + local_start;
                    let end = sentence.start_byte + local_end;
                    let person =
                        nearest_entity(entities, entity_ids, sentence, start, end, "PERSON");
                    let Some((person_index, distance)) = person else {
                        continue;
                    };
                    if distance > 80
                        || !attached(
                            source,
                            entities[person_index].start,
                            entities[person_index].end,
                            start,
                            end,
                            &file.attach_markers,
                        )
                    {
                        continue;
                    }

                    let scoped_type = match role.scope.as_str() {
                        "org" => Some("ORG"),
                        "place" => Some("LOCATION"),
                        _ => None,
                    };
                    let scoped_entity = scoped_type.and_then(|kind| {
                        nearest_entity(entities, entity_ids, sentence, start, end, kind)
                            .filter(|(_, distance)| *distance <= 80)
                            .map(|(index, _)| entity_ids[index].clone())
                    });
                    let (organization_id, place_id) = match role.scope.as_str() {
                        "org" => (scoped_entity, None),
                        "place" => (None, scoped_entity),
                        _ => (None, None),
                    };
                    let context = local_context(sentence_text, local_start, local_end);
                    let temporal = temporal_facts
                        .iter()
                        .find(|fact| {
                            fact.start_byte >= sentence.start_byte
                                && fact.end_byte <= sentence.end_byte
                                && fact.precision != semantic_core::dates::PRECISION_UNRESOLVED
                                && !fact.derived
                        })
                        .cloned();
                    let tense = infer_tense(context, &file.tense_markers);
                    let rank = rank_for(
                        &role.role_code,
                        tense,
                        has_marker(context, &file.tense_markers.acting),
                    );
                    let evidence = sentence_text.to_string();
                    output.push(PersonRole {
                        id: format!("pr_{sentence_index}_{start}"),
                        person_id: entity_ids[person_index].clone(),
                        role_code: role.role_code.clone(),
                        role_surface: source[start..end].to_string(),
                        gender: match surface.gender.as_str() {
                            "masculine" => "masculine".to_string(),
                            "feminine" => "feminine".to_string(),
                            _ => "unknown".to_string(),
                        },
                        rank,
                        organization_id,
                        place_id,
                        temporal,
                        tense,
                        status: infer_status(context),
                        start_byte: start,
                        end_byte: end,
                        sentence_index,
                        evidence,
                    });
                }
            }
        }
    }
    output.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| a.role_code.cmp(&b.role_code))
            .then_with(|| a.person_id.cmp(&b.person_id))
    });
    output.dedup_by(|a, b| a.start_byte == b.start_byte && a.person_id == b.person_id);
    output
}

fn whole_word_spans(text: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for (start, _) in text.char_indices() {
        let Some(end) = start.checked_add(needle.len()) else {
            continue;
        };
        let Some(candidate) = text.get(start..end) else {
            continue;
        };
        if !candidate.eq_ignore_ascii_case(needle) {
            continue;
        }
        let before = text.get(..start).and_then(|s| s.chars().next_back());
        let after = text.get(end..).and_then(|s| s.chars().next());
        if before.is_some_and(char::is_alphanumeric) || after.is_some_and(char::is_alphanumeric) {
            continue;
        }
        spans.push((start, end));
    }
    spans
}

fn nearest_entity(
    entities: &[VisualEntity],
    ids: &[String],
    sentence: &SentenceSpan,
    start: usize,
    end: usize,
    kind: &str,
) -> Option<(usize, usize)> {
    let mut matches: Vec<(usize, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| {
            entity.r#type == kind
                && entity.start >= sentence.start_byte
                && entity.end <= sentence.end_byte
        })
        .filter_map(|(index, entity)| {
            ids.get(index)?;
            let distance = if entity.end <= start {
                start - entity.end
            } else if end <= entity.start {
                entity.start.saturating_sub(end)
            } else {
                0
            };
            Some((index, distance))
        })
        .collect();
    matches.sort_by_key(|(index, distance)| (*distance, entities[*index].start));
    if matches
        .get(1)
        .is_some_and(|(_, next_distance)| *next_distance == matches[0].1)
    {
        return None;
    }
    matches.first().copied()
}

fn local_context(text: &str, start: usize, end: usize) -> &str {
    let mut left = start.saturating_sub(48);
    let mut right = end.saturating_add(24).min(text.len());
    while !text.is_char_boundary(left) {
        left -= 1;
    }
    while !text.is_char_boundary(right) {
        right += 1;
    }
    text.get(left..right).unwrap_or("")
}

fn attached(
    source: &str,
    person_start: usize,
    person_end: usize,
    role_start: usize,
    role_end: usize,
    markers: &[String],
) -> bool {
    let (left, right) = if person_end <= role_start {
        (person_end, role_start)
    } else if role_end <= person_start {
        (role_end, person_start)
    } else {
        return false;
    };
    let Some(gap) = source.get(left..right) else {
        return false;
    };
    let trimmed = gap.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|ch| matches!(ch, ',' | ':' | '—' | '–' | '-'))
    {
        return true;
    }
    let haystack = format!(" {trimmed} ").to_lowercase();
    markers
        .iter()
        .any(|marker| haystack.contains(&marker.to_lowercase()))
}

fn infer_tense(sentence: &str, markers: &TenseMarkers) -> RoleTense {
    for (terms, tense) in [
        (&markers.former, RoleTense::Former),
        (&markers.appointed, RoleTense::Appointed),
        (&markers.resigned, RoleTense::Resigned),
    ] {
        if has_marker(sentence, terms) {
            return tense;
        }
    }
    RoleTense::Unknown
}

fn has_marker(sentence: &str, markers: &[String]) -> bool {
    let lower = format!(" {} ", sentence.to_lowercase());
    markers
        .iter()
        .any(|term| lower.contains(&format!(" {} ", term.to_lowercase())))
}

fn rank_for(code: &str, tense: RoleTense, acting: bool) -> RoleRank {
    match tense {
        RoleTense::Former => RoleRank::Former,
        _ if acting => RoleRank::Acting,
        RoleTense::Unknown if code.starts_with("vice_") => RoleRank::Deputy,
        RoleTense::Unknown
            if matches!(
                code,
                "chief_executive"
                    | "president"
                    | "chief_operating_officer"
                    | "chief_financial_officer"
                    | "chief_technology_officer"
                    | "prime_minister"
                    | "mayor"
                    | "governor"
                    | "chairperson"
            ) =>
        {
            RoleRank::Head
        }
        _ => RoleRank::Member,
    }
}

fn infer_status(sentence: &str) -> RoleStatus {
    let lower = format!(" {} ", sentence.to_lowercase());
    if [" not ", " no longer ", " non ", " mai "]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        RoleStatus::Negated
    } else if [" allegedly ", " forse ", " presunto ", " reportedly "]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        RoleStatus::Uncertain
    } else {
        RoleStatus::Asserted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(text: &str, kind: &str, start: usize) -> VisualEntity {
        VisualEntity {
            text: text.to_string(),
            r#type: kind.to_string(),
            score: 0.9,
            start,
            end: start + text.len(),
            evidence: text.to_string(),
        }
    }

    #[test]
    fn role_requires_person_and_keeps_exact_utf8_evidence() {
        let source = "Mario Rossi, CEO di Acme.";
        let entities = vec![
            entity("Mario Rossi", "PERSON", 0),
            entity("Acme", "ORG", 20),
        ];
        let roles = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["person_1".into(), "org_1".into()],
            &[],
        );
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role_code, "chief_executive");
        assert_eq!(roles[0].person_id, "person_1");
        assert_eq!(roles[0].organization_id.as_deref(), Some("org_1"));
        assert_eq!(&source[roles[0].start_byte..roles[0].end_byte], "CEO");
        assert!(roles[0].evidence.starts_with("Mario Rossi"));
    }

    #[test]
    fn generic_title_without_grounded_person_is_not_emitted() {
        let source = "The CEO signed the agreement.";
        let entities = vec![];
        assert!(extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &[],
            &[],
        )
        .is_empty());
    }

    #[test]
    fn non_ascii_prefix_keeps_role_offsets_byte_exact() {
        let source = "Élodie, CEO di Acme.";
        let entities = vec![entity("Élodie", "PERSON", 0), entity("Acme", "ORG", 16)];
        let roles = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["person_1".into(), "org_1".into()],
            &[],
        );
        assert_eq!(roles.len(), 1);
        assert_eq!((roles[0].start_byte, roles[0].end_byte), (9, 12));
        assert_eq!(&source[roles[0].start_byte..roles[0].end_byte], "CEO");
    }

    #[test]
    fn italian_former_role_uses_feminine_surface_and_former_rank() {
        let source = "Anna Bianchi, ex ministra.";
        let entities = vec![entity("Anna Bianchi", "PERSON", 0)];
        let roles = extract(
            source,
            "it",
            &semantic_core::split_sentences(source, "it", None),
            &entities,
            &["person_1".into()],
            &[],
        );
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role_code, "minister");
        assert_eq!(roles[0].role_surface, "ministra");
        assert_eq!(roles[0].rank, RoleRank::Former);
        assert_eq!(roles[0].tense, RoleTense::Former);
    }

    #[test]
    fn spanish_feminine_role_is_grounded_and_keeps_source_surface() {
        let source = "María García, alcaldesa de Madrid, habló.";
        let start = source.find("María García").unwrap();
        let entities = vec![entity("María García", "PERSON", start)];
        let roles = extract(
            source,
            "es",
            &semantic_core::split_sentences(source, "es", None),
            &entities,
            &["person_1".into()],
            &[],
        );
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role_code, "mayor");
        assert_eq!(roles[0].role_surface, "alcaldesa");
        assert_eq!(roles[0].gender, "feminine");
        assert_eq!(&source[roles[0].start_byte..roles[0].end_byte], "alcaldesa");
    }

    #[test]
    fn localized_role_lexicons_are_loaded_for_all_remaining_languages() {
        let examples = [
            ("de", "Maya Chen, Bürgermeister.", "Bürgermeister", "mayor"),
            ("fr", "Maya Chen, maire.", "maire", "mayor"),
            ("id", "Maya Chen, dokter.", "dokter", "doctor"),
            ("nl", "Maya Chen, burgemeester.", "burgemeester", "mayor"),
            ("pl", "Maya Chen, prezes.", "prezes", "president"),
            ("pt", "Maya Chen, prefeito.", "prefeito", "mayor"),
            ("ru", "Maya Chen, мэр.", "мэр", "mayor"),
            ("tr", "Maya Chen, mühendis.", "mühendis", "engineer"),
        ];
        for (language, source, surface, role_code) in examples {
            let person_start = source.find("Maya Chen").unwrap();
            let entities = vec![entity("Maya Chen", "PERSON", person_start)];
            let roles = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &entities,
                &["person_1".into()],
                &[],
            );
            assert_eq!(roles.len(), 1, "{language}: {source}; roles={roles:?}");
            assert_eq!(roles[0].role_code, role_code, "{language}");
            assert_eq!(roles[0].role_surface, surface, "{language}");
            assert_eq!(
                &source[roles[0].start_byte..roles[0].end_byte],
                surface,
                "{language} byte span"
            );
        }
    }

    #[test]
    fn localized_gendered_role_surfaces_preserve_gender_without_guessing_turkish() {
        let examples = [
            (
                "de",
                "Maya Chen, Bürgermeisterin.",
                "Bürgermeisterin",
                "feminine",
            ),
            ("pl", "Maya Chen, ministra.", "ministra", "feminine"),
            ("ru", "Maya Chen, певица.", "певица", "feminine"),
            ("tr", "Maya Chen, başkan.", "başkan", "unknown"),
        ];
        for (language, source, surface, gender) in examples {
            let person_start = source.find("Maya Chen").unwrap();
            let entities = vec![entity("Maya Chen", "PERSON", person_start)];
            let roles = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &entities,
                &["person_1".into()],
                &[],
            );
            assert_eq!(roles.len(), 1, "{language}: {source}; roles={roles:?}");
            assert_eq!(roles[0].gender, gender, "{language}");
            assert_eq!(roles[0].role_surface, surface, "{language}");
        }
    }

    #[test]
    fn slavic_german_and_turkish_role_case_forms_keep_exact_spans() {
        let examples = [
            ("de", "Maya Chen, Präsidenten.", "Präsidenten", "president"),
            ("pl", "Maya Chen, prezesem.", "prezesem", "president"),
            ("ru", "Maya Chen, президентом.", "президентом", "president"),
            ("tr", "Maya Chen, başkana.", "başkana", "president"),
        ];
        for (language, source, surface, role_code) in examples {
            let person_start = source.find("Maya Chen").unwrap();
            let entities = vec![entity("Maya Chen", "PERSON", person_start)];
            let roles = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &entities,
                &["person_1".into()],
                &[],
            );
            assert_eq!(roles.len(), 1, "{language}: {source}; roles={roles:?}");
            assert_eq!(roles[0].role_code, role_code, "{language}");
            assert_eq!(&source[roles[0].start_byte..roles[0].end_byte], surface);
        }
    }

    #[test]
    fn role_binds_only_an_explicit_date_in_its_sentence() {
        let source = "Anna Bianchi was CEO in 2018.";
        let entities = vec![entity("Anna Bianchi", "PERSON", 0)];
        let dates = semantic_core::temporal_facts(source, "en");
        let roles = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &entities,
            &["person_1".into()],
            &dates,
        );
        assert_eq!(roles.len(), 1);
        assert_eq!(
            roles[0]
                .temporal
                .as_ref()
                .and_then(|d| d.normalized.as_deref()),
            Some("2018")
        );
    }
}
