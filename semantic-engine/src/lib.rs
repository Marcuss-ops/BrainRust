//! Unified Rust semantic engine (`semantic_manifest.v2`).
//!
//! One request carries every scene of a script. Each scene is analysed once by
//! `visualner` (entities, relations, bullets, numeric and temporal facts), then
//! projected into typed visual *proposals*. Rust decides what the text contains
//! and what could be represented; it never downloads assets, never renders, and
//! never admits a candidate that is not grounded in the source.
//!
//! Shared analysis primitives (numeric parsing, date normalization, identity
//! registry, grounding, the manifest contract) live in `semantic-core` so every
//! extractor agrees on them. Killer rule: NO EVIDENCE -> NO CANDIDATE.

#![deny(unsafe_code)]

pub mod cache;
pub mod groups;
pub mod identity_lookup;
pub mod protocol;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use semantic_core::grounding::check_span;
use semantic_core::grounding::Grounding;
use semantic_core::{
    AssertedRelation, Candidate, CoMention, EntityFact, IdentityRegistry, RelationStatus,
    RelationView, SceneManifest, SemanticGroup, SemanticGroupMember, SemanticManifest,
    VisualProposal,
};
use visualner::analysis::Bullet;
use visualner::{analyze, ExtractOptions, TemporalFact, VisualEntity};

use groups::VisualCandidate;
use identity_lookup::IdentityLookup;

/// Wire version of the manifest produced by this engine.
pub use semantic_core::SCHEMA_VERSION;
/// Re-exported contract types so callers (and tests) build the same request
/// shape the worker deserializes.
pub use semantic_core::{SceneInput, ScriptRequest};

/// Entity safety cap used when a request does not set one. This bounds
/// pathological inputs only: it is NOT an editorial budget. The real top-N is
/// chosen by the Go planner, so a scene with eight people keeps all eight here.
pub const DEFAULT_ENTITY_SAFETY_CAP: usize = 10_000;

/// Analyzer versions recorded on every manifest for replay provenance. The
/// cache keys on these so a version bump never serves a stale result.
pub fn analyzer_versions() -> semantic_core::AnalyzerVersions {
    semantic_core::AnalyzerVersions {
        visualner: "visualner".to_string(),
        semantic_core: env!("CARGO_PKG_VERSION").to_string(),
        phrase_impact: "phrase_impact".to_string(),
    }
}

/// Analyses one scene and records its identities in the shared registry.
/// Extractor refusals become a `rejected` scene rather than failing the script.
pub fn analyze_scene(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
) -> SceneManifest {
    let options = ExtractOptions {
        language: language.to_string(),
        // The extractor keeps every grounded entity up to the safety cap; the
        // editorial top-N belongs to the planner.
        entity_count: safety_cap,
    };
    let mut analysis = match analyze(&scene.text, &options) {
        Ok(analysis) => analysis,
        Err(reason) => {
            return SceneManifest {
                scene_id: scene.scene_id.clone(),
                status: "rejected",
                reason: Some(reason),
                entities: Vec::new(),
                identities: Vec::new(),
                relations: RelationView::default(),
                numeric_facts: Vec::new(),
                temporal_facts: Vec::new(),
                entity_facts: Vec::new(),
                bullets: Vec::new(),
                short_phrases: Vec::new(),
                long_phrases: Vec::new(),
                quotes: Vec::new(),
                important_words: Vec::new(),
                visual_proposals: Vec::new(),
                semantic_groups: Vec::new(),
                rejected_items: 0,
            };
        }
    };

    let source = scene.text.as_str();
    let facility_entities =
        collect_facilities(source, safety_cap.saturating_sub(analysis.entities.len()));
    for facility in facility_entities {
        if let Some(existing) = analysis
            .entities
            .iter_mut()
            .find(|entity| entity.start < facility.end && facility.start < entity.end)
        {
            existing.r#type = "FACILITY".to_string();
            existing.score = existing.score.max(facility.score);
        } else {
            analysis.entities.push(facility);
        }
    }
    collect_explicit_owner_entities(source, &mut analysis.entities, safety_cap);
    let (entities, rejected_entities) = retain_grounded_entities(source, analysis.entities);
    let temporal_facts = retain_grounded_temporal(source, analysis.temporal_facts);

    let identities = IdentityLookup::build(registry, &scene.scene_id, &entities);
    let entity_facts = extract_entity_facts(
        source,
        language,
        &entities,
        &analysis.numeric_facts,
        &temporal_facts,
        &identities,
    );
    let relations = build_relations(
        source,
        language,
        &entities,
        &analysis.relations,
        &entity_facts,
        &identities,
    );
    let bullets = bullet_candidates(source, &analysis.bullets);
    let candidates = groups::build_candidates(
        &entities,
        &analysis.bullets,
        &analysis.chart_points,
        &analysis.lower_thirds,
        &temporal_facts,
        &identities,
    );
    let (visual_candidates, rejected_candidates) = retain_grounded_candidates(source, candidates);
    let sentence_spans = semantic_core::split_sentences(source, language, None);
    let semantic_groups = {
        let mut groups = chronological_groups(&scene.scene_id, source, language, &temporal_facts);
        groups.extend(process_groups(
            &scene.scene_id,
            source,
            language,
            &entities,
            &identities,
        ));
        groups.extend(comparison_groups(
            &scene.scene_id,
            source,
            language,
            &entities,
            &identities,
        ));
        groups.extend(causal_groups(
            &scene.scene_id,
            source,
            language,
            &entities,
            &identities,
            &relations,
        ));
        groups.extend(numbered_groups(&scene.scene_id, source, language));
        groups
    };

    SceneManifest {
        scene_id: scene.scene_id.clone(),
        status: "valid",
        reason: None,
        entities: entity_candidates(source, language, &entities, &identities),
        identities: registry.identities().to_vec(),
        relations,
        numeric_facts: analysis.numeric_facts,
        temporal_facts,
        entity_facts,
        important_words: important_word_candidates(source, language, &entities, &identities),
        bullets,
        short_phrases: phrase_candidates(source, language, 0, 18),
        long_phrases: phrase_candidates(source, language, 18, usize::MAX),
        quotes: quote_candidates(source, language),
        visual_proposals: visual_candidates
            .into_iter()
            .map(|candidate| to_visual_proposal(candidate, source, &sentence_spans))
            .collect(),
        semantic_groups,
        rejected_items: rejected_entities + rejected_candidates,
    }
}

