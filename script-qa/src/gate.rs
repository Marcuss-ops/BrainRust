use std::collections::{HashMap, HashSet};

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::dates::{explicit_calendar_dates, CalendarDate};
use crate::model::{Evaluation, GateRequest, PlanInput, Quality, ResultInput, SceneInput};
use crate::tokenize::{compute_source_text_coverage, detect_language, is_generic_text, tokenize};

pub const DEFAULT_MIN_SOURCE_TEXT_COVERAGE: f64 = 0.70;
pub const MIN_TARGET_WORDS_RATIO: f64 = 0.80;
pub const MAX_TARGET_WORDS_RATIO: f64 = 1.20;

/// Evaluate one gate request. `None` mirrors Go's `Evaluate(nil, ...)`:
/// a missing result has no quality verdict.
pub fn evaluate(input: &GateRequest) -> Option<Evaluation> {
    let result = input.result.as_ref()?;
    let plan = &input.plan;

    let requested_language = plan.language.trim().to_lowercase();
    let generated_text = result.output.text.trim().to_string();

    let mut quality = Quality {
        language_requested: requested_language.clone(),
        target_words: plan.target_words,
        actual_words: result.output.word_count,
        ..Quality::default()
    };

    if generated_text.is_empty() {
        quality.passed = false;
        return Some(Evaluation {
            quality,
            reasons: vec!["generated text is empty".to_string()],
        });
    }

    if is_generic_text(&generated_text) {
        quality.passed = false;
        return Some(Evaluation {
            quality,
            reasons: vec!["generated text is generic or placeholder".to_string()],
        });
    }

    quality.language_detected = detect_language(&generated_text);

    let source_text = build_source_text(plan);
    if source_text.trim().is_empty() {
        quality.source_text_coverage_status = "NOT_EVALUATED".to_string();
        quality.source_text_coverage = 0.0;
    } else {
        quality.source_text_coverage_status = "EVALUATED".to_string();
        quality.source_text_coverage = compute_source_text_coverage(&generated_text, &source_text);
    }

    quality.clip_evidence_coverage = compute_clip_evidence_coverage(plan, result);
    quality.unsupported_claims = count_unsupported_claims(result, &source_text);

    let (min_source_coverage, mut min_clip_coverage) = policy_thresholds(&plan.grounding_policy);
    if plan
        .clip_evidence
        .as_ref()
        .is_none_or(|evidence| evidence.accepted_clip_ids.is_empty())
    {
        min_clip_coverage = 0.0;
    }

    let mut reasons = Vec::new();
    reasons.extend(language_match_reasons(result, plan, &quality));
    reasons.extend(source_coverage_reasons(&quality, min_source_coverage));
    reasons.extend(source_date_preservation_reasons(result, plan));
    reasons.extend(clip_coverage_reasons(
        &quality,
        min_clip_coverage,
        &plan.grounding_policy,
    ));
    reasons.extend(unsupported_claims_reasons(&quality, plan));
    reasons.extend(research_candidate_coverage_reasons(result, plan));
    reasons.extend(segment_narrative_reasons(result, plan));
    reasons.extend(target_words_reasons(&quality, plan, &source_text));

    quality.passed = reasons.is_empty();
    Some(Evaluation { quality, reasons })
}

pub fn policy_thresholds(policy: &str) -> (f64, f64) {
    match policy {
        "clips_primary" => (0.40, 1.00),
        "source_primary" => (0.85, 0.00),
        "balanced" => (0.60, 0.50),
        _ => (DEFAULT_MIN_SOURCE_TEXT_COVERAGE, 0.00),
    }
}

pub fn compute_clip_evidence_coverage(plan: &PlanInput, result: &ResultInput) -> f64 {
    let Some(evidence) = plan.clip_evidence.as_ref() else {
        return 1.0;
    };
    let mut accepted = evidence.accepted_clip_ids.as_slice();
    if plan.num_clips > 0 && plan.num_clips < accepted.len() {
        accepted = &accepted[..plan.num_clips];
    }
    if accepted.is_empty() {
        return 1.0;
    }

    let bound: HashSet<&str> = result
        .output
        .specscene
        .scenes
        .iter()
        .filter_map(|scene| scene.bindings.clip.as_ref())
        .map(|clip| clip.clip_id.as_str())
        .filter(|clip_id| !clip_id.is_empty())
        .collect();

    let matches = accepted
        .iter()
        .filter(|clip_id| bound.contains(clip_id.as_str()))
        .count();
    matches as f64 / accepted.len() as f64
}

