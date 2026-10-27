use super::*;

/// Recognizes a named facility only after an explicit facility noun and when
/// the following name is title-cased. The emitted evidence is still an exact
/// source span and remains subject to the scene safety cap.
pub(crate) fn collect_facilities(source: &str, remaining: usize) -> Vec<VisualEntity> {
    if remaining == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for noun in &lexicon::lexicon().facility.facility_nouns {
        let lower_source = source.to_lowercase();
        let lower_noun = noun.to_lowercase();
        let cue = format!("{lower_noun} ");
        for (cue_start, _) in lower_source.match_indices(&cue) {
            if out.len() >= remaining {
                return out;
            }
            let name_start = cue_start + cue.len();
            let tail = &source[name_start..];
            let mut name_end = name_start;
            let mut cursor = 0usize;
            for token in tail.split_whitespace().take(3) {
                let Some(relative) = tail[cursor..].find(token) else {
                    break;
                };
                let token_start = name_start + cursor + relative;
                // "Aurora. La" is two sentences, not one facility name: a
                // token that carries sentence punctuation closes the name.
                let trimmed = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-');
                let trim_start = token.find(trimmed).unwrap_or(0);
                let core_len = trimmed.len();
                let core = trimmed;
                let Some(first) = core.chars().next() else {
                    break;
                };
                if !first.is_uppercase() {
                    break;
                }
                name_end = token_start + trim_start + core_len;
                cursor += relative + token.len();
                if core_len != token.len() {
                    break;
                }
                if source[name_end..].starts_with(" di ") || source[name_end..].starts_with(" of ")
                {
                    break;
                }
            }
            if name_end == name_start {
                continue;
            }
            let evidence = source[cue_start..name_end].to_string();
            if out
                .iter()
                .any(|entity: &VisualEntity| entity.start == cue_start && entity.end == name_end)
            {
                continue;
            }
            out.push(VisualEntity {
                text: evidence.clone(),
                r#type: "FACILITY".to_string(),
                score: 0.9,
                start: cue_start,
                end: name_end,
                evidence,
            });
        }
        // Some languages conventionally place the proper name before the
        // facility head ("Aurora Stadium"). Recover the nearest one or two
        // title-cased words in the same sentence and keep the whole span.
        for (noun_start, _) in lower_source.match_indices(&lower_noun) {
            let noun_end = noun_start + noun.len();
            let after = source.get(noun_end..).and_then(|tail| tail.chars().next());
            if after.is_some_and(char::is_alphanumeric) {
                continue;
            }
            let sentence_start = source[..noun_start]
                .rfind(['.', '!', '?', '\n'])
                .map_or(0, |position| position + 1);
            let prefix = &source[sentence_start..noun_start];
            let words: Vec<_> = prefix.split_whitespace().collect();
            let mut name_start = None;
            let candidate_words: Vec<_> = words.iter().rev().take(2).rev().copied().collect();
            let articles = [
                "a",
                "an",
                "the",
                "lo",
                "la",
                "il",
                "i",
                "gli",
                "le",
                "l'",
                "el",
                "los",
                "las",
                "un",
                "una",
                "o",
                "os",
                "as",
                "um",
                "uma",
                "les",
                "der",
                "die",
                "das",
                "den",
                "dem",
                "het",
                "een",
                "de",
                "do",
                "da",
                "dos",
                "этот",
                "эта",
                "это",
                "этого",
                "этой",
                "в",
                "на",
                "yang",
                "sebuah",
                "suatu",
            ];
            // A title-cased article is not a proper name. Also leave head-first
            // constructions to the prefix recognizer when a name follows.
            if candidate_words.len() == 1
                && articles.contains(
                    &candidate_words[0]
                        .trim_matches(|ch: char| !ch.is_alphanumeric())
                        .to_lowercase()
                        .as_str(),
                )
            {
                continue;
            }
            let next_word = source[noun_end..].split_whitespace().next().unwrap_or("");
            let next_name = next_word
                .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-')
                .chars()
                .next()
                .is_some_and(char::is_uppercase);
            if next_name {
                continue;
            }
            // Walk outward from the head: the name is the contiguous run of
            // title-cased words directly before it. A lowercase modifier
            // ("kommunales Stadion") ends the run, so an earlier capitalized
            // word ("Thema") is never pulled into the facility name.
            for word in candidate_words.iter().rev() {
                let cleaned = word.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-');
                if cleaned.chars().next().is_some_and(char::is_uppercase) {
                    let start = source[..noun_start].rfind(word).unwrap_or(noun_start);
                    name_start = Some(name_start.map_or(start, |prior: usize| start.min(prior)));
                } else {
                    break;
                }
            }
            let Some(name_start) = name_start else {
                continue;
            };
            if out
                .iter()
                .any(|entity| entity.start == name_start && entity.end == noun_end)
            {
                continue;
            }
            let evidence = source[name_start..noun_end].to_string();
            out.push(VisualEntity {
                text: evidence.clone(),
                r#type: "FACILITY".to_string(),
                score: 0.9,
                start: name_start,
                end: noun_end,
                evidence,
            });
        }
    }
    out.sort_by_key(|entity| entity.start);
    out
}

