//! Scene Intelligence shared semantic signals (analysis-only, shadow mode).
//!
//! Single shared analysis producing seven signal kinds for one canonical
//! manifest. This module does NOT embed, rank media, plan overlays, or
//! render: it identifies what the script says (facts, relations, suggestions
//! with text references). PipelineGen plans, RenderingGen draws.
//!
//! Reuse rule: no new NER, no new embedding engine, no media ranking here.
//! Entity surfaces arrive as input (from the existing NLP/semantic context);
//! sentence splitting and tokenization are the only shared text utilities,
//! kept deliberately small and language-independent.
//!
//! Determinism: pure functions of (input text, entities, config versions).
//! No randomness, no wall-clock, no global state. Outputs are sorted.

use crate::error::MuscleError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Canonical manifest schema version.
pub const SCHEMA_VERSION: &str = "scene_intelligence.v1";
/// Analyzer version pinned into every manifest (part of the identity).
pub const ANALYZER_VERSION: &str = "scene_intelligence.v1";
/// Relation registry version (single typed contract shared by comparison,
/// causal and entity-graph signals).
pub const RELATION_REGISTRY_VERSION: &str = "relation_registry.v1";
/// Visual selection budget for entity graph compositions. The semantic
/// manifest keeps every extracted relation; renderer/planner admission applies
/// this separate display budget.
pub const MAX_RELATION_NODES_PER_SCENE: usize = 6;

// ── Shared context ────────────────────────────────────────────────

/// One scene's text with its video-playback window (kept separate from any
/// historical event time mentioned inside the text).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneText {
    pub scene_id: String,
    pub text: String,
    #[serde(default)]
    pub language: String,
    pub video_start_ms: i64,
    pub video_end_ms: i64,
}

/// One indexed sentence (global narration order across all scenes).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Sentence {
    pub id: usize,
    pub scene_id: String,
    pub index_in_scene: usize,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Tokenizer + sentence index shared by every detector.
#[derive(Clone, Debug, Default)]
pub struct SharedContext {
    pub sentences: Vec<Sentence>,
}

impl SharedContext {
    pub fn build(scenes: &[SceneText]) -> Self {
        let mut sentences = Vec::new();
        let mut id = 0usize;
        for scene in scenes {
            for (idx, segment) in split_sentences(&scene.text, &scene.language)
                .into_iter()
                .enumerate()
            {
                sentences.push(Sentence {
                    id,
                    scene_id: scene.scene_id.clone(),
                    index_in_scene: idx,
                    text: segment.text,
                    start_byte: segment.start_byte,
                    end_byte: segment.end_byte,
                });
                id += 1;
            }
        }
        Self { sentences }
    }

    pub fn sentences_for_scene(&self, scene_id: &str) -> Vec<&Sentence> {
        self.sentences
            .iter()
            .filter(|s| s.scene_id == scene_id)
            .collect()
    }
}

/// Split into canonical language-aware sentence spans, retaining original
/// UTF-8 byte offsets. SI deliberately disables phrase-impact's long-sentence
/// chunking so semantic evidence continues to represent whole sentences.
///
/// This is the shared [`crate::phrase_impact`] boundary detector (not a
/// second implementation): the only difference from
/// [`crate::phrase_impact::split_sentences`] is the chunking parameter, and
/// the returned [`crate::phrase_impact::Segment`] type is shared.
pub fn split_sentences(text: &str, language: &str) -> Vec<crate::phrase_impact::Segment> {
    crate::phrase_impact::split_sentence_spans(text, language, None)
}