pub fn source_date_preservation_reasons(result: &ResultInput, plan: &PlanInput) -> Vec<String> {
    if plan.segments.is_empty() {
        return Vec::new();
    }

    let scenes = &result.output.specscene.scenes;
    let by_segment: HashMap<&str, &SceneInput> = scenes
        .iter()
        .filter(|scene| !scene.segment_id.trim().is_empty())
        .map(|scene| (scene.segment_id.trim(), scene))
        .collect();

    let mut reasons = Vec::new();
    for (index, segment) in plan.segments.iter().enumerate() {
        let expected = explicit_calendar_dates(&segment.source_text);
        if expected.is_empty() {
            continue;
        }

        let body = if let Some(scene) = by_segment.get(segment.id.trim()) {
            scene.text.as_str()
        } else if scenes.len() == plan.segments.len() && index < scenes.len() {
            scenes[index].text.as_str()
        } else {
            result.output.text.as_str()
        };

        let actual: HashSet<CalendarDate> = explicit_calendar_dates(body).into_iter().collect();
        for date in expected {
            if !actual.contains(&date) {
                reasons.push(format!(
                    "segment \"{}\" does not preserve source date {}",
                    segment.id, date
                ));
            }
        }
    }
    reasons
}

fn build_source_text(plan: &PlanInput) -> String {
    let mut source = plan.source_text.clone();
    if let Some(evidence) = plan.clip_evidence.as_ref() {
        source.push(' ');
        source.push_str(&evidence.model_source_text());
    }
    source
}

fn language_match_reasons(
    result: &ResultInput,
    plan: &PlanInput,
    quality: &Quality,
) -> Vec<String> {
    let requested = plan.language.trim().to_lowercase();
    if requested.is_empty() {
        return Vec::new();
    }

    let mut reasons = Vec::new();
    if !quality.language_detected.is_empty() && quality.language_detected != requested {
        reasons.push(format!(
            "detected language {} does not match requested language {}",
            quality.language_detected, requested
        ));
    }

    for scene in &result.output.specscene.scenes {
        let text = scene.text.trim();
        if tokenize(text).len() < 30 {
            continue;
        }
        let detected = detect_language(text);
        if !detected.is_empty() && detected != requested {
            reasons.push(format!(
                "scene {} detected language {} does not match requested language {}",
                scene.id.trim(),
                detected,
                requested
            ));
        }
    }
    reasons
}

fn source_coverage_reasons(quality: &Quality, min_source_coverage: f64) -> Vec<String> {
    if quality.source_text_coverage_status == "EVALUATED"
        && quality.source_text_coverage < min_source_coverage
    {
        return vec!["source_text coverage below threshold".to_string()];
    }
    Vec::new()
}

fn clip_coverage_reasons(
    quality: &Quality,
    min_clip_coverage: f64,
    grounding_policy: &str,
) -> Vec<String> {
    if quality.clip_evidence_coverage < min_clip_coverage {
        let policy_label = if grounding_policy.trim().is_empty() {
            "default"
        } else {
            grounding_policy
        };
        return vec![format!(
            "clip_evidence coverage below threshold for {}",
            policy_label
        )];
    }
    Vec::new()
}

fn unsupported_claims_reasons(quality: &Quality, plan: &PlanInput) -> Vec<String> {
    if quality.unsupported_claims > 0 && unsupported_claims_are_blocking(plan) {
        return vec!["unsupported claims detected".to_string()];
    }
    Vec::new()
}

fn unsupported_claims_are_blocking(plan: &PlanInput) -> bool {
    if !plan.grounding_policy.trim().is_empty() {
        return true;
    }
    matches!(
        plan.source_kind.as_str(),
        "research" | "clips" | "catalog" | "search" | "curate"
    )
}