/// Recognize a family name only after an explicit ownership predicate.
pub(crate) fn collect_explicit_owner_entities(
    source: &str,
    entities: &mut Vec<VisualEntity>,
    safety_cap: usize,
) {
    if entities.len() >= safety_cap {
        return;
    }
    let lower = source.to_lowercase();
    for marker in ["di proprietà della famiglia ", "owned by the "] {
        let mut search_from = 0;
        while let Some(relative) = lower[search_from..].find(marker) {
            let start = search_from + relative + marker.len();
            let tail = &source[start..];
            let Some(token) = tail.split_whitespace().next() else {
                break;
            };
            let name = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-');
            if name.chars().next().is_some_and(char::is_uppercase) && !name.is_empty() {
                let end = start + name.len();
                if let Some(existing) = entities
                    .iter_mut()
                    .find(|entity| entity.start == start && entity.end == end)
                {
                    existing.r#type = "ORG".to_string();
                } else {
                    entities.push(VisualEntity {
                        text: name.to_string(),
                        r#type: "ORG".to_string(),
                        score: 0.8,
                        start,
                        end,
                        evidence: name.to_string(),
                    });
                }
            }
            search_from = start;
            if search_from >= lower.len() || entities.len() >= safety_cap {
                return;
            }
        }
    }
}