/// Analyses every scene of a script in one call.
pub fn analyze_script(request: &ScriptRequest) -> Result<SemanticManifest, String> {
    let language = request.language.trim();
    if language.is_empty() {
        return Err("language is required".to_string());
    }
    if request.scenes.is_empty() {
        return Err("scenes must not be empty".to_string());
    }
    let mut ids = BTreeSet::new();
    for scene in &request.scenes {
        if !ids.insert(scene.scene_id.as_str()) {
            return Err(format!("duplicate scene_id: {}", scene.scene_id));
        }
    }

    let safety_cap = if request.entity_safety_cap == 0 {
        DEFAULT_ENTITY_SAFETY_CAP
    } else {
        request.entity_safety_cap
    };

    // One registry for the whole script so identity is shared across scenes.
    let mut registry = IdentityRegistry::new();
    let mut scenes: Vec<SceneManifest> = request
        .scenes
        .iter()
        .map(|scene| analyze_scene(scene, language, safety_cap, &mut registry))
        .collect();
    add_cross_scene_comparison_groups(&request.scenes, &mut scenes);
    add_cross_scene_causal_groups(&request.scenes, &mut scenes);
    add_cross_scene_process_groups(&request.scenes, &mut scenes, language);

    let analyzers = analyzer_versions();
    let fingerprint = request_fingerprint(request, &analyzers);
    let editorial = request
        .editorial
        .as_ref()
        .map(|input| {
            let mut editorial_request = input.request.clone();
            if editorial_request.semantic_bullet_spans.is_none() {
                if let Some(spans) =
                    semantic_bullet_spans(&editorial_request.transcript, &input.scenes, &scenes)
                {
                    editorial_request.semantic_bullet_spans = Some(spans);
                }
            }
            semantic_core::phrase_impact::run_with_scene_inputs(
                editorial_request,
                &input.scenes,
                &input.chapter_options,
            )
        })
        .transpose()?
        .map(Into::into);

    Ok(SemanticManifest {
        schema_version: SCHEMA_VERSION,
        language: language.to_string(),
        fingerprint,
        analyzers,
        scenes,
        editorial,
    })
}

/// Maps scene-local semantic bullet candidates to exact transcript spans.
/// Like the Go implementation before this cutover, the semantic candidate pool
/// is activated only when every scene has a verified global slice; otherwise
/// Phrase Impact retains its historical candidate behavior.
fn semantic_bullet_spans(
    transcript: &str,
    editorial_scenes: &[semantic_core::phrase_impact::SceneInput],
    manifests: &[SceneManifest],
) -> Option<Vec<semantic_core::phrase_impact::SemanticBulletSpan>> {
    use semantic_core::phrase_impact::SemanticBulletSpan;

    let by_id: BTreeMap<_, _> = manifests
        .iter()
        .map(|manifest| (manifest.scene_id.as_str(), manifest))
        .collect();
    for scene in editorial_scenes {
        let start = scene.start_byte?;
        let end = scene.end_byte?;
        if end <= start || transcript.get(start..end)? != scene.text {
            return None;
        }
    }
    let mut spans = Vec::new();
    for scene in editorial_scenes {
        let start = scene.start_byte?;
        let manifest = by_id.get(scene.scene_id.as_str())?;
        for candidate in &manifest.bullets {
            let Some(local) = scene.text.get(candidate.start_byte..candidate.end_byte) else {
                continue;
            };
            if local != candidate.text {
                continue;
            }
            let global_start = start + candidate.start_byte;
            let global_end = start + candidate.end_byte;
            if transcript.get(global_start..global_end) != Some(candidate.text.as_str()) {
                continue;
            }
            spans.push(SemanticBulletSpan {
                start_byte: global_start,
                end_byte: global_end,
                text: candidate.text.clone(),
            });
        }
    }
    Some(spans)
}

