//! Internal split of scene_intelligence (pure move, no behavior change).

use super::*;

// ── Manifest ──────────────────────────────────────────────────────

/// Per-scene signals.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneSignals {
    pub scene_id: String,
    pub density: Density,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comparisons: Vec<Comparison>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causal: Vec<CausalRelation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_events: Vec<TemporalEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<EntityRelation>,
}

/// Canonical manifest: the single analysis product every planner consumes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneIntelligenceManifest {
    pub schema_version: String,
    pub analyzer_version: String,
    pub relation_registry_version: String,
    pub scenes: Vec<SceneSignals>,
}

/// Full-pipeline analysis: shared context → detectors → canonical manifest.
/// `entities_per_scene` parallels `scenes` (same order) and carries the
/// existing NLP entity surfaces per scene.
pub fn analyze(
    scenes: &[SceneText],
    entities_per_scene: &[Vec<String>],
) -> SceneIntelligenceManifest {
    let ctx = SharedContext::build(scenes);
    let mut prior_concepts: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for (i, scene) in scenes.iter().enumerate() {
        let entities: &[String] = entities_per_scene
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let sents = ctx.sentences_for_scene(&scene.scene_id);
        let density = analyze_density(&scene.scene_id, &sents, entities, &prior_concepts);
        for s in &sents {
            for t in tokens(&s.text) {
                if t.chars().count() >= 5 {
                    prior_concepts.insert(t);
                }
            }
        }
        let comparisons = detect_comparison(&scene.scene_id, &sents, entities)
            .into_iter()
            .collect::<Vec<_>>();
        let mut causal = Vec::new();
        for s in &sents {
            if let Some(c) = extract_causal(s) {
                // Skip pure-sequence markers already covered as temporal flow?
                // No: keep both — timeline and causal consumers differ.
                causal.push(c);
            }
        }
        let temporal_events = extract_temporal(&sents);
        let mut relations = Vec::new();
        for s in &sents {
            relations.extend(extract_relations(s, entities));
        }
        out.push(SceneSignals {
            scene_id: scene.scene_id.clone(),
            density,
            comparisons,
            causal,
            temporal_events,
            relations,
        });
    }
    out.sort_by(|a, b| a.scene_id.cmp(&b.scene_id));
    SceneIntelligenceManifest {
        schema_version: SCHEMA_VERSION.to_string(),
        analyzer_version: ANALYZER_VERSION.to_string(),
        relation_registry_version: RELATION_REGISTRY_VERSION.to_string(),
        scenes: out,
    }
}

/// Fail-closed manifest validation (structure only, no semantic judgment).
pub fn validate_manifest(m: &SceneIntelligenceManifest) -> Result<(), MuscleError> {
    if m.schema_version != SCHEMA_VERSION {
        return Err(MuscleError::generic(format!(
            "unknown schema {}",
            m.schema_version
        )));
    }
    if m.analyzer_version.is_empty() || m.relation_registry_version.is_empty() {
        return Err(MuscleError::generic("missing version pins"));
    }
    let mut seen = BTreeSet::new();
    for s in &m.scenes {
        if s.scene_id.trim().is_empty() {
            return Err(MuscleError::generic("scene with empty id"));
        }
        if !seen.insert(s.scene_id.clone()) {
            return Err(MuscleError::generic(format!(
                "duplicate scene {}",
                s.scene_id
            )));
        }
        if s.density.scene_id != s.scene_id {
            return Err(MuscleError::generic(format!(
                "density scene reference mismatch in {}",
                s.scene_id
            )));
        }
        if !(0.0..=1.0).contains(&s.density.score) {
            return Err(MuscleError::generic(format!(
                "density out of range in {}",
                s.scene_id
            )));
        }
        for c in &s.comparisons {
            if c.scene_id != s.scene_id {
                return Err(MuscleError::generic(format!(
                    "comparison scene reference mismatch in {}",
                    s.scene_id
                )));
            }
            if c.subjects.len() < 2 || c.subjects.iter().any(|subject| subject.trim().is_empty()) {
                return Err(MuscleError::generic(format!(
                    "comparison subjects invalid in {}",
                    s.scene_id
                )));
            }
            if c.evidence_sentence_ids.is_empty() {
                return Err(MuscleError::generic(format!(
                    "comparison evidence missing in {}",
                    s.scene_id
                )));
            }
            if c.kind == "comparison"
                && c.candidate_visual == "split_comparison"
                && !c.values.is_empty()
            {
                return Err(MuscleError::generic(
                    "qualitative comparison must not carry values",
                ));
            }
            if c.values.iter().any(|value| !value.is_finite()) {
                return Err(MuscleError::generic(format!(
                    "comparison has non-finite value in {}",
                    s.scene_id
                )));
            }
        }
        for c in &s.causal {
            if c.cause.trim().is_empty()
                || c.effect.trim().is_empty()
                || !(0.0..=1.0).contains(&c.extraction_confidence)
            {
                return Err(MuscleError::generic(format!(
                    "causal relation invalid in {}",
                    s.scene_id
                )));
            }
            if c.evidence_sentence_ids.is_empty() {
                return Err(MuscleError::generic(format!(
                    "causal evidence missing in {}",
                    s.scene_id
                )));
            }
        }
        for event in &s.temporal_events {
            if event.event_id.trim().is_empty() || !(0.0..=1.0).contains(&event.certainty) {
                return Err(MuscleError::generic(format!(
                    "temporal event invalid in {}",
                    s.scene_id
                )));
            }
            if let (Some(start), Some(end)) = (event.normalized_year, event.normalized_end_year) {
                if end < start {
                    return Err(MuscleError::generic(format!(
                        "temporal interval is reversed in {}",
                        s.scene_id
                    )));
                }
            }
        }
        for relation in &s.relations {
            if relation.subject.trim().is_empty()
                || relation.object.trim().is_empty()
                || relation.subject == relation.object
            {
                return Err(MuscleError::generic(format!(
                    "entity relation endpoints invalid in {}",
                    s.scene_id
                )));
            }
            if relation.evidence_sentence_ids.is_empty() {
                return Err(MuscleError::generic(format!(
                    "entity relation evidence missing in {}",
                    s.scene_id
                )));
            }
        }
    }
    Ok(())
}

/// Canonical JSON (sorted scenes already guaranteed by `analyze`).
pub fn manifest_json(m: &SceneIntelligenceManifest) -> Result<String, MuscleError> {
    serde_json::to_string(m).map_err(|e| MuscleError::generic(e.to_string()))
}