/// Attach facility facts only when explicit textual predicates identify the
/// date, capacity, ownership, or location relation in the same sentence.
pub(crate) fn extract_entity_facts(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    numeric_facts: &[semantic_core::NumericFact],
    temporal_facts: &[TemporalFact],
    identities: &IdentityLookup,
) -> Vec<EntityFact> {
    let facility_indices: Vec<usize> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.r#type == "FACILITY")
        .map(|(index, _)| index)
        .collect();
    if facility_indices.is_empty() {
        return Vec::new();
    }
    // A scene that names a single facility can keep attributing facts to it
    // when a later sentence uses an anaphor ("l'impianto", "la struttura")
    // instead of repeating the name. Two distinct facilities make that
    // attribution ambiguous, so facts are attached only to a facility named in
    // the same sentence.
    let facility_ids: BTreeSet<String> = facility_indices
        .iter()
        .map(|&index| identities.id_of(index))
        .collect();
    let unique_facility = (facility_ids.len() == 1).then(|| facility_indices[0]);
    let mut out = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let facility_in_sentence = facility_indices.iter().copied().find(|&index| {
            entities[index].start >= sentence.start_byte && entities[index].end <= sentence.end_byte
        });
        let Some(entity_index) = facility_in_sentence.or(unique_facility) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let normalized_sentence = evidence.to_lowercase();
        let subject_entity_id = identities.id_of(entity_index);
        for fact in temporal_facts {
            if fact.start_byte < sentence.start_byte || fact.end_byte > sentence.end_byte {
                continue;
            }
            if !contains_any(
                &normalized_sentence,
                &lexicon::lexicon().facility.inauguration_markers,
            ) {
                continue;
            }
            out.push(EntityFact {
                id: format!("{}_inaugurated_{:02}", subject_entity_id, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: "inaugurated_year".to_string(),
                value: fact.surface.clone(),
                value_type: "DATE".to_string(),
                normalized_value: fact.normalized.clone(),
                normalized_date: fact.normalized.clone(),
                target_entity_id: None,
                unit: None,
                operator: None,
                status: fact_status(&normalized_sentence),
                sentence_index,
                evidence: evidence.to_string(),
                start_byte: fact.start_byte,
                end_byte: fact.end_byte,
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                importance: 0.0,
                visual_concept_ids: Vec::new(),
            });
        }
        for fact in numeric_facts {
            if fact.start_byte < sentence.start_byte
                || fact.end_byte > sentence.end_byte
                || temporal_facts.iter().any(|date| {
                    date.start_byte == fact.start_byte && date.end_byte == fact.end_byte
                })
            {
                continue;
            }
            // Overlapping numeric spans ("oltre 10.000 posti" and "10.000")
            // describe ONE value: keep the widest grounded surface so a
            // capacity fact is never emitted twice with different text.
            if numeric_facts.iter().any(|other| {
                other.start_byte >= sentence.start_byte
                    && other.end_byte <= sentence.end_byte
                    && other.start_byte <= fact.start_byte
                    && other.end_byte >= fact.end_byte
                    && (other.end_byte - other.start_byte) > (fact.end_byte - fact.start_byte)
            }) {
                continue;
            }
            let facility = &lexicon::lexicon().facility;
            if !contains_any(&normalized_sentence, &facility.capacity_markers) {
                continue;
            }
            let unit = if contains_any(&normalized_sentence, &facility.capacity_unit_markers) {
                Some("seats")
            } else {
                None
            };
            let operator = facility
                .operators
                .iter()
                .find(|operator| contains_any(&normalized_sentence, &operator.markers))
                .map(|operator| operator.symbol.as_str());
            out.push(EntityFact {
                id: format!("{}_capacity_{:02}", subject_entity_id, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: "capacity".to_string(),
                value: fact.surface.clone(),
                value_type: fact.kind.clone(),
                normalized_value: fact.value.map(|value| value.to_string()),
                normalized_date: None,
                target_entity_id: None,
                unit: unit.map(str::to_string),
                operator: operator.map(str::to_string),
                status: fact_status(&normalized_sentence),
                sentence_index,
                evidence: evidence.to_string(),
                start_byte: fact.start_byte,
                end_byte: fact.end_byte,
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                importance: 0.0,
                visual_concept_ids: Vec::new(),
            });
        }
        for (marker, predicate, target_kind) in [
            ("di proprietà", "owned_by", "owner"),
            ("owned by", "owned_by", "owner"),
        ] {
            let Some(marker_offset) = normalized_sentence.find(marker) else {
                continue;
            };
            let target_after = sentence.start_byte + marker_offset + marker.len();
            let target = entities
                .iter()
                .enumerate()
                .find(|(candidate_index, candidate)| {
                    *candidate_index != entity_index
                        && candidate.start >= target_after
                        && candidate.end <= sentence.end_byte
                        && candidate.r#type != "FACILITY"
                        && if target_kind == "LOCATION" {
                            candidate.r#type == "LOCATION"
                        } else {
                            matches!(candidate.r#type.as_str(), "PERSON" | "ORG" | "ORGANIZATION")
                        }
                });
            let Some((target_index, target)) = target else {
                continue;
            };
            let target_entity_id = identities.id_of(target_index);
            out.push(EntityFact {
                id: format!("{}_{}_{:02}", subject_entity_id, predicate, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: predicate.to_string(),
                value: target.text.clone(),
                value_type: "ENTITY".to_string(),
                normalized_value: None,
                normalized_date: None,
                target_entity_id: Some(target_entity_id),
                unit: None,
                operator: None,
                status: relation_fact_status(&normalized_sentence, marker_offset),
                sentence_index,
                evidence: evidence.to_string(),
                start_byte: target.start,
                end_byte: target.end,
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                importance: 0.0,
                visual_concept_ids: Vec::new(),
            });
        }
        for marker in &lexicon::lexicon().facility.location_relation_markers {
            let Some(marker_offset) = normalized_sentence.find(marker) else {
                continue;
            };
            let target_after = sentence.start_byte + marker_offset + marker.len();
            let local_target = entities
                .iter()
                .enumerate()
                .find(|(candidate_index, candidate)| {
                    *candidate_index != entity_index
                        && candidate.start >= target_after
                        && candidate.end <= sentence.end_byte
                        && candidate.r#type == "LOCATION"
                });
            let target_span = local_target
                .map(|(target_index, target)| {
                    (target_index, target.text.clone(), target.start, target.end)
                })
                .or_else(|| {
                    // Entity extraction may keep only the first mention of a
                    // canonical place. After an explicit locative predicate, a
                    // later exact spelling of that already typed place is still
                    // grounded evidence and can supply this sentence's span.
                    let tail_start = target_after.max(sentence.start_byte);
                    let tail = source.get(tail_start..sentence.end_byte)?;
                    entities.iter().enumerate().find_map(|(index, candidate)| {
                        if candidate.r#type != "LOCATION" {
                            return None;
                        }
                        let relative = tail.find(&candidate.text)?;
                        let start = tail_start + relative;
                        Some((
                            index,
                            candidate.text.clone(),
                            start,
                            start + candidate.text.len(),
                        ))
                    })
                });
            let Some((target_index, target_text, target_start, target_end)) = target_span else {
                continue;
            };
            let target_entity_id = identities.id_of(target_index);
            out.push(EntityFact {
                id: format!("{}_located_in_{:02}", subject_entity_id, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: "located_in".to_string(),
                value: target_text,
                value_type: "ENTITY".to_string(),
                normalized_value: None,
                normalized_date: None,
                target_entity_id: Some(target_entity_id),
                unit: None,
                operator: None,
                status: relation_fact_status(&normalized_sentence, marker_offset),
                sentence_index,
                evidence: evidence.to_string(),
                start_byte: target_start,
                end_byte: target_end,
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                importance: 0.0,
                visual_concept_ids: Vec::new(),
            });
            break;
        }
        // "lo stadio di Torino": a bare prepositional phrase is enough when
        // the subject is the scene's facility and the target is a grounded
        // LOCATION mention. An explicit marker above wins over this fallback.
        let has_location = out.iter().any(|fact| {
            fact.subject_entity_id == subject_entity_id && fact.predicate == "located_in"
        });
        if !has_location {
            let target = entities.iter().enumerate().find(|(index, candidate)| {
                *index != entity_index
                    && candidate.r#type == "LOCATION"
                        // A city embedded in another proper name earlier in
                        // the sentence ("Comune di Torino ... Stadio Aurora")
                        // is not evidence that the facility itself is there.
                        && candidate.start >= entities[entity_index].end
                    && candidate.end <= sentence.end_byte
                    && location_cue_precedes(
                        source,
                        sentence.start_byte,
                        candidate.start,
                        &lexicon::lexicon().facility.bare_location_prepositions,
                    )
            });
            if let Some((target_index, target)) = target {
                out.push(EntityFact {
                    id: format!(
                        "{}_{}_{:02}",
                        subject_entity_id,
                        "located_in",
                        out.len() + 1
                    ),
                    subject_entity_id: subject_entity_id.clone(),
                    predicate: "located_in".to_string(),
                    value: target.text.clone(),
                    value_type: "ENTITY".to_string(),
                    normalized_value: None,
                    normalized_date: None,
                    target_entity_id: Some(identities.id_of(target_index)),
                    unit: None,
                    operator: None,
                    status: fact_status(&normalized_sentence),
                    sentence_index,
                    evidence: evidence.to_string(),
                    start_byte: target.start,
                    end_byte: target.end,
                    evidence_start_byte: sentence.start_byte,
                    evidence_end_byte: sentence.end_byte,
                    importance: 0.0,
                    visual_concept_ids: Vec::new(),
                });
            }
        }
    }
    out
}

/// True when the word immediately before `target_start` is a locative
/// preposition ("di Torino", "a Roma", "in Lombardia").
pub(crate) fn location_cue_precedes(
    source: &str,
    sentence_start: usize,
    target_start: usize,
    accepted_cues: &[String],
) -> bool {
    let prefix = source
        .get(sentence_start..target_start)
        .unwrap_or_default()
        .trim_end();
    let word = prefix.split_whitespace().next_back().unwrap_or_default();
    accepted_cues.contains(&word.to_lowercase())
}

/// A bare surname the script already introduced belongs to that person.
///
/// Identity stays surface-based: this only fixes the *type* of the mention, so
/// "Stasi" is a PERSON rather than an untitled visual concept. It is limited to
/// the exact surname token of an unambiguous two-word name the script states
/// and never overrides a place, an institution or a typed entity.
pub(crate) fn promote_known_surnames(
    source: &str,
    entities: &mut Vec<visualner::VisualEntity>,
    context: &visualner::ScriptContext,
    known_person_surnames: &HashSet<String>,
    language: &str,
) {
    if context.person_surnames.is_empty() || known_person_surnames.is_empty() {
        return;
    }
    let lexicon = semantic_core::lexicon::lexicon();
    for entity in entities.iter_mut() {
        if entity.r#type != "VISUAL_CONCEPT" {
            continue;
        }
        let lower = entity.text.to_lowercase();
        if lower.split_whitespace().count() != 1
            || lower.chars().filter(|ch| ch.is_alphabetic()).count() < 4
            || !context.person_surnames.contains_key(&lower)
            || !known_person_surnames.contains(&lower)
        {
            continue;
        }
        if lexicon.is_place(&entity.text, language)
            || lexicon
                .gazetteer
                .generic_visual_nouns
                .iter()
                .any(|noun| noun == &lower)
        {
            continue;
        }
        entity.r#type = "PERSON".to_string();
        entity.score = entity.score.max(0.9);
    }

    // Some extractors omit a one-token surname mention entirely. Add only
    // exact, standalone surname tokens that resolve to one full name already
    // introduced in the script. This gives fact attribution a grounded span
    // in the sentence and avoids falling back to a person from a prior
    // sentence (which misread glossary definitions as assertions).
    let mut cursor = 0usize;
    while cursor < source.len() {
        let Some((_, ch)) = source[cursor..].char_indices().next() else {
            break;
        };
        if !ch.is_alphanumeric() {
            cursor += ch.len_utf8();
            continue;
        }
        let start = cursor;
        let mut end = start + ch.len_utf8();
        while let Some(next) = source[end..].chars().next() {
            if !next.is_alphanumeric() {
                break;
            }
            end += next.len_utf8();
        }
        let surface = &source[start..end];
        let lower = surface.to_lowercase();
        if !context.person_surnames.contains_key(&lower) || !known_person_surnames.contains(&lower)
        {
            cursor = end;
            continue;
        }
        if lexicon.is_place(surface, language)
            || lexicon
                .gazetteer
                .generic_visual_nouns
                .iter()
                .any(|noun| noun == &lower)
        {
            cursor = end;
            continue;
        }
        if entities
            .iter()
            .any(|entity| entity.start < end && start < entity.end)
        {
            cursor = end;
            continue;
        }
        entities.push(visualner::VisualEntity {
            text: surface.to_string(),
            r#type: "PERSON".to_string(),
            score: 0.9,
            start,
            end,
            evidence: surface.to_string(),
        });
        cursor = end;
    }
}

/// Substring marker check shared by every fact extractor in this crate.
pub(crate) fn contains_any<S: AsRef<str>>(text: &str, markers: &[S]) -> bool {
    markers.iter().any(|marker| text.contains(marker.as_ref()))
}

pub(crate) fn fact_status(sentence: &str) -> String {
    fact_status_lower(sentence).to_string()
}

/// Same classification, borrowing the status instead of allocating a `String`
/// for every fact. The caller supplies the already case-folded sentence, so a
/// loop that visits several facts in one sentence folds and classifies it once.
pub(crate) fn fact_status_lower(sentence: &str) -> &'static str {
    let facility = &lexicon::lexicon().facility;
    if contains_any(sentence, &facility.status_uncertainty_cues) {
        "uncertain"
    } else if contains_any(sentence, &facility.status_negation_substrings) {
        "negated"
    } else {
        "asserted"
    }
}

pub(crate) fn relation_fact_status(sentence: &str, marker_offset: usize) -> String {
    let prefix = &sentence[..marker_offset];
    fact_status(&sentence[semantic_core::relations::clause_start(prefix)..])
}