/// FNV-1a fingerprint over every input that can change a manifest, including
/// optional editorial options and caller-supplied embeddings.
pub fn request_fingerprint(
    request: &ScriptRequest,
    analyzers: &semantic_core::AnalyzerVersions,
) -> String {
    let body = serde_json::to_vec(&(
        request.language.trim(),
        &request.scenes,
        &request.editorial,
        request.entity_safety_cap,
        analyzers,
    ))
    .unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// FNV-1a 64 over the canonical request and analyzer versions. Equal inputs
/// give equal fingerprints (idempotent replay).
pub fn fingerprint(
    language: &str,
    scenes: &[SceneInput],
    analyzers: &semantic_core::AnalyzerVersions,
) -> String {
    let body = serde_json::to_vec(&(language, scenes, analyzers)).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Relations split into co-mentions (grouping signal) and asserted relations.
/// `visualner` only proves co-occurrence. Asserted edges are admitted only
/// from explicit ownership/location predicates already grounded as facts.
fn build_relations(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    relations: &[visualner::analysis::EntityRelation],
    facts: &[EntityFact],
    identities: &IdentityLookup,
) -> RelationView {
    let mut co_mentions = Vec::new();
    for relation in relations {
        // The pair must reference two grounded, distinct entities. Entity
        // indices are validated against the grounded list; the relation is
        // admitted only when its evidence actually occurs in the source.
        if relation.source >= entities.len() || relation.target >= entities.len() {
            continue;
        }
        // Measurements (numbers, percentages, money, dates) are not things
        // that co-occur as subjects: a spelled-out number such as "due" must
        // never pair with a person. Only visual subjects form a co-mention.
        if !is_visual_subject(&entities[relation.source])
            || !is_visual_subject(&entities[relation.target])
        {
            continue;
        }
        if relation.evidence.is_empty() || !source.contains(&relation.evidence) {
            continue;
        }
        co_mentions.push(CoMention {
            left_id: identities.id_of(relation.source),
            right_id: identities.id_of(relation.target),
            sentence_index: relation.sentence_index,
            evidence: relation.evidence.clone(),
        });
    }
    let mut asserted_relations: Vec<AssertedRelation> = facts
        .iter()
        .filter_map(|fact| {
            let target_id = fact.target_entity_id.as_ref()?;
            let predicate = match fact.predicate.as_str() {
                "owned_by" => "ownership",
                "located_in" => "location",
                _ => return None,
            };
            if fact.evidence.is_empty()
                || !source
                    .get(fact.evidence_start_byte..fact.evidence_end_byte)
                    .is_some_and(|evidence| evidence == fact.evidence)
            {
                return None;
            }
            let status = match fact.status.as_str() {
                "negated" => RelationStatus::Negated,
                "uncertain" => RelationStatus::Uncertain,
                _ => RelationStatus::Asserted,
            };
            Some(AssertedRelation {
                source_id: fact.subject_entity_id.clone(),
                target_id: target_id.clone(),
                predicate: Some(predicate.to_string()),
                status,
                sentence_index: fact.sentence_index,
                evidence: fact.evidence.clone(),
            })
        })
        .collect();
    let mut seen: HashSet<(String, String, String)> = asserted_relations
        .iter()
        .filter_map(|relation| {
            Some((
                relation.source_id.clone(),
                relation.target_id.clone(),
                relation.predicate.clone()?,
            ))
        })
        .collect();
    for (sentence_index, sentence) in semantic_core::split_sentences(source, language, None)
        .into_iter()
        .enumerate()
    {
        let present: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                is_visual_subject(entity)
                    && entity.start >= sentence.start_byte
                    && entity.end <= sentence.end_byte
            })
            .map(|(index, _)| index)
            .collect();
        if present.len() < 2 {
            continue;
        }
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let Some(marker) =
            semantic_core::relations::explicit_relation_marker(sentence_text, language)
        else {
            continue;
        };
        let (source_index, target_index) = if matches!(
            marker.predicate.as_str(),
            "ownership" | "funding" | "causal" | "temporal"
        ) {
            let before = present
                .iter()
                .copied()
                .rfind(|index| entities[*index].end <= sentence.start_byte + marker.start_byte);
            let after = present
                .iter()
                .copied()
                .find(|index| entities[*index].start >= sentence.start_byte + marker.start_byte);
            (
                before.unwrap_or(present[0]),
                after.unwrap_or(*present.last().unwrap()),
            )
        } else {
            (present[0], *present.last().unwrap())
        };
        if source_index == target_index {
            continue;
        }
        let source_id = identities.id_of(source_index);
        let target_id = identities.id_of(target_index);
        if !seen.insert((
            source_id.clone(),
            target_id.clone(),
            marker.predicate.clone(),
        )) {
            continue;
        }
        asserted_relations.push(AssertedRelation {
            source_id,
            target_id,
            predicate: Some(marker.predicate),
            status: marker.status,
            sentence_index,
            evidence: sentence_text.to_string(),
        });
    }
    RelationView {
        co_mentions,
        asserted_relations,
    }
}