fn count_unsupported_claims(result: &ResultInput, source_text: &str) -> i32 {
    let Some(entities) = result.artifacts.entities.as_ref() else {
        return 0;
    };

    let source_tokens: HashSet<String> = tokenize(source_text).into_iter().collect();
    let mut count = 0;
    for entity in entities
        .persons
        .iter()
        .chain(entities.places.iter())
        .chain(entities.concepts.iter())
    {
        let name = entity.value.trim();
        if name.is_empty() {
            continue;
        }
        let tokens = tokenize(name);
        if tokens.is_empty() {
            continue;
        }
        if tokens
            .iter()
            .any(|token| !source_tokens.contains(token.as_str()))
        {
            count += 1;
        }
    }
    count
}

fn research_candidate_coverage_reasons(result: &ResultInput, plan: &PlanInput) -> Vec<String> {
    let Some(evidence) = plan.research_evidence.as_ref() else {
        return Vec::new();
    };
    if evidence.candidates.is_empty() {
        return Vec::new();
    }

    let text = research_coverage_text(&result.output.text);
    let missing: Vec<&str> = evidence
        .candidates
        .iter()
        .filter(|candidate| !text.contains(&research_coverage_text(&candidate.label)))
        .map(|candidate| candidate.label.as_str())
        .collect();

    if missing.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "research output omits {} ranked candidate(s): {}",
        missing.len(),
        missing.join(", ")
    )]
}

fn research_coverage_text(value: &str) -> String {
    let normalized: String = value
        .to_lowercase()
        .nfd()
        .filter(|ch| !is_combining_mark(*ch))
        .map(|ch| {
            if ch.is_alphabetic() || ch.is_numeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn segment_narrative_reasons(result: &ResultInput, plan: &PlanInput) -> Vec<String> {
    if plan.segments.is_empty() {
        return Vec::new();
    }

    let scenes = &result.output.specscene.scenes;
    if scenes.len() != plan.segments.len() {
        return vec![format!(
            "explicit segment scene count mismatch: got {}, want {}",
            scenes.len(),
            plan.segments.len()
        )];
    }

    let by_segment: HashMap<&str, &SceneInput> = scenes
        .iter()
        .filter(|scene| !scene.segment_id.trim().is_empty())
        .map(|scene| (scene.segment_id.trim(), scene))
        .collect();

    let mut reasons = Vec::new();
    for (index, segment) in plan.segments.iter().enumerate() {
        let scene = by_segment
            .get(segment.id.trim())
            .copied()
            .unwrap_or(&scenes[index]);
        let words = scene.text.split_whitespace().count();

        if words == 0 {
            reasons.push(format!("segment \"{}\" is empty", segment.id));
        }
        if plan.clip_evidence.is_none() && words < 100 {
            reasons.push(format!(
                "segment \"{}\" contains fewer than 100 words",
                segment.id
            ));
        }

        if !segment.id.trim().to_lowercase().starts_with("boxer-") {
            continue;
        }

        let body = scene
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let topic = segment.topic.trim().to_lowercase();
        if !topic.is_empty() && !body.contains(&topic) {
            reasons.push(format!(
                "segment \"{}\" does not mention its declared topic \"{}\"",
                segment.id, segment.topic
            ));
        }
        for other in &plan.segments {
            if other.id == segment.id || !other.id.trim().to_lowercase().starts_with("boxer-") {
                continue;
            }
            let other_topic = other.topic.trim().to_lowercase();
            if !other_topic.is_empty() && body.contains(&other_topic) {
                reasons.push(format!(
                    "segment \"{}\" contains the declared topic of \"{}\"",
                    segment.id, other.id
                ));
            }
        }
    }
    reasons
}

fn target_words_reasons(quality: &Quality, plan: &PlanInput, source_text: &str) -> Vec<String> {
    if plan.clip_evidence.is_some() {
        return Vec::new();
    }
    if plan.target_words > 0
        && !source_text.trim().is_empty()
        && plan.translate_to.trim().is_empty()
    {
        let lower = f64::from(plan.target_words) * MIN_TARGET_WORDS_RATIO;
        let upper = f64::from(plan.target_words) * MAX_TARGET_WORDS_RATIO;
        let actual = f64::from(quality.actual_words);
        if actual < lower || actual > upper {
            return vec!["actual word count outside target tolerance".to_string()];
        }
    }
    Vec::new()
}