/// Lowercase alphanumeric tokens (unicode-aware via char methods).
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '\'' || ch == '’' {
            cur.push(ch.to_lowercase().next().unwrap_or(ch));
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn contains_token(haystack: &[String], needle: &str) -> bool {
    haystack.iter().any(|t| t == needle)
}

fn contains_phrase(haystack: &[String], phrase: &[&str]) -> bool {
    if phrase.is_empty() || phrase.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(phrase.len())
        .any(|w| w.iter().zip(phrase.iter()).all(|(a, b)| a == b))
}

// ── 1. Information density ────────────────────────────────────────

mod causal;
mod comparison;
mod density;
mod manifest;
mod relations;
mod temporal;

pub use causal::*;
pub use comparison::*;
pub use density::*;
pub use manifest::*;
pub use relations::*;
pub use temporal::*;


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn scenes(texts: &[(&str, &str)]) -> (Vec<SceneText>, SharedContext) {
        let v: Vec<SceneText> = texts
            .iter()
            .map(|(id, t)| SceneText {
                scene_id: id.to_string(),
                text: t.to_string(),
                language: "it".to_string(),
                video_start_ms: 0,
                video_end_ms: 20_000,
            })
            .collect();
        let ctx = SharedContext::build(&v);
        (v, ctx)
    }

    #[test]
    fn density_repetition_does_not_inflate() {
        let (_v, ctx) = scenes(&[(
            "scene-01",
            "I costi energetici riducono i margini delle imprese industriali.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-01");
        let base = analyze_density("scene-01", &sents, &["imprese".into()], &BTreeSet::new());
        // Same content padded with verbatim repetition.
        let (_v2, ctx2) = scenes(&[(
            "scene-01",
            "I costi energetici riducono i margini delle imprese industriali. I costi energetici riducono i margini delle imprese industriali. I costi energetici riducono i margini delle imprese industriali.",
        )]);
        let sents2: Vec<&Sentence> = ctx2.sentences_for_scene("scene-01");
        let padded = analyze_density("scene-01", &sents2, &["imprese".into()], &BTreeSet::new());
        assert_eq!(
            base.score, padded.score,
            "repeated words must not raise density"
        );
        assert_eq!(base.new_concepts, padded.new_concepts);
    }

    #[test]
    fn comparison_qualitative_invents_no_numbers() {
        let (_v, ctx) = scenes(&[(
            "scene-07",
            "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-07");
        let c = detect_comparison("scene-07", &sents, &["Ford".into(), "Toyota".into()])
            .expect("comparison detected");
        assert_eq!(c.candidate_visual, "split_comparison");
        assert!(
            c.values.is_empty(),
            "qualitative comparison must not invent values"
        );
        assert_eq!(c.subjects.len(), 2);
    }

    #[test]
    fn dopo_is_sequence_not_cause() {
        let (_v, ctx) = scenes(&[
            ("s1", "Le vendite sono diminuite dopo l'aumento dei prezzi."),
            (
                "s2",
                "Le vendite sono diminuite a causa dell'aumento dei prezzi.",
            ),
        ]);
        let seq: Vec<&Sentence> = ctx.sentences_for_scene("s1");
        let cau: Vec<&Sentence> = ctx.sentences_for_scene("s2");
        let r1 = extract_causal(seq[0]).expect("sequence extracted");
        let r2 = extract_causal(cau[0]).expect("cause extracted");
        assert_eq!(r1.relation, "precedes", "dopo is temporal sequence");
        assert_eq!(r2.relation, "causes", "a causa di is asserted causation");
        assert_ne!(r1.relation, r2.relation);
    }

    #[test]
    fn timeline_event_time_separate_from_video_time() {
        let (_v, ctx) = scenes(&[("scene-03", "Nel 2024 la crisi rientra. Già nel 2020 i costi esplosero, e nel 2022 i margini crollarono.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-03");
        let events = extract_temporal(&sents);
        let years: Vec<i32> = events.iter().filter_map(|e| e.normalized_year).collect();
        assert_eq!(
            years,
            vec![2020, 2022, 2024],
            "chronological order despite narration order"
        );
        // Narration order preserved alongside.
        let narr: Vec<usize> = events.iter().map(|e| e.narration_index).collect();
        assert_eq!(
            narr,
            vec![1, 1, 0],
            "chronological sorting preserves each event's narration index"
        );
        // No video timestamps leak into events.
        let j = serde_json::to_string(&events).unwrap();
        assert!(
            !j.contains("video"),
            "event time and video time stay separate"
        );
        // Relative expressions stay relative.
        let (_v2, ctx2) = scenes(&[("s", "Tre anni dopo gli investimenti crollarono.")]);
        let s2: Vec<&Sentence> = ctx2.sentences_for_scene("s");
        let rel = extract_temporal(&s2);
        assert!(
            rel.iter().any(|e| e.normalized_year.is_none()),
            "ambiguous relative stays unresolved"
        );
    }

    #[test]
    fn comentions_without_marker_link_nothing() {
        let (_v, ctx) = scenes(&[(
            "s",
            "Ford e Toyota restano due grandi case automobilistiche.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let rels = extract_relations(sents[0], &["ford".into(), "toyota".into()]);
        assert!(rels.is_empty(), "co-mention alone is not a relation");
        let (_v2, ctx2) = scenes(&[("s", "Ford collabora con Toyota sulle batterie.")]);
        let s2: Vec<&Sentence> = ctx2.sentences_for_scene("s");
        let rels2 = extract_relations(s2[0], &["ford".into(), "toyota".into()]);
        assert_eq!(rels2.len(), 1);
        assert_eq!(rels2[0].relation, "partners_with");
    }

    #[test]
    fn manifest_deterministic_and_valid() {
        let sc = vec![
            SceneText {
                scene_id: "scene-02".into(),
                text: "Ford cresce, Toyota rallenta.".into(),
                language: "it".into(),
                video_start_ms: 0,
                video_end_ms: 20000,
            },
            SceneText {
                scene_id: "scene-01".into(),
                text: "Nel 2020 i costi energetici aumentarono.".into(),
                language: "it".into(),
                video_start_ms: 0,
                video_end_ms: 20000,
            },
        ];
        let ents = vec![vec!["Ford".into(), "Toyota".into()], vec!["costi".into()]];
        let a = analyze(&sc, &ents);
        let b = analyze(&sc, &ents);
        assert_eq!(a, b, "same inputs must give identical manifests");
        validate_manifest(&a).expect("valid manifest");
        assert_eq!(
            a.scenes[0].scene_id, "scene-01",
            "scenes sorted for canonical JSON"
        );
    }

    #[test]
    fn manifest_rejects_broken_scene_references_and_evidence() {
        let scene = SceneSignals {
            scene_id: "s".into(),
            density: Density {
                scene_id: "other".into(),
                score: 0.2,
                level: "low".into(),
                new_entities: 0,
                numeric_claims: 0,
                new_concepts: 0,
                suggested_visual_budget: "standard".into(),
            },
            comparisons: vec![],
            causal: vec![],
            temporal_events: vec![],
            relations: vec![],
        };
        let mut manifest = SceneIntelligenceManifest {
            schema_version: SCHEMA_VERSION.into(),
            analyzer_version: ANALYZER_VERSION.into(),
            relation_registry_version: RELATION_REGISTRY_VERSION.into(),
            scenes: vec![scene],
        };
        assert!(validate_manifest(&manifest)
            .unwrap_err()
            .contains("scene reference mismatch"));
        manifest.scenes[0].density.scene_id = "s".into();
        manifest.scenes[0].relations.push(EntityRelation {
            subject: "a".into(),
            object: "b".into(),
            relation: "partners_with".into(),
            evidence_sentence_ids: vec![],
        });
        assert!(validate_manifest(&manifest)
            .unwrap_err()
            .contains("evidence missing"));
    }

    #[test]
    fn manifest_rejects_qualitative_with_values() {
        let mut m = analyze(
            &[SceneText {
                scene_id: "s".into(),
                text: "Ford cresce, Toyota rallenta.".into(),
                language: "it".into(),
                video_start_ms: 0,
                video_end_ms: 1000,
            }],
            &[vec!["Ford".into(), "Toyota".into()]],
        );
        if let Some(c) = m.scenes[0].comparisons.first_mut() {
            c.values.push(42.0);
        } else {
            panic!("expected a comparison");
        }
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn density_high_suggests_low_budget() {
        let (_v, ctx) = scenes(&[(
            "scene-04",
            "Nel 2020 Siemens, Volkswagen e Bosch annunciarono investimenti da 120 milioni mentre i costi energetici del 34% riducevano i margini industriali e la collaborazione con E.ON ridefiniva la strategia.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-04");
        let d = analyze_density(
            "scene-04",
            &sents,
            &[
                "Siemens".into(),
                "Volkswagen".into(),
                "Bosch".into(),
                "E.ON".into(),
            ],
            &BTreeSet::new(),
        );
        assert_eq!(
            d.suggested_visual_budget, "low",
            "dense scenes get few simultaneous overlays"
        );
        assert!(d.numeric_claims >= 2);
    }

    #[test]
    fn relation_extractor_retains_each_explicit_evidenced_edge() {
        let inputs = [
            (
                0,
                "Ford collabora con Toyota.",
                "ford",
                "toyota",
                "partners_with",
            ),
            (
                1,
                "Ford compete con Volkswagen.",
                "ford",
                "volkswagen",
                "competes_with",
            ),
            (2, "Siemens finanzia Bosch.", "siemens", "bosch", "funds"),
            (3, "Bosch possiede E.ON.", "bosch", "e.on", "owns"),
        ];
        for (id, text, subject, object, expected_kind) in inputs {
            let sentence = Sentence {
                id,
                scene_id: "s".into(),
                index_in_scene: id,
                text: text.into(),
                start_byte: 0,
                end_byte: text.len(),
            };
            let entities = vec![subject.into(), object.into()];
            let relations = extract_relations(&sentence, &entities);
            assert_eq!(relations.len(), 1, "one marker-grounded edge: {text}");
            assert_eq!(
                relations[0].relation, expected_kind,
                "typed relation: {text}"
            );
            assert_eq!(relations[0].evidence_sentence_ids, vec![id]);
        }
    }

    #[test]
    fn shared_context_sentence_index() {
        let (_v, ctx) = scenes(&[("a", "Prima frase. Seconda frase!"), ("b", "Terza?")]);
        assert_eq!(ctx.sentences.len(), 3);
        assert_eq!(ctx.sentences[0].id, 0);
        assert_eq!(ctx.sentences[2].scene_id, "b");
        assert_eq!(ctx.sentences_for_scene("a").len(), 2);
    }

    // ── Runtime certification: comparison ──

    #[test]
    fn cert_temporal_comparison_years_as_subjects() {
        // C02: "Vendite 2020 vs 2025" is a temporal comparison; years are
        // temporal evidence, never invented quantities.
        let (_v, ctx) = scenes(&[("s", "Vendite 2020 vs 2025: il trend è chiaro.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let c = detect_comparison("s", &sents, &[]).expect("temporal comparison");
        assert_eq!(c.dimension, "temporal");
        assert_eq!(c.relation, "contrast");
        assert!(c.values.is_empty(), "years are not quantities");
    }

    #[test]
    fn cert_negated_comparative_keeps_negation() {
        // C05: "non è superiore" stays a negated comparison.
        let (_v, ctx) = scenes(&[("s", "Ford non è superiore a Toyota negli investimenti.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let c = detect_comparison("s", &sents, &["Ford".into(), "Toyota".into()])
            .expect("comparison detected");
        assert!(c.negated, "negation must be preserved, not dropped");
        assert_eq!(c.subjects.len(), 2);
    }

    #[test]
    fn cert_paraphrased_comparison_equivalent() {
        // C06: same contrast paraphrased yields the same structure.
        let (_v, ctx) = scenes(&[
            (
                "a",
                "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini.",
            ),
            (
                "b",
                "Toyota rallenta sui margini mentre Ford cresce con gli ibridi.",
            ),
        ]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let ents = vec!["Ford".into(), "Toyota".into()];
        let ca = detect_comparison("a", &sa, &ents).expect("a");
        let cb = detect_comparison("b", &sb, &ents).expect("b");
        assert_eq!(ca.subjects, cb.subjects);
        assert_eq!(ca.dimension, cb.dimension);
        assert_eq!(ca.relation, cb.relation);
        assert_eq!(ca.candidate_visual, cb.candidate_visual);
    }

    // ── Runtime certification: cause–effect ──

    #[test]
    fn cert_negated_cause_yields_no_positive_relation() {
        // E03: denied causation produces no positive causal relation.
        let (_v, ctx) = scenes(&[("s", "La crisi non è stata causata dall'inflazione.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        assert!(
            extract_causal(sents[0]).is_none(),
            "negated cause is not a positive relation"
        );
    }

    #[test]
    fn cert_attributed_hypothetical_keeps_attribution() {
        // E04: attributed, conditional claims keep both markers with
        // lowered confidence — never a certain arrow.
        let (_v, ctx) = scenes(&[(
            "s",
            "Secondo gli analisti, l'aumento dei costi avrebbe provocato il calo dei margini.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert_eq!(r.relation, "causes");
        assert!(r.hypothetical, "conditional mood preserved");
        assert!(r.attribution.is_some(), "attribution preserved");
        assert!(
            r.extraction_confidence < 0.9,
            "hypothetical confidence discounted"
        );
    }

    #[test]
    fn cert_chain_extracts_links_without_transitive_invention() {
        // E05: A → B → C yields the asserted links, never an A → C shortcut.
        let (_v, ctx) = scenes(&[("s",
            "L'aumento dei costi ha causato il calo dei margini. Il calo dei margini ha provocato il crollo degli investimenti. La caduta dell'occupazione è stata causata dal crollo degli investimenti.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        assert_eq!(sents.len(), 3);
        let rels: Vec<CausalRelation> = sents.iter().filter_map(|s| extract_causal(s)).collect();
        assert_eq!(rels.len(), 3, "one relation per asserted link");
        let transitive = rels
            .iter()
            .any(|r| r.cause.contains("costi") && r.effect.contains("occupazione"));
        assert!(!transitive, "no invented A → C shortcut");
    }

    #[test]
    fn cert_reverse_order_keeps_direction() {
        // E06: "Y a causa di X" still means X causes Y.
        let (_v, ctx) = scenes(&[(
            "s",
            "I margini si sono ridotti a causa dell'aumento dei costi.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert!(r.cause.contains("costi"), "cause is costs, got {}", r.cause);
        assert!(
            r.effect.contains("margini"),
            "effect is margins, got {}",
            r.effect
        );
    }

    #[test]
    fn cert_contributes_to_distinct_from_causes() {
        let (_v, ctx) = scenes(&[(
            "s",
            "L'aumento dei costi energetici ha contribuito alla riduzione dei margini aziendali.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert_eq!(r.relation, "contributes_to", "contributes_to is not causes");
    }

    // ── Runtime certification: timeline ──

    #[test]
    fn cert_relative_resolves_with_anchor_not_without() {
        // T02/T03: known anchor resolves, missing anchor stays unresolved.
        let (_v, ctx) = scenes(&[("s", "Tre anni dopo gli investimenti crollarono.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let open = extract_temporal(&sents);
        assert!(
            open.iter().any(|e| e.normalized_year.is_none()),
            "T03: ambiguous relative stays unresolved"
        );
        let closed = extract_temporal_with_anchor(&sents, Some(2020));
        let rel = closed
            .iter()
            .find(|e| e.temporal_expression.contains("anni dopo"))
            .expect("relative event");
        assert_eq!(
            rel.normalized_year,
            Some(2023),
            "T02: anchor 2020 + 3 years"
        );
    }

    #[test]
    fn cert_interval_represented_as_span() {
        // T04: "2020–2023" is one interval, not an instant, and the two
        // years are not duplicated as bare singles.
        let (_v, ctx) = scenes(&[("s", "L'evento copre il periodo 2020–2023 con tre fasi.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let events = extract_temporal(&sents);
        assert_eq!(events.len(), 1, "one interval event, got {:?}", events);
        assert_eq!(events[0].normalized_year, Some(2020));
        assert_eq!(events[0].normalized_end_year, Some(2023));
    }

    #[test]
    fn cert_same_year_two_events_kept() {
        // T05: equal years for different events are both conserved.
        let (_v, ctx) = scenes(&[(
            "s",
            "Nel 2020 fu annunciato il progetto. Nel 2020 iniziarono i lavori.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let events = extract_temporal(&sents);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.normalized_year == Some(2020)));
        assert_ne!(events[0].event_id, events[1].event_id);
    }

    // ── Runtime certification: entity graph ──

    #[test]
    fn cert_ownership_and_collaboration_are_distinct() {
        // G01/G02: an explicit ownership marker and a collaboration marker
        // produce typed, non-interchangeable directed edges.
        let (_v, ctx) = scenes(&[
            ("owns", "Alfa possiede Beta."),
            ("partners", "Alfa collabora con Beta."),
        ]);
        let owns_sentence = ctx.sentences_for_scene("owns");
        let partners_sentence = ctx.sentences_for_scene("partners");
        let entities = &["alfa".to_string(), "beta".to_string()];
        let owns = extract_relations(owns_sentence[0], entities);
        let partners = extract_relations(partners_sentence[0], entities);
        assert_eq!(owns.len(), 1);
        assert_eq!(owns[0].relation, "owns");
        assert_eq!(partners.len(), 1);
        assert_eq!(partners[0].relation, "partners_with");
    }

    #[test]
    fn cert_negated_ownership_links_nothing_positive() {
        // G04: "non possiede più" keeps negation and time: no `owns`.
        let (_v, ctx) = scenes(&[("s", "Ford non possiede più Rivian dal 2023.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let rels = extract_relations(sents[0], &["ford".into(), "rivian".into()]);
        assert!(
            rels.iter().all(|r| r.relation != "owns"),
            "negated ownership is not owns"
        );
    }

    #[test]
    fn cert_alias_surfaces_share_identity() {
        // G05: casing/punctuation variants of the same supplied canonical
        // surface collapse; arbitrary abbreviations (e.g. VW) require an
        // upstream alias registry and are intentionally not guessed here.
        let subjects = ground_subjects(
            "Volkswagen (VOLKSWAGEN) cresce.",
            &["Volkswagen".into(), "volkswagen".into()],
        );
        assert_eq!(
            subjects.len(),
            1,
            "same normalized identity is emitted once"
        );
        assert_eq!(normalize_surface(&subjects[0]), "volkswagen");
    }

    #[test]
    fn cert_extractor_keeps_full_graph_for_separate_visual_budget() {
        // G06 data half: fifty distinct entities participate in one hundred
        // explicit relations. The semantic manifest must retain all edges;
        // the separate visual admission layer is responsible for display caps.
        let entity_name = |prefix: &str, index: usize| format!("{prefix}{index:02}");
        let sentences = (0..100)
            .map(|i| {
                format!(
                    "{} collabora con {}.",
                    entity_name("Azienda", i / 4),
                    entity_name("Partner", (i / 4 + i % 4) % 25),
                )
            })
            .collect::<Vec<_>>();
        let text = sentences.join(" ");
        let (_v, ctx) = scenes(&[("s", &text)]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let ents: Vec<String> = (0..25)
            .map(|i| entity_name("Azienda", i).to_lowercase())
            .chain((0..25).map(|i| entity_name("Partner", i).to_lowercase()))
            .collect();
        let all: Vec<EntityRelation> = sents
            .iter()
            .flat_map(|s| extract_relations(s, &ents))
            .collect();
        assert_eq!(ents.len(), 50, "fixture contains fifty entities");
        assert_eq!(
            all.len(),
            100,
            "complete graph data retains one hundred evidenced edges"
        );
        let sc = vec![SceneText {
            scene_id: "s".into(),
            text: text.clone(),
            language: "it".into(),
            video_start_ms: 0,
            video_end_ms: 20000,
        }];
        let m = analyze(&sc, &[ents]);
        assert_eq!(
            m.scenes[0].relations.len(),
            100,
            "semantic manifest retains the complete graph"
        );
        let graph_ids: BTreeSet<usize> = m.scenes[0]
            .relations
            .iter()
            .flat_map(|relation| relation.evidence_sentence_ids.iter().copied())
            .collect();
        assert_eq!(
            graph_ids.len(),
            100,
            "each relation preserves its distinct source sentence"
        );
        validate_manifest(&m).expect("complete graph manifest valid");
    }

    // ── Runtime certification: density ordering ──

    #[test]
    fn cert_density_ordering_sparse_vs_rich() {
        // D01 < D02: ordering over cases, never an exact magic number.
        let (_v, ctx) = scenes(&[
            ("sparse", "Il giorno era bello e calmo."),
            ("rich", "Alfa conta 17 tecnici e 23 impianti. Beta registra 31 ordini mentre Gamma cresce dopo Delta perché Epsilon investe 42 milioni rispetto a Zeta, Eta e Theta con 56 prodotti."),
        ]);
        let ss: Vec<&Sentence> = ctx.sentences_for_scene("sparse");
        let rs: Vec<&Sentence> = ctx.sentences_for_scene("rich");
        let sparse = analyze_density("sparse", &ss, &[], &BTreeSet::new());
        let rich = analyze_density(
            "rich",
            &rs,
            &[
                "Alfa".into(),
                "Beta".into(),
                "Gamma".into(),
                "Delta".into(),
                "Epsilon".into(),
                "Zeta".into(),
                "Eta".into(),
                "Theta".into(),
            ],
            &BTreeSet::new(),
        );
        assert_eq!(
            rich.new_entities, 8,
            "D02 fixture contains eight grounded entities"
        );
        assert_eq!(
            rich.numeric_claims, 5,
            "D02 fixture contains five stated numbers"
        );
        assert!(
            sparse.score < rich.score,
            "D01 < D02: {} vs {}",
            sparse.score,
            rich.score
        );
    }

    #[test]
    fn cert_density_fivefold_repetition_not_proportional() {
        // D03: repeating one concept 5x must not 5x the novelty.
        let one = "I costi energetici riducono i margini.";
        let five = [one; 5].join(" ");
        let (_v, ctx) = scenes(&[("a", one), ("b", &five)]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let d1 = analyze_density("a", &sa, &[], &BTreeSet::new());
        let d5 = analyze_density("b", &sb, &[], &BTreeSet::new());
        assert_eq!(d1.score, d5.score, "verbatim repetition adds no novelty");
    }

    #[test]
    fn cert_density_new_info_increases_at_same_length() {
        // D04: same word count, more distinct concepts → higher density.
        let repeated = vec!["costi"; 100].join(" ");
        let alphabetic_suffix = |mut n: usize| {
            let mut chars = Vec::new();
            loop {
                chars.push((b'a' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    break;
                }
            }
            chars.into_iter().rev().collect::<String>()
        };
        let distinct = (0..100)
            .map(|i| format!("concetto{}", alphabetic_suffix(i)))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(repeated.split_whitespace().count(), 100);
        assert_eq!(distinct.split_whitespace().count(), 100);
        let (_v, ctx) = scenes(&[("a", &repeated), ("b", &distinct)]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let da = analyze_density("a", &sa, &[], &BTreeSet::new());
        let db = analyze_density("b", &sb, &[], &BTreeSet::new());
        assert!(
            db.score > da.score,
            "100 distinct-concept words outrank 100 repeated words"
        );
        assert!(db.new_concepts > da.new_concepts);
    }

    #[test]
    fn cert_event_date_separate_from_video_window() {
        // T06: a 2020 spoken at video-second 60 keeps event_date on the
        // event and video time on the scene — never mixed.
        let sc = vec![SceneText {
            scene_id: "s".into(),
            text: "Nel 2020 i costi aumentarono.".into(),
            language: "it".into(),
            video_start_ms: 60_000,
            video_end_ms: 80_000,
        }];
        let m = analyze(&sc, &[vec![]]);
        let j = manifest_json(&m).expect("serializes");
        assert!(
            !j.contains("video_start_ms"),
            "no video timestamps inside the manifest"
        );
        assert!(!j.contains("video_end_ms"));
        assert!(j.contains("2020"), "event date preserved");
        let ev = &m.scenes[0].temporal_events;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].normalized_year, Some(2020));
    }

    #[test]
    fn cert_manifest_stable_over_100_runs() {
        // X06: same input 100 times → identical manifests.
        let sc = vec![SceneText {
            scene_id: "s".into(),
            text: "Ford cresce, Toyota rallenta. Nel 2020 i costi aumentarono.".into(),
            language: "it".into(),
            video_start_ms: 0,
            video_end_ms: 20000,
        }];
        let ents = vec![vec!["Ford".into(), "Toyota".into()]];
        let first = analyze(&sc, &ents);
        for _ in 0..100 {
            assert_eq!(analyze(&sc, &ents), first);
        }
    }
}