fn entity_candidates(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<Candidate> {
    entities
        .iter()
        .enumerate()
        .map(|(index, entity)| Candidate {
            id: identities.id_of(index),
            text: entity.text.clone(),
            kind: entity.r#type.clone(),
            start_byte: entity.start,
            end_byte: entity.end,
            sentence_index: sentence_index_at(source, language, entity.start),
            evidence: entity.evidence.clone(),
            score: entity.score,
            provenance: "visualner".to_string(),
        })
        .filter(|candidate| {
            check_span(
                source,
                candidate.start_byte,
                candidate.end_byte,
                &candidate.evidence,
            ) == Grounding::Grounded
        })
        .collect()
}

fn bullet_candidates(source: &str, bullets: &[Bullet]) -> Vec<Candidate> {
    bullets
        .iter()
        .enumerate()
        .map(|(index, bullet)| Candidate {
            id: format!("bullet_{:02}", index + 1),
            text: bullet.text.clone(),
            kind: "SENTENCE".to_string(),
            start_byte: bullet.start,
            end_byte: bullet.end,
            sentence_index: bullet.sentence_index,
            evidence: bullet.text.clone(),
            score: 1.0,
            provenance: "visualner".to_string(),
        })
        .filter(|candidate| {
            check_span(
                source,
                candidate.start_byte,
                candidate.end_byte,
                &candidate.evidence,
            ) == Grounding::Grounded
        })
        .collect()
}

fn sentence_index_at(source: &str, language: &str, offset: usize) -> usize {
    semantic_core::split_sentences(source, language, None)
        .iter()
        .position(|sentence| offset >= sentence.start_byte && offset < sentence.end_byte)
        .unwrap_or(0)
}

fn phrase_candidates(
    source: &str,
    language: &str,
    min_words: usize,
    max_words: usize,
) -> Vec<Candidate> {
    semantic_core::split_sentences(source, language, None)
        .into_iter()
        .enumerate()
        .filter_map(|(sentence_index, sentence)| {
            let evidence = source.get(sentence.start_byte..sentence.end_byte)?;
            let word_count = evidence.split_whitespace().count();
            if word_count < min_words || word_count >= max_words {
                return None;
            }
            (check_span(source, sentence.start_byte, sentence.end_byte, evidence)
                == Grounding::Grounded)
                .then(|| Candidate {
                    id: format!("sentence_{:04}", sentence_index + 1),
                    text: evidence.to_string(),
                    kind: "PHRASE".to_string(),
                    start_byte: sentence.start_byte,
                    end_byte: sentence.end_byte,
                    sentence_index,
                    evidence: evidence.to_string(),
                    score: (0.5 + (word_count.min(30) as f32 / 60.0)).min(1.0),
                    provenance: "semantic_core_sentence_segmenter".to_string(),
                })
        })
        .collect()
}

/// Extracts complete, explicitly delimited quotations. The delimiters remain
/// part of the candidate so the text and offsets are an exact source slice.
/// Apostrophes and single quotes are deliberately excluded because they are
/// common orthographic characters and cannot safely delimit a quote alone.
fn quote_candidates(source: &str, language: &str) -> Vec<Candidate> {
    let delimiters = [("“", "”"), ("„", "“"), ("«", "»"), ("\"", "\"")];
    let mut spans = Vec::new();
    for (opening, closing) in delimiters {
        let mut cursor = 0;
        while let Some(relative_start) = source[cursor..].find(opening) {
            let start = cursor + relative_start;
            let content_start = start + opening.len();
            let Some(relative_end) = source[content_start..].find(closing) else {
                break;
            };
            let close_start = content_start + relative_end;
            let end = close_start + closing.len();
            if !source[content_start..close_start].trim().is_empty() {
                spans.push((start, end));
            }
            cursor = end;
        }
    }
    spans.sort_unstable();
    spans.dedup();
    spans
        .into_iter()
        .enumerate()
        .filter_map(|(index, (start, end))| {
            let evidence = source.get(start..end)?;
            (check_span(source, start, end, evidence) == Grounding::Grounded).then(|| Candidate {
                id: format!("quote_{:04}", index + 1),
                text: evidence.to_string(),
                kind: "QUOTE".to_string(),
                start_byte: start,
                end_byte: end,
                sentence_index: sentence_index_at(source, language, start),
                evidence: evidence.to_string(),
                score: 1.0,
                provenance: "semantic-core".to_string(),
            })
        })
        .collect()
}

fn important_word_candidates(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<Candidate> {
    entity_candidates(source, language, entities, identities)
        .into_iter()
        .filter(|candidate| {
            !matches!(
                candidate.kind.as_str(),
                "DATE" | "NUMBER" | "PERCENT" | "MONEY"
            )
        })
        .collect()
}

/// Recognizes a named facility only after an explicit facility noun and when
/// the following name is title-cased. The emitted evidence is still an exact
/// source span and remains subject to the scene safety cap.
fn collect_facilities(source: &str, remaining: usize) -> Vec<VisualEntity> {
    if remaining == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for cue in [
        "Stadio ", "stadio ", "Stadium ", "stadium ", "Arena ", "arena ",
    ] {
        for (cue_start, _) in source.match_indices(cue) {
            if out.len() >= remaining {
                return out;
            }
            let name_start = cue_start + cue.len();
            let tail = &source[name_start..];
            let mut name_end = name_start;
            for token in tail.split_whitespace().take(3) {
                let token_start = source[name_start..]
                    .find(token)
                    .map(|offset| name_start + offset)
                    .unwrap_or(name_start);
                let Some(first) = token.chars().next() else {
                    break;
                };
                if !first.is_uppercase() {
                    break;
                }
                let token_end = token_start + token.len();
                name_end = token_end;
                if source[token_end..].starts_with(" di ")
                    || source[token_end..].starts_with(" of ")
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
    }
    out.sort_by_key(|entity| entity.start);
    out
}

/// Recognize a family name only after an explicit ownership predicate.
fn collect_explicit_owner_entities(
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
fn extract_entity_facts(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    numeric_facts: &[semantic_core::NumericFact],
    temporal_facts: &[TemporalFact],
    identities: &IdentityLookup,
) -> Vec<EntityFact> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut out = Vec::new();
    for entity_index in 0..entities.len() {
        let entity = &entities[entity_index];
        if entity.r#type != "FACILITY" {
            continue;
        }
        let Some((sentence_index, sentence)) =
            sentences.iter().enumerate().find(|(_, sentence)| {
                entity.start >= sentence.start_byte && entity.end <= sentence.end_byte
            })
        else {
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
            if !contains_any(&normalized_sentence, &["inaugur", "opened", "inaugurated"]) {
                continue;
            }
            out.push(EntityFact {
                id: format!("{}_inaugurated_{:02}", subject_entity_id, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: "inaugurated_year".to_string(),
                value: fact.surface.clone(),
                value_type: "DATE".to_string(),
                normalized_value: fact.normalized.clone(),
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
            if !contains_any(
                &normalized_sentence,
                &["capienza", "posti", "capacity", "seats"],
            ) {
                continue;
            }
            let unit = if contains_any(&normalized_sentence, &["posti", "seats"]) {
                Some("seats")
            } else {
                None
            };
            let operator = if contains_any(
                &normalized_sentence,
                &["non superiore a", "not more than", "at most"],
            ) {
                Some("<=")
            } else if contains_any(
                &normalized_sentence,
                &["non inferiore a", "not less than", "at least"],
            ) {
                Some(">=")
            } else if contains_any(
                &normalized_sentence,
                &["oltre", "superiore a", "more than", "over"],
            ) {
                Some(">")
            } else if contains_any(
                &normalized_sentence,
                &["meno di", "inferiore a", "less than", "under"],
            ) {
                Some("<")
            } else {
                None
            };
            out.push(EntityFact {
                id: format!("{}_capacity_{:02}", subject_entity_id, out.len() + 1),
                subject_entity_id: subject_entity_id.clone(),
                predicate: "capacity".to_string(),
                value: fact.surface.clone(),
                value_type: fact.kind.clone(),
                normalized_value: fact.value.map(|value| value.to_string()),
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
            });
        }
        for (marker, predicate, target_kind) in [
            ("di proprietà", "owned_by", "owner"),
            ("owned by", "owned_by", "owner"),
            ("si trova a", "located_in", "LOCATION"),
            ("situato a", "located_in", "LOCATION"),
            ("located in", "located_in", "LOCATION"),
            ("based in", "located_in", "LOCATION"),
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
            });
        }
    }
    out
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn fact_status(sentence: &str) -> String {
    if contains_any(
        sentence,
        &["forse", "perhaps", "allegedly", "presumibilmente"],
    ) {
        "uncertain".to_string()
    } else if contains_any(sentence, &["non ", "not ", "never ", "mai "]) {
        "negated".to_string()
    } else {
        "asserted".to_string()
    }
}

fn relation_fact_status(sentence: &str, marker_offset: usize) -> String {
    let prefix = &sentence[..marker_offset];
    let clause_start = [" e ", " and ", " y ", " et ", " und ", ";"]
        .iter()
        .filter_map(|separator| {
            prefix
                .rfind(separator)
                .map(|offset| offset + separator.len())
        })
        .max()
        .unwrap_or(0);
    fact_status(&sentence[clause_start..])
}

/// Forms a chronology only when at least two dates are fully normalized and
/// each member can be tied to an exact source sentence. Unresolved dates are
/// retained as facts but cannot silently establish temporal order.
fn chronological_groups(
    scene_id: &str,
    source: &str,
    language: &str,
    facts: &[TemporalFact],
) -> Vec<SemanticGroup> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut members = Vec::new();
    for (fact_index, fact) in facts.iter().enumerate() {
        let Some(normalized) = fact.normalized.as_deref() else {
            continue;
        };
        let Some((sentence_index, sentence)) =
            sentences.iter().enumerate().find(|(_, sentence)| {
                fact.start_byte >= sentence.start_byte && fact.end_byte <= sentence.end_byte
            })
        else {
            continue;
        };
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if !is_event_sentence(sentence_text) {
            continue;
        }
        members.push((
            normalized.to_string(),
            SemanticGroupMember {
                candidate_id: format!("date_event_{:02}", fact_index + 1),
                scene_id: scene_id.to_string(),
                rank: None,
                sentence_index,
                text: sentence_text.to_string(),
                evidence: sentence_text.to_string(),
                start_byte: sentence.start_byte,
                end_byte: sentence.end_byte,
                temporal_surface: Some(fact.surface.clone()),
            },
        ));
    }
    if members.len() < 2 {
        return Vec::new();
    }
    members.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.sentence_index.cmp(&b.1.sentence_index))
    });
    vec![SemanticGroup {
        id: "chronology_01".to_string(),
        group_type: "chronological".to_string(),
        title: "Cronologia".to_string(),
        ranking_basis: None,
        target_count: None,
        complete: None,
        members: members.into_iter().map(|(_, member)| member).collect(),
    }]
}

/// Require an explicit event/process predicate before dates are grouped into
/// a visual timeline. Merely placing unrelated dates in a scene is not enough.
fn is_event_sentence(sentence: &str) -> bool {
    let sentence = sentence.to_lowercase();
    [
        "progett",
        "costru",
        "inaugur",
        "apert",
        "ampli",
        "ristruttur",
        "inizi",
        "termin",
        "fondat",
        "nasc",
        "morto",
        "vinto",
        "pers",
        "annull",
        "conferm",
        "condann",
        "assolt",
        "riapert",
        "pronunci",
        "decret",
        "built",
        "opened",
        "founded",
        "began",
        "started",
        "completed",
        "expanded",
        "renovated",
        "won",
        "lost",
        "born",
        "died",
    ]
    .iter()
    .any(|marker| sentence.contains(marker))
}

/// Links explicit phases only when the same grounded entity is named in each
/// source sentence, phase order is increasing, and every assertion is certain
/// and positive. This is a scene-local process group; cross-scene continuation
/// requires scene-addressed members and is intentionally a separate contract
/// change.
fn process_groups(
    scene_id: &str,
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<SemanticGroup> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut phases: BTreeMap<String, Vec<(usize, SemanticGroupMember, String)>> = BTreeMap::new();
    let mut invalid = BTreeSet::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if fact_status(&text.to_lowercase()) != "asserted" {
            continue;
        }
        let Some((phase, phase_id)) = process_phase(text) else {
            continue;
        };
        for (entity_index, entity) in entities.iter().enumerate() {
            if entity.start < sentence.start_byte || entity.end > sentence.end_byte {
                continue;
            }
            if matches!(
                entity.r#type.as_str(),
                "NUMBER" | "PERCENT" | "MONEY" | "DATE"
            ) {
                continue;
            }
            let identity_id = identities.id_of(entity_index);
            let members = phases.entry(identity_id.clone()).or_default();
            if let Some((previous, _, _)) = members.last() {
                if *previous == phase {
                    continue;
                }
                if *previous > phase {
                    invalid.insert(identity_id);
                    continue;
                }
            }
            members.push((
                phase,
                SemanticGroupMember {
                    candidate_id: format!("process_{}_{}", identity_id, phase_id),
                    scene_id: scene_id.to_string(),
                    rank: None,
                    sentence_index,
                    text: text.to_string(),
                    evidence: text.to_string(),
                    start_byte: sentence.start_byte,
                    end_byte: sentence.end_byte,
                    temporal_surface: None,
                },
                entity.text.clone(),
            ));
        }
    }

    phases
        .into_iter()
        .filter_map(|(identity_id, members)| {
            if invalid.contains(&identity_id) || members.len() < 3 {
                return None;
            }
            let title_entity = &members[0].2;
            Some(SemanticGroup {
                id: format!("process_{}", identity_id),
                group_type: "process".to_string(),
                title: format!("Processo: {title_entity}"),
                ranking_basis: Some("explicit_source_phase_order".to_string()),
                target_count: None,
                complete: None,
                members: members.into_iter().map(|(_, member, _)| member).collect(),
            })
        })
        .collect()
}

fn process_phase(sentence: &str) -> Option<(usize, &'static str)> {
    let sentence = sentence.to_lowercase();
    let phases: &[(usize, &str, &[&str])] = &[
        (
            1,
            "planning",
            &["progett", "pianificat", "planned", "designed", "conceived"],
        ),
        (
            2,
            "construction",
            &["costru", "building", "built", "constructed"],
        ),
        (3, "opening", &["inaugur", "opened", "opening"]),
        (
            4,
            "expansion",
            &["ampli", "expanded", "expansion", "renovated", "ristruttur"],
        ),
    ];
    phases.iter().find_map(|(phase, name, markers)| {
        markers
            .iter()
            .any(|marker| sentence.contains(marker))
            .then_some((*phase, *name))
    })
}

fn comparison_groups(
    scene_id: &str,
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<SemanticGroup> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut groups = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if fact_status(&text.to_lowercase()) != "asserted" || !has_explicit_comparison_cue(text) {
            continue;
        }
        let mut found = BTreeMap::<String, &VisualEntity>::new();
        for (entity_index, entity) in entities.iter().enumerate() {
            if entity.start < sentence.start_byte
                || entity.end > sentence.end_byte
                || !is_visual_subject(entity)
                || is_comparison_marker_surface(&entity.text)
            {
                continue;
            }
            found
                .entry(identities.id_of(entity_index))
                .or_insert(entity);
        }
        if found.len() < 2 {
            continue;
        }
        let mut members = Vec::new();
        for (identity_id, entity) in found {
            members.push(SemanticGroupMember {
                candidate_id: format!("comparison_{identity_id}"),
                scene_id: scene_id.to_string(),
                rank: None,
                sentence_index,
                text: entity.text.clone(),
                evidence: entity.evidence.clone(),
                start_byte: entity.start,
                end_byte: entity.end,
                temporal_surface: None,
            });
        }
        let title = format!(
            "Confronto: {}",
            members
                .iter()
                .map(|member| member.text.as_str())
                .collect::<Vec<_>>()
                .join(" vs ")
        );
        groups.push(SemanticGroup {
            id: format!("comparison_{}_{}", scene_id, groups.len() + 1),
            group_type: "comparison".to_string(),
            title,
            ranking_basis: Some("explicit_source_comparison".to_string()),
            target_count: None,
            complete: None,
            members,
        });
    }
    groups
}

fn has_explicit_comparison_cue(sentence: &str) -> bool {
    let folded = format!(" {} ", sentence.to_lowercase());
    [
        " versus ",
        " vs ",
        " contro ",
        " compared to ",
        " compared with ",
        " rispetto a ",
        " rispetto al ",
        " rispetto allo ",
        " rispetto all'",
        " rispetto all’",
        " rispetto alla ",
        " rispetto ai ",
        " rispetto agli ",
        " rispetto alle ",
        " gegenüber ",
        " gegenueber ",
        " par rapport à ",
    ]
    .iter()
    .any(|cue| folded.contains(cue))
}

fn is_comparison_marker_surface(surface: &str) -> bool {
    let normalized = surface
        .trim_matches(|character: char| {
            character.is_ascii_punctuation() || character.is_whitespace()
        })
        .to_lowercase();
    [
        "versus",
        "vs",
        "contro",
        "compared",
        "rispetto",
        "gegenüber",
        "gegenueber",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn causal_groups(
    scene_id: &str,
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    identities: &IdentityLookup,
    relations: &RelationView,
) -> Vec<SemanticGroup> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut groups = Vec::new();
    for relation in relations.asserted_relations.iter().filter(|relation| {
        relation.predicate.as_deref() == Some("causal")
            && relation.status == RelationStatus::Asserted
    }) {
        let Some(sentence) = sentences.get(relation.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let endpoints = [relation.source_id.as_str(), relation.target_id.as_str()];
        let mut members = Vec::new();
        for endpoint in endpoints {
            let Some((entity_index, entity)) =
                entities.iter().enumerate().find(|(index, entity)| {
                    identities.id_of(*index) == endpoint
                        && entity.start >= sentence.start_byte
                        && entity.end <= sentence.end_byte
                        && is_visual_subject(entity)
                })
            else {
                members.clear();
                break;
            };
            let role = if members.is_empty() {
                "cause"
            } else {
                "effect"
            };
            members.push(SemanticGroupMember {
                candidate_id: format!("{role}_{}", identities.id_of(entity_index)),
                scene_id: scene_id.to_string(),
                rank: None,
                sentence_index: relation.sentence_index,
                text: entity.text.clone(),
                evidence: entity.evidence.clone(),
                start_byte: entity.start,
                end_byte: entity.end,
                temporal_surface: None,
            });
        }
        if members.len() != 2 || evidence.is_empty() {
            continue;
        }
        groups.push(SemanticGroup {
            id: format!("cause_effect_{}_{}", scene_id, groups.len() + 1),
            group_type: "cause_effect".to_string(),
            title: format!("{} → {}", members[0].text, members[1].text),
            ranking_basis: Some("explicit_asserted_causal_relation".to_string()),
            target_count: Some(2),
            complete: Some(true),
            members,
        });
    }
    groups
}

/// Consolidates explicit comparisons of the same canonical pair when both
/// identities are compared in at least two scenes. Each member keeps its own
/// scene-local mention span so Go can independently ground the full group.
fn add_cross_scene_comparison_groups(input_scenes: &[SceneInput], manifests: &mut [SceneManifest]) {
    let mut comparisons: BTreeMap<Vec<String>, Vec<(usize, Vec<SemanticGroupMember>)>> =
        BTreeMap::new();
    for (scene_index, manifest) in manifests.iter().enumerate() {
        for group in manifest
            .semantic_groups
            .iter()
            .filter(|group| group.group_type == "comparison")
        {
            let mut identity_ids: Vec<String> = group
                .members
                .iter()
                .filter_map(|member| {
                    member
                        .candidate_id
                        .strip_prefix("comparison_")
                        .map(str::to_string)
                })
                .collect();
            identity_ids.sort();
            identity_ids.dedup();
            if identity_ids.len() < 2 || identity_ids.len() != group.members.len() {
                continue;
            }
            let mut members = group.members.clone();
            for member in &mut members {
                member.candidate_id = format!(
                    "comparison_evidence_{}_{}_{}",
                    input_scenes[scene_index].scene_id, member.candidate_id, member.start_byte
                );
            }
            comparisons
                .entry(identity_ids)
                .or_default()
                .push((scene_index, members));
        }
    }

    for (identity_ids, evidence) in comparisons {
        let scene_indices: BTreeSet<usize> = evidence.iter().map(|(index, _)| *index).collect();
        if scene_indices.len() < 2 {
            continue;
        }
        let first_scene = *scene_indices.first().expect("two scenes were checked");
        let mut members = Vec::new();
        for (_, pair) in evidence {
            members.extend(pair);
        }
        if members.len() < 4 {
            continue;
        }
        let title = format!(
            "Confronto: {}",
            members
                .iter()
                .take(identity_ids.len())
                .map(|member| member.text.as_str())
                .collect::<Vec<_>>()
                .join(" vs ")
        );
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!("cross_comparison_{}", identity_ids.join("_")),
            group_type: "comparison".to_string(),
            title,
            ranking_basis: Some("explicit_pair_comparison_across_scenes".to_string()),
            target_count: None,
            complete: None,
            members,
        });
    }
}

/// Joins explicit causal edges into a cross-scene chain only when the first
/// edge's effect is the next edge's cause. Both edges must already have
/// grounded, asserted scene-local groups; co-mentions and uncertain language
/// cannot enter this projection.
fn add_cross_scene_causal_groups(input_scenes: &[SceneInput], manifests: &mut [SceneManifest]) {
    let mut edges = Vec::new();
    for (scene_index, manifest) in manifests.iter().enumerate() {
        for group in manifest
            .semantic_groups
            .iter()
            .filter(|group| group.group_type == "cause_effect" && group.members.len() == 2)
        {
            let Some(source_id) = group.members[0].candidate_id.strip_prefix("cause_") else {
                continue;
            };
            let Some(target_id) = group.members[1].candidate_id.strip_prefix("effect_") else {
                continue;
            };
            edges.push((
                scene_index,
                source_id.to_string(),
                target_id.to_string(),
                group.members.clone(),
            ));
        }
    }

    let mut successors: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (edge_index, edge) in edges.iter().enumerate() {
        successors
            .entry(edge.1.clone())
            .or_default()
            .push(edge_index);
    }
    let mut chains = Vec::new();
    for first in &edges {
        if let Some(next_indices) = successors.get(&first.2) {
            for &next_index in next_indices {
                let second = &edges[next_index];
                if first.0 < second.0 {
                    chains.push((first.clone(), second.clone()));
                }
            }
        }
    }

    for (first, second) in chains {
        let first_scene = first.0;
        let mut members = first.3;
        members.extend(second.3);
        for member in &mut members {
            member.candidate_id = format!(
                "causal_chain_evidence_{}_{}_{}",
                member.scene_id, member.candidate_id, member.start_byte
            );
        }
        let title = format!(
            "{} → {} → {}",
            members[0].text, members[1].text, members[3].text
        );
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!(
                "cross_cause_effect_{}_{}_{}_{}_{}",
                first.1,
                first.2,
                second.2,
                input_scenes[first.0].scene_id,
                input_scenes[second.0].scene_id
            ),
            group_type: "cause_effect".to_string(),
            title,
            ranking_basis: Some("explicit_asserted_causal_chain_across_scenes".to_string()),
            target_count: None,
            complete: None,
            members,
        });
    }
}

/// Joins explicit process phases across scenes when each phase independently
/// names the same canonical identity. A group is anchored to its first scene,
/// while each member carries its own scene ID and local UTF-8 span.
fn add_cross_scene_process_groups(
    input_scenes: &[SceneInput],
    manifests: &mut [SceneManifest],
    language: &str,
) {
    let mut phases: BTreeMap<String, Vec<(usize, usize, SemanticGroupMember, String)>> =
        BTreeMap::new();
    let mut invalid = BTreeSet::new();
    for (scene_index, (input, manifest)) in input_scenes.iter().zip(manifests.iter()).enumerate() {
        if manifest.status != "valid" {
            continue;
        }
        let sentences = semantic_core::split_sentences(&input.text, language, None);
        for (sentence_index, sentence) in sentences.iter().enumerate() {
            let Some(text) = input.text.get(sentence.start_byte..sentence.end_byte) else {
                continue;
            };
            if fact_status(&text.to_lowercase()) != "asserted" {
                continue;
            }
            let Some((phase, phase_id)) = process_phase(text) else {
                continue;
            };
            for entity in manifest.entities.iter().filter(|entity| {
                entity.start_byte >= sentence.start_byte
                    && entity.end_byte <= sentence.end_byte
                    && !matches!(
                        entity.kind.as_str(),
                        "NUMBER" | "PERCENT" | "MONEY" | "DATE"
                    )
            }) {
                let members = phases.entry(entity.id.clone()).or_default();
                if members
                    .last()
                    .is_some_and(|(previous, _, _, _)| *previous == phase)
                {
                    continue;
                }
                if members
                    .last()
                    .is_some_and(|(previous, _, _, _)| *previous > phase)
                {
                    invalid.insert(entity.id.clone());
                    continue;
                }
                members.push((
                    phase,
                    scene_index,
                    SemanticGroupMember {
                        candidate_id: format!("cross_process_{}_{}", entity.id, phase_id),
                        scene_id: input.scene_id.clone(),
                        rank: None,
                        sentence_index,
                        text: text.to_string(),
                        evidence: text.to_string(),
                        start_byte: sentence.start_byte,
                        end_byte: sentence.end_byte,
                        temporal_surface: None,
                    },
                    entity.text.clone(),
                ));
            }
        }
    }

    for (identity_id, members) in phases {
        if invalid.contains(&identity_id) || members.len() < 3 {
            continue;
        }
        let distinct_scenes: BTreeSet<usize> =
            members.iter().map(|(_, scene, _, _)| *scene).collect();
        if distinct_scenes.len() < 2 {
            continue;
        }
        // A complete local group is already a better representation of the
        // same process. Avoid publishing overlapping local and global groups.
        let local_group_id = format!("process_{identity_id}");
        if manifests.iter().any(|manifest| {
            manifest
                .semantic_groups
                .iter()
                .any(|group| group.id == local_group_id)
        }) {
            continue;
        }
        let first_scene = members[0].1;
        let entity_name = &members[0].3;
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!("cross_process_{identity_id}"),
            group_type: "process".to_string(),
            title: format!("Processo: {entity_name}"),
            ranking_basis: Some("explicit_source_phase_order".to_string()),
            target_count: None,
            complete: None,
            members: members
                .into_iter()
                .map(|(_, _, member, _)| member)
                .collect(),
        });
    }
}

/// Recognizes only explicit, consecutively numbered source lines. A declared
/// Top 10 is complete only when all ten distinct ranks are present; no items
/// are generated to fill a gap.
fn numbered_groups(scene_id: &str, source: &str, language: &str) -> Vec<SemanticGroup> {
    let target_count =
        if source.to_lowercase().contains("top 10") || source.to_lowercase().contains("top ten") {
            Some(10usize)
        } else {
            None
        };
    let mut candidates = Vec::<(usize, usize, usize, String)>::new();
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let leading = content.len() - content.trim_start().len();
        let trimmed = &content[leading..];
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && digits <= 2 {
            let delimiter = trimmed.as_bytes().get(digits).copied();
            if matches!(delimiter, Some(b'.' | b')'))
                && trimmed
                    .as_bytes()
                    .get(digits + 1)
                    .is_some_and(u8::is_ascii_whitespace)
            {
                if let Ok(rank) = trimmed[..digits].parse::<usize>() {
                    if rank > 0 {
                        let start = offset + leading;
                        let end = offset + content.len();
                        candidates.push((rank, start, end, content[leading..].to_string()));
                    }
                }
            }
        }
        offset += line.len();
    }
    if candidates.len() < 2 {
        return Vec::new();
    }
    let ascending = candidates.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1);
    let descending = candidates.windows(2).all(|pair| pair[0].0 == pair[1].0 + 1);
    if !ascending && !descending {
        return Vec::new();
    }
    if target_count.is_some_and(|target| candidates.len() > target) {
        return Vec::new();
    }
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut members = Vec::with_capacity(candidates.len());
    for (rank, start, end, evidence) in &candidates {
        if check_span(source, *start, *end, evidence) != Grounding::Grounded {
            return Vec::new();
        }
        let sentence_index = sentences
            .iter()
            .position(|sentence| *start >= sentence.start_byte && *start < sentence.end_byte)
            .unwrap_or(members.len());
        members.push(SemanticGroupMember {
            candidate_id: format!("ranked_item_{rank:02}"),
            scene_id: scene_id.to_string(),
            rank: Some(*rank),
            sentence_index,
            text: evidence.clone(),
            evidence: evidence.clone(),
            start_byte: *start,
            end_byte: *end,
            temporal_surface: None,
        });
    }
    let complete = target_count.map(|target| {
        candidates.len() == target
            && (1..=target).all(|rank| candidates.iter().any(|item| item.0 == rank))
    });
    vec![SemanticGroup {
        id: "numbered_list_01".to_string(),
        group_type: if target_count.is_some() {
            "ranking"
        } else {
            "enumeration"
        }
        .to_string(),
        title: if target_count.is_some() {
            "Top 10"
        } else {
            "Elenco numerato"
        }
        .to_string(),
        ranking_basis: Some("explicit_source_order".to_string()),
        target_count,
        complete,
        members,
    }]
}

fn to_visual_proposal(
    candidate: VisualCandidate,
    source: &str,
    sentence_spans: &[semantic_core::sentences::SentenceSpan],
) -> VisualProposal {
    let score = match candidate.kind {
        "chart" => 0.9,
        "timeline" => 0.8,
        "multiple_entities" => 0.7,
        _ => 0.6,
    };
    let entity_query = candidate.entity_texts.join(" ");
    let mut visual_query_suggestions = Vec::new();
    if !entity_query.is_empty() {
        visual_query_suggestions.push(entity_query.clone());
        if candidate.kind == "entity_search" {
            let sentence = sentence_spans
                .iter()
                .find(|span| candidate.start >= span.start_byte && candidate.end <= span.end_byte);
            if let Some(sentence) = sentence {
                if let Some(context) =
                    contextual_visual_query(source, sentence, candidate.start, candidate.end)
                {
                    if context != entity_query {
                        visual_query_suggestions.push(context.to_string());
                    }
                }
                let sentence = sentence.text(source);
                if sentence != entity_query
                    && !visual_query_suggestions
                        .iter()
                        .any(|query| query == sentence)
                {
                    visual_query_suggestions.push(sentence.to_string());
                }
            }
        } else if candidate.evidence != entity_query {
            visual_query_suggestions.push(candidate.evidence.clone());
        }
    }
    VisualProposal {
        proposal_type: candidate.kind.to_string(),
        entity_ids: candidate.entity_ids,
        visual_query_suggestions,
        suggested_visual: candidate
            .suggested_layout
            .unwrap_or(candidate.kind)
            .to_string(),
        status: "requires_semantic_validation".to_string(),
        score,
        chart_items: candidate.chart_items,
        temporal_surfaces: candidate.temporal_surfaces,
        evidence: candidate.evidence,
        start_byte: candidate.start,
        end_byte: candidate.end,
    }
}

/// Returns a short, contiguous source phrase around an entity mention. This
/// gives image/stock providers a useful multiword concept while preserving an
/// exact source surface; the full canonical sentence remains available too.
fn contextual_visual_query<'a>(
    source: &'a str,
    sentence: &semantic_core::sentences::SentenceSpan,
    mention_start: usize,
    mention_end: usize,
) -> Option<&'a str> {
    const MAX_WORDS: usize = 8;
    const WORDS_BEFORE: usize = 2;
    const WORDS_AFTER: usize = 4;

    let text = sentence.text(source);
    let words: Vec<(usize, usize)> = text
        .split_whitespace()
        .map(|word| {
            let start = word.as_ptr() as usize - text.as_ptr() as usize;
            let end = start + word.len();
            (start + sentence.start_byte, end + sentence.start_byte)
        })
        .collect();
    let first = words
        .iter()
        .position(|(start, end)| *start < mention_end && mention_start < *end)?;
    let last = words
        .iter()
        .rposition(|(start, end)| *start < mention_end && mention_start < *end)?;
    let mut first = first.saturating_sub(WORDS_BEFORE);
    let mut after = (last + 1 + WORDS_AFTER).min(words.len());
    if after - first > MAX_WORDS {
        after = first + MAX_WORDS;
    }
    if last + 1 > after {
        after = last + 1;
        first = after.saturating_sub(MAX_WORDS);
    }
    let start = words.get(first)?.0;
    let end = words.get(after.checked_sub(1)?)?.1;
    source.get(start..end)
}

/// A visual subject is something that can be pictured: not a measurement.
fn is_visual_subject(entity: &VisualEntity) -> bool {
    !matches!(
        entity.r#type.as_str(),
        "NUMBER" | "PERCENT" | "MONEY" | "DATE"
    )
}

fn retain_grounded_entities(
    source: &str,
    entities: Vec<VisualEntity>,
) -> (Vec<VisualEntity>, usize) {
    let total = entities.len();
    let kept: Vec<VisualEntity> = entities
        .into_iter()
        .filter(|entity| {
            entity.text == entity.evidence
                && check_span(source, entity.start, entity.end, &entity.evidence)
                    == Grounding::Grounded
        })
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}

fn retain_grounded_temporal(source: &str, facts: Vec<TemporalFact>) -> Vec<TemporalFact> {
    facts
        .into_iter()
        .filter(|fact| {
            check_span(source, fact.start_byte, fact.end_byte, &fact.evidence)
                == Grounding::Grounded
        })
        .collect()
}

fn retain_grounded_candidates(
    source: &str,
    candidates: Vec<VisualCandidate>,
) -> (Vec<VisualCandidate>, usize) {
    let total = candidates.len();
    let kept: Vec<VisualCandidate> = candidates
        .into_iter()
        .filter(|candidate| {
            check_span(source, candidate.start, candidate.end, &candidate.evidence)
                == Grounding::Grounded
        })
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}
