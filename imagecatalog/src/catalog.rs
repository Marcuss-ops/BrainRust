//! In-memory index over the existing catalog snapshot.
//!
//! The rules are a Rust projection of the Go catalog policy:
//! `entitycatalog.IsUsableCandidateQuality`, `ClassifyCandidateStatus`,
//! `AssessCandidateState` and the materialization reuse check. They are
//! duplicated only as behavior, never as a second persistent catalog.
//!
//! The optional semantic sidecar adds a second, separate check: if an
//! annotation marks the catalog identity as a mismatch, or describes a
//! synthetic/illustration image, the candidate is not usable for a PERSON
//! identity query. Descriptions never override the existing certification;
//! they make the certification auditable and expose what is actually visible.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::clock::classify_candidate_state;
use crate::types::{
    CatalogAnnotation, CatalogCandidate, CatalogEntity, CatalogMaterialization, CatalogSnapshot,
};

/// Minimum semantic score when the catalog recorded one.
pub const MIN_SEMANTIC_SCORE: f64 = 0.60;
/// Minimum technical score when the catalog recorded one.
pub const MIN_TECHNICAL_SCORE: f64 = 0.50;
/// Minimum long side for a usable catalog image.
pub const MIN_LONG_SIDE: u32 = 800;
/// Minimum pixel count for a usable catalog image.
pub const MIN_PIXELS: u64 = 400_000;
/// Ranking penalty for reusing an asset already selected in this script.
pub const REUSE_PENALTY: f64 = 0.20;
/// A composite is less suitable than a single-subject portrait for a PERSON
/// identity query. It stays eligible, but ranks below an otherwise comparable
/// portrait.
pub const COMPOSITE_PENALTY: f64 = 0.08;
/// A visual description whose identity was not verified is still usable, but
/// ranks below one that was reviewed.
pub const UNVERIFIED_IDENTITY_PENALTY: f64 = 0.05;
/// Bonus for each additional visible identity that also appears in the scene.
pub const CONTEXT_SUBJECT_BONUS: f64 = 0.07;
/// Cap on the context-subject bonus, so a large group shot cannot dominate a
/// scene solely by listing many identities.
pub const MAX_CONTEXT_SUBJECT_BONUS: f64 = 0.10;

/// A normalized, deterministic view of a catalog snapshot.
#[derive(Debug, Clone)]
pub struct CatalogIndex {
    entities: HashMap<String, CatalogEntity>,
    candidates: HashMap<String, Vec<CatalogCandidate>>,
    materializations: HashMap<i64, CatalogMaterialization>,
    annotations: HashMap<i64, CatalogAnnotation>,
    fingerprint: String,
}

/// One candidate after the state, quality, materialization and annotation
/// gates.
#[derive(Debug, Clone, PartialEq)]
pub struct AssessedCandidate {
    pub candidate: CatalogCandidate,
    /// Effective state: `fresh`, `stale`, `broken` or `retired`.
    pub state: String,
    pub eligible: bool,
    /// Deterministic ranking score; 0.0 when not eligible.
    pub score: f64,
    pub reason: String,
    pub materialization: Option<CatalogMaterialization>,
    pub annotation: Option<CatalogAnnotation>,
    /// A verified durable materialization can be rendered without the remote
    /// URL and survives a `broken`/`retired` remote state.
    pub durable: bool,
    /// Already selected by an earlier scene in this script.
    pub reused: bool,
}

impl CatalogIndex {
    /// Builds the index from the existing rows. Orphaned rows are ignored;
    /// identical duplicates are deduplicated; contradictory duplicates are
    /// rejected so the result cannot depend on export row order. The snapshot
    /// is sorted before hashing, so a database export row order cannot change
    /// the fingerprint.
    pub fn new(snapshot: &CatalogSnapshot) -> Result<Self, String> {
        let mut entities: BTreeMap<String, CatalogEntity> = BTreeMap::new();
        for entity in &snapshot.entities {
            let id = entity.canonical_entity_id.trim();
            if id.is_empty() {
                continue;
            }
            if let Some(existing) = entities.get(id) {
                if existing != entity {
                    return Err(format!("contradictory duplicate catalog entity: {id}"));
                }
            } else {
                entities.insert(id.to_string(), entity.clone());
            }
        }

        let mut candidates: BTreeMap<i64, CatalogCandidate> = BTreeMap::new();
        for candidate in &snapshot.candidates {
            let canonical_entity_id = candidate.canonical_entity_id.trim();
            if !entities.contains_key(canonical_entity_id) {
                continue;
            }
            let mut candidate = candidate.clone();
            candidate.canonical_entity_id = canonical_entity_id.to_string();
            if let Some(existing) = candidates.get(&candidate.candidate_id) {
                if existing != &candidate {
                    return Err(format!(
                        "contradictory duplicate catalog candidate: {}",
                        candidate.candidate_id
                    ));
                }
            } else {
                candidates.insert(candidate.candidate_id, candidate);
            }
        }

        let mut materializations: BTreeMap<i64, CatalogMaterialization> = BTreeMap::new();
        for materialization in &snapshot.materializations {
            if let Some(existing) = materializations.get(&materialization.candidate_id) {
                if existing != materialization {
                    return Err(format!(
                        "contradictory duplicate catalog materialization: {}",
                        materialization.candidate_id
                    ));
                }
            } else {
                materializations.insert(materialization.candidate_id, materialization.clone());
            }
        }

        let mut annotations: BTreeMap<i64, CatalogAnnotation> = BTreeMap::new();
        for annotation in &snapshot.annotations {
            if !candidates.contains_key(&annotation.candidate_id) {
                continue;
            }
            let mut normalized = annotation.clone();
            normalized.description = normalized.description.trim().to_string();
            normalized.subjects = normalized_strings(&normalized.subjects);
            normalized.tags = normalized_strings(&normalized.tags);
            normalized.depiction = normalized.depiction.trim().to_lowercase();
            normalized.identity_status = normalized.identity_status.trim().to_lowercase();
            normalized.review_status = normalized.review_status.trim().to_lowercase();
            normalized.source = normalized.source.trim().to_string();
            normalized.evidence = normalized.evidence.trim().to_string();
            // A later annotation replaces an earlier one for the same
            // candidate, which is how the exporter applies a curated overlay
            // on top of automatic metadata.
            annotations.insert(annotation.candidate_id, normalized);
        }

        let normalized = CatalogSnapshot {
            entities: entities.values().cloned().collect(),
            candidates: candidates.values().cloned().collect(),
            materializations: materializations.values().cloned().collect(),
            annotations: annotations.values().cloned().collect(),
        };
        let encoded = serde_json::to_vec(&normalized).unwrap_or_default();
        let fingerprint = fnv1a_hex(&encoded);

        let mut by_entity: HashMap<String, Vec<CatalogCandidate>> = HashMap::new();
        for candidate in candidates.into_values() {
            by_entity
                .entry(candidate.canonical_entity_id.clone())
                .or_default()
                .push(candidate);
        }
        for rows in by_entity.values_mut() {
            rows.sort_by_key(|candidate| candidate.candidate_id);
        }

        Ok(Self {
            entities: entities.into_iter().collect(),
            candidates: by_entity,
            materializations: materializations.into_iter().collect(),
            annotations: annotations.into_iter().collect(),
            fingerprint,
        })
    }

    /// FNV-1a over the normalized snapshot, including semantic annotations.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn entity(&self, canonical_entity_id: &str) -> Option<&CatalogEntity> {
        self.entities.get(canonical_entity_id.trim())
    }

    pub fn annotation(&self, candidate_id: i64) -> Option<&CatalogAnnotation> {
        self.annotations.get(&candidate_id)
    }

    pub fn candidate_count(&self, canonical_entity_id: &str) -> usize {
        self.candidates
            .get(canonical_entity_id.trim())
            .map_or(0, Vec::len)
    }

    /// Assesses every candidate for one canonical identity without scene
    /// context. Eligible rows come first, then descending ranking score, then
    /// ascending candidate id for a deterministic replay.
    pub fn assess(
        &self,
        canonical_entity_id: &str,
        now: i64,
        used: &BTreeSet<i64>,
    ) -> Vec<AssessedCandidate> {
        self.assess_with_context(canonical_entity_id, now, used, &BTreeSet::new())
    }

    /// Assesses every candidate for one canonical identity. `context_subjects`
    /// are the other canonical identities present in the scene; a candidate
    /// annotated with one of them receives a bounded promotion. The primary
    /// identity is always required by the catalog lookup itself.
    pub fn assess_with_context(
        &self,
        canonical_entity_id: &str,
        now: i64,
        used: &BTreeSet<i64>,
        context_subjects: &BTreeSet<String>,
    ) -> Vec<AssessedCandidate> {
        let Some(rows) = self.candidates.get(canonical_entity_id.trim()) else {
            return Vec::new();
        };
        let mut assessed: Vec<AssessedCandidate> = rows
            .iter()
            .map(|candidate| self.assess_candidate(candidate, now, used, context_subjects))
            .collect();
        assessed.sort_by(|left, right| {
            right
                .eligible
                .cmp(&left.eligible)
                .then_with(|| {
                    right
                        .score
                        .partial_cmp(&left.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    left.candidate
                        .candidate_id
                        .cmp(&right.candidate.candidate_id)
                })
        });
        assessed
    }

    fn assess_candidate(
        &self,
        candidate: &CatalogCandidate,
        now: i64,
        used: &BTreeSet<i64>,
        context_subjects: &BTreeSet<String>,
    ) -> AssessedCandidate {
        let state = classify_candidate_state(now, &candidate.status, &candidate.last_seen_at);
        let materialization = self.materializations.get(&candidate.candidate_id).cloned();
        let annotation = self.annotations.get(&candidate.candidate_id).cloned();
        let (durable, materialization_problem) = materialization_status(materialization.as_ref());
        let annotation_problem = annotation_quality_problem(candidate, annotation.as_ref());
        let quality_problem = candidate_quality_problem(candidate, annotation.as_ref());
        let status_usable = matches!(state.as_str(), "fresh" | "stale");
        // A durable copy may keep a broken remote URL renderable. It does not
        // resurrect a retired row or an unrecognized status: those are policy
        // decisions that a materialization must never override.
        let durable_bypass = durable && state == "broken";
        let reused = used.contains(&candidate.candidate_id);

        let eligible = annotation_problem.is_none()
            && quality_problem.is_none()
            && (status_usable || durable_bypass);
        let score = if eligible {
            ranking_score(
                candidate,
                &state,
                durable,
                reused,
                annotation.as_ref(),
                context_subjects,
            )
        } else {
            0.0
        };

        let reason = if let Some(problem) = annotation_problem {
            format!("not eligible: {problem}")
        } else if let Some(problem) = quality_problem {
            format!("not eligible: {problem}")
        } else if !status_usable && durable_bypass {
            format!("eligible: {state} remote URL bypassed by a verified materialization")
        } else if !status_usable {
            match materialization_problem {
                Some(problem) => {
                    format!("not eligible: candidate status {state}; {problem}")
                }
                None => format!("not eligible: candidate status {state}"),
            }
        } else if durable {
            format!("eligible: {state} candidate with verified materialization")
        } else {
            let problem =
                materialization_problem.unwrap_or_else(|| "no materialization".to_string());
            format!("eligible: {state} candidate; {problem}")
        };
        let reason = if reused && eligible {
            format!("{reason}; already selected in this script")
        } else {
            reason
        };
        let reason = annotation_note(annotation.as_ref())
            .map(|note| format!("{reason}; annotation {note}"))
            .unwrap_or(reason);

        AssessedCandidate {
            candidate: candidate.clone(),
            state,
            eligible,
            score,
            reason,
            materialization,
            annotation,
            durable,
            reused,
        }
    }
}

/// Applies the existing semantic, technical and dimension gates. Missing or
/// legacy certification fails closed.
pub fn candidate_quality_problem(
    candidate: &CatalogCandidate,
    annotation: Option<&CatalogAnnotation>,
) -> Option<String> {
    if candidate.source_url.trim().is_empty() {
        return Some("source URL is empty".to_string());
    }
    if !candidate.semantic_score.is_finite() || !(0.0..=1.0).contains(&candidate.semantic_score) {
        return Some("semantic score outside [0, 1]".to_string());
    }
    if !candidate.technical_score.is_finite() || !(0.0..=1.0).contains(&candidate.technical_score) {
        return Some("technical score outside [0, 1]".to_string());
    }

    let semantic_status = candidate.semantic_status.trim().to_lowercase();
    match semantic_status.as_str() {
        "accepted" => {}
        "rejected" => return Some("semantic status rejected".to_string()),
        "" | "unknown" => {
            return Some("semantic status unknown (legacy row is not certified)".to_string())
        }
        other => return Some(format!("semantic status {other} is not accepted")),
    }
    let quality_reason = candidate.quality_reason.to_lowercase();
    let reviewed_identity = annotation.is_some_and(|annotation| {
        annotation.identity_status == "verified_visual"
            && annotation.review_status == "reviewed"
            && annotation
                .subjects
                .iter()
                .any(|subject| subject == &candidate.canonical_entity_id)
            && !annotation.source.trim().is_empty()
            && !annotation.evidence.trim().is_empty()
    });
    if !reviewed_identity
        && !quality_reason.contains("ollama visual identity verification confirmed")
        && !quality_reason.contains("manual visual identity review confirmed")
        && !quality_reason.contains("pinned by media_manifest.v1: offline sha256 verify")
    {
        return Some("accepted status has no visual identity-verification provenance".to_string());
    }
    if candidate.semantic_score > 0.0 && candidate.semantic_score < MIN_SEMANTIC_SCORE {
        return Some(format!(
            "semantic score {:.2} below {:.2}",
            candidate.semantic_score, MIN_SEMANTIC_SCORE
        ));
    }
    if candidate.technical_score > 0.0 && candidate.technical_score < MIN_TECHNICAL_SCORE {
        return Some(format!(
            "technical score {:.2} below {:.2}",
            candidate.technical_score, MIN_TECHNICAL_SCORE
        ));
    }
    if candidate.width > 0 && candidate.height > 0 {
        let long_side = candidate.width.max(candidate.height);
        let pixels = u64::from(candidate.width) * u64::from(candidate.height);
        if long_side < MIN_LONG_SIDE || pixels < MIN_PIXELS {
            return Some(format!(
                "dimensions {}x{} below {}px long side / {}px",
                candidate.width, candidate.height, MIN_LONG_SIDE, MIN_PIXELS
            ));
        }
    }
    None
}

/// Applies the semantic sidecar's identity gates. The existing catalog
/// certification remains authoritative for eligibility; this function only
/// adds a fail-closed check when the annotation says the catalog identity is
/// wrong or the image is not a documentary identity image.
pub fn annotation_quality_problem(
    candidate: &CatalogCandidate,
    annotation: Option<&CatalogAnnotation>,
) -> Option<String> {
    let annotation = annotation?;
    let identity_status = annotation.identity_status.as_str();
    match identity_status {
        "mismatch" => {
            return Some("annotation marks the catalog identity as a mismatch".to_string())
        }
        "" | "verified_visual" | "catalog_claimed" | "unverified" => {}
        other => {
            return Some(format!(
                "annotation identity status {other} is not recognized"
            ))
        }
    }

    let depiction = annotation.depiction.as_str();
    match depiction {
        "" | "unknown" | "photograph" | "composite" => {}
        "illustration" | "synthetic" | "generated" => {
            return Some(format!(
                "annotation depicts {depiction} content, not a documentary identity image"
            ))
        }
        other => return Some(format!("annotation depiction {other} is not recognized")),
    }

    match annotation.review_status.as_str() {
        "" | "reviewed" | "unverified" | "pending" => {}
        other => {
            return Some(format!(
                "annotation review status {other} is not recognized"
            ))
        }
    }

    let primary = candidate.canonical_entity_id.trim();
    if !annotation.subjects.is_empty()
        && !annotation.subjects.iter().any(|subject| subject == primary)
    {
        return Some(format!(
            "annotation subjects do not include the catalog identity {primary}"
        ));
    }
    None
}

/// A verified materialization requires the same fields as the Go owner:
/// `materialized` status, asset id, Drive link and a SHA-256 content address.
fn materialization_status(
    materialization: Option<&CatalogMaterialization>,
) -> (bool, Option<String>) {
    let Some(materialization) = materialization else {
        return (false, None);
    };
    if !materialization
        .status
        .trim()
        .eq_ignore_ascii_case("materialized")
    {
        return (
            false,
            Some(format!(
                "materialization status {}",
                materialization.status.trim()
            )),
        );
    }
    if materialization.asset_id.trim().is_empty() {
        return (false, Some("materialization has no asset id".to_string()));
    }
    if materialization.drive_link.trim().is_empty() {
        return (false, Some("materialization has no Drive link".to_string()));
    }
    if materialization.content_sha256.trim().is_empty() {
        return (
            false,
            Some("materialization has no content SHA-256".to_string()),
        );
    }
    if !is_sha256(&materialization.content_sha256) {
        return (
            false,
            Some("materialization content hash is not a SHA-256".to_string()),
        );
    }
    (true, None)
}

/// Deterministic ranking score. It is not a calibrated probability.
fn ranking_score(
    candidate: &CatalogCandidate,
    state: &str,
    durable: bool,
    reused: bool,
    annotation: Option<&CatalogAnnotation>,
    context_subjects: &BTreeSet<String>,
) -> f64 {
    let quality = 0.6 * candidate.semantic_score + 0.4 * candidate.technical_score;
    let freshness = match state {
        "fresh" => 0.05,
        "stale" => -0.02,
        _ => 0.0,
    };
    let rank_bonus = 0.05 / f64::from(candidate.rank.max(1));
    let durable_bonus = if durable { 0.10 } else { 0.0 };
    let reuse_penalty = if reused { REUSE_PENALTY } else { 0.0 };
    let composite_penalty = annotation
        .filter(|annotation| annotation.depiction == "composite")
        .map_or(0.0, |_| COMPOSITE_PENALTY);
    let unverified_penalty = annotation
        .filter(|annotation| annotation.identity_status == "unverified")
        .map_or(0.0, |_| UNVERIFIED_IDENTITY_PENALTY);
    let context_bonus = annotation.map_or(0.0, |annotation| {
        let extra_subjects = annotation
            .subjects
            .iter()
            .filter(|subject| {
                subject.as_str() != candidate.canonical_entity_id
                    && context_subjects.contains(subject.as_str())
            })
            .count();
        (extra_subjects as f64 * CONTEXT_SUBJECT_BONUS).min(MAX_CONTEXT_SUBJECT_BONUS)
    });

    quality + freshness + rank_bonus + durable_bonus + context_bonus
        - reuse_penalty
        - composite_penalty
        - unverified_penalty
}

fn annotation_note(annotation: Option<&CatalogAnnotation>) -> Option<String> {
    let annotation = annotation?;
    let mut parts = Vec::new();
    if !annotation.identity_status.is_empty() {
        parts.push(annotation.identity_status.clone());
    }
    if !annotation.depiction.is_empty() {
        parts.push(annotation.depiction.clone());
    }
    if !annotation.subjects.is_empty() {
        parts.push(format!("{} subject(s)", annotation.subjects.len()));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn normalized_strings(values: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn is_sha256(value: &str) -> bool {
    let value = value.trim();
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn fnv1a_hex(body: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_utc_timestamp;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn candidate(id: i64, status: &str, semantic_status: &str) -> CatalogCandidate {
        CatalogCandidate {
            candidate_id: id,
            canonical_entity_id: "person:alberto-stasi".to_string(),
            provider: "duckduckgo".to_string(),
            rank: id as u32,
            source_url: format!("https://example.com/{id}.jpg"),
            source_page_url: String::new(),
            thumbnail_url: String::new(),
            width: 1200,
            height: 800,
            status: status.to_string(),
            semantic_status: semantic_status.to_string(),
            semantic_score: 0.92,
            technical_score: 0.88,
            quality_reason: if semantic_status == "accepted" {
                "Ollama visual identity verification confirmed the requested person and one dominant subject".to_string()
            } else {
                String::new()
            },
            last_seen_at: "2026-11-09T00:00:00Z".to_string(),
        }
    }

    fn snapshot(
        candidates: Vec<CatalogCandidate>,
        materializations: Vec<CatalogMaterialization>,
    ) -> CatalogSnapshot {
        CatalogSnapshot {
            entities: vec![CatalogEntity {
                canonical_entity_id: "person:alberto-stasi".to_string(),
                entity_type: "PERSON".to_string(),
                canonical_name: "Alberto Stasi".to_string(),
            }],
            candidates,
            materializations,
            annotations: Vec::new(),
        }
    }

    fn annotation(
        candidate_id: i64,
        identity_status: &str,
        depiction: &str,
        subjects: &[&str],
    ) -> CatalogAnnotation {
        CatalogAnnotation {
            candidate_id,
            description: "a person in a room".to_string(),
            subjects: subjects.iter().map(|subject| subject.to_string()).collect(),
            tags: vec!["room".to_string()],
            depiction: depiction.to_string(),
            identity_status: identity_status.to_string(),
            review_status: "reviewed".to_string(),
            source: "test".to_string(),
            evidence: String::new(),
        }
    }

    fn materialization(candidate_id: i64) -> CatalogMaterialization {
        CatalogMaterialization {
            candidate_id,
            asset_id: format!("asset-{candidate_id}"),
            content_sha256: SHA.to_string(),
            drive_file_id: format!("drive-{candidate_id}"),
            drive_link: format!("https://drive.example/{candidate_id}"),
            status: "materialized".to_string(),
            materialized_at: String::new(),
            last_verified_at: String::new(),
        }
    }

    fn now() -> i64 {
        parse_utc_timestamp("2026-11-10T00:00:00Z").unwrap()
    }

    fn build_index(snapshot: &CatalogSnapshot) -> CatalogIndex {
        CatalogIndex::new(snapshot).expect("test catalog snapshot must be consistent")
    }

    #[test]
    fn accepted_fresh_candidate_is_eligible_and_durable_one_wins() {
        let index = build_index(&snapshot(
            vec![
                candidate(1, "fresh", "accepted"),
                candidate(2, "fresh", "accepted"),
            ],
            vec![materialization(2)],
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(assessed[0].eligible);
        assert_eq!(assessed[0].candidate.candidate_id, 2);
        assert!(assessed[0].durable);
        assert!(assessed[1].eligible);
    }

    #[test]
    fn unknown_or_rejected_semantic_status_fails_closed() {
        for status in ["unknown", "rejected", ""] {
            let index = build_index(&snapshot(vec![candidate(1, "fresh", status)], Vec::new()));
            let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
            assert!(!assessed[0].eligible, "status {status:?} must fail closed");
        }
    }

    #[test]
    fn technical_and_dimension_gates_reject_bad_rows() {
        let mut low_technical = candidate(1, "fresh", "accepted");
        low_technical.technical_score = 0.30;
        let index = build_index(&snapshot(vec![low_technical], Vec::new()));
        assert!(!index.assess("person:alberto-stasi", now(), &BTreeSet::new())[0].eligible);

        let mut small = candidate(2, "fresh", "accepted");
        small.width = 640;
        small.height = 480;
        let index = build_index(&snapshot(vec![small], Vec::new()));
        assert!(!index.assess("person:alberto-stasi", now(), &BTreeSet::new())[0].eligible);
    }

    #[test]
    fn broken_url_is_rejected_unless_a_verified_materialization_exists() {
        let index = build_index(&snapshot(
            vec![candidate(1, "broken", "accepted")],
            Vec::new(),
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(!assessed[0].eligible);

        let index = build_index(&snapshot(
            vec![candidate(1, "broken", "accepted")],
            vec![materialization(1)],
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(assessed[0].eligible);
        assert!(assessed[0].durable);
        assert!(assessed[0].reason.contains("bypassed"));
    }

    #[test]
    fn a_materialization_without_a_sha256_is_not_reusable() {
        let mut bad = materialization(1);
        bad.content_sha256 = "legacy-md5".to_string();
        let index = build_index(&snapshot(
            vec![candidate(1, "broken", "accepted")],
            vec![bad],
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(!assessed[0].eligible);
        assert!(!assessed[0].durable);
        assert!(assessed[0].reason.contains("SHA-256"));
    }

    #[test]
    fn reused_candidates_are_demoted_but_never_dropped() {
        let index = build_index(&snapshot(
            vec![
                candidate(1, "fresh", "accepted"),
                candidate(2, "fresh", "accepted"),
            ],
            Vec::new(),
        ));
        let mut used = BTreeSet::new();
        used.insert(1);
        let assessed = index.assess("person:alberto-stasi", now(), &used);
        assert_eq!(assessed[0].candidate.candidate_id, 2);
        assert!(assessed[1].eligible);
        assert!(assessed[1].reused);
        assert!(assessed[1].reason.contains("already selected"));
    }

    #[test]
    fn equal_scores_break_ties_by_candidate_id() {
        let index = build_index(&snapshot(
            vec![
                candidate(9, "fresh", "accepted"),
                candidate(3, "fresh", "accepted"),
            ],
            Vec::new(),
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert_eq!(assessed[0].candidate.candidate_id, 3);
        assert_eq!(assessed[1].candidate.candidate_id, 9);
    }

    #[test]
    fn fingerprint_is_independent_of_snapshot_vector_order() {
        let first = build_index(&snapshot(
            vec![
                candidate(1, "fresh", "accepted"),
                candidate(2, "fresh", "accepted"),
            ],
            vec![materialization(2)],
        ));
        let mut reversed = snapshot(
            vec![
                candidate(2, "fresh", "accepted"),
                candidate(1, "fresh", "accepted"),
            ],
            vec![materialization(2)],
        );
        reversed.entities.reverse();
        let second = build_index(&reversed);
        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn annotation_mismatch_rejects_an_otherwise_eligible_candidate() {
        let mut row = snapshot(vec![candidate(1, "fresh", "accepted")], Vec::new());
        row.annotations = vec![annotation(1, "mismatch", "photograph", &[])];
        let index = build_index(&row);
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(!assessed[0].eligible);
        assert!(assessed[0].reason.contains("mismatch"));
    }

    #[test]
    fn annotation_subjects_must_include_the_catalog_identity() {
        let mut row = snapshot(vec![candidate(1, "fresh", "accepted")], Vec::new());
        row.annotations = vec![annotation(
            1,
            "verified_visual",
            "photograph",
            &["person:andrea-sempio"],
        )];
        let index = build_index(&row);
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(!assessed[0].eligible);
        assert!(assessed[0].reason.contains("do not include"));
    }

    #[test]
    fn synthetic_or_illustration_depictions_are_not_identity_images() {
        for depiction in ["synthetic", "illustration", "generated"] {
            let mut row = snapshot(vec![candidate(1, "fresh", "accepted")], Vec::new());
            row.annotations = vec![annotation(
                1,
                "verified_visual",
                depiction,
                &["person:alberto-stasi"],
            )];
            let index = build_index(&row);
            let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
            assert!(
                !assessed[0].eligible,
                "depiction {depiction:?} must not be selected as an identity image"
            );
            assert!(assessed[0].reason.contains("not a documentary"));
        }
    }

    #[test]
    fn a_composite_annotation_is_demoted_below_a_single_portrait() {
        let mut single = candidate(1, "fresh", "accepted");
        single.semantic_score = 0.95;
        single.rank = 2;
        let mut composite = candidate(2, "fresh", "accepted");
        composite.semantic_score = 1.0;
        composite.rank = 1;
        let mut row = snapshot(vec![single, composite], Vec::new());
        row.annotations = vec![annotation(
            2,
            "verified_visual",
            "composite",
            &["person:alberto-stasi"],
        )];
        let index = build_index(&row);
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(assessed[0].eligible);
        assert_eq!(assessed[0].candidate.candidate_id, 1);
        assert!(assessed[1].score < assessed[0].score);
    }

    #[test]
    fn an_extra_scene_identity_promotes_an_annotated_candidate() {
        let mut without_context = candidate(1, "fresh", "accepted");
        without_context.semantic_score = 0.90;
        without_context.rank = 2;
        let mut with_context = candidate(2, "fresh", "accepted");
        with_context.semantic_score = 0.92;
        with_context.rank = 1;
        let mut row = snapshot(vec![without_context, with_context], Vec::new());
        row.annotations = vec![annotation(
            1,
            "verified_visual",
            "photograph",
            &["person:alberto-stasi", "person:andrea-sempio"],
        )];
        let index = build_index(&row);
        let mut context = BTreeSet::new();
        context.insert("person:andrea-sempio".to_string());
        let assessed =
            index.assess_with_context("person:alberto-stasi", now(), &BTreeSet::new(), &context);
        assert_eq!(assessed[0].candidate.candidate_id, 1);
        assert!(assessed[0].score > assessed[1].score);
    }

    #[test]
    fn fingerprint_changes_when_an_annotation_changes() {
        let mut first = snapshot(vec![candidate(1, "fresh", "accepted")], Vec::new());
        let mut second = first.clone();
        first.annotations = vec![annotation(
            1,
            "verified_visual",
            "photograph",
            &["person:alberto-stasi"],
        )];
        second.annotations = vec![annotation(
            1,
            "verified_visual",
            "composite",
            &["person:alberto-stasi"],
        )];
        assert_ne!(
            build_index(&first).fingerprint(),
            build_index(&second).fingerprint()
        );
    }

    #[test]
    fn a_retired_candidate_is_never_selectable_even_with_a_materialization() {
        let index = build_index(&snapshot(
            vec![candidate(1, "retired", "accepted")],
            vec![materialization(1)],
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert!(!assessed[0].eligible);
        assert!(assessed[0].durable);
        assert!(assessed[0].reason.contains("retired"));
    }

    #[test]
    fn an_unrecognized_status_is_not_treated_as_fresh() {
        let index = build_index(&snapshot(
            vec![candidate(1, "pending", "accepted")],
            Vec::new(),
        ));
        let assessed = index.assess("person:alberto-stasi", now(), &BTreeSet::new());
        assert_eq!(assessed[0].state, "unknown");
        assert!(!assessed[0].eligible);
        assert!(assessed[0].reason.contains("unknown"));
    }

    #[test]
    fn contradictory_duplicate_candidates_are_rejected() {
        let first = candidate(1, "fresh", "accepted");
        let mut second = first.clone();
        second.source_url = "https://example.com/different.jpg".to_string();
        let error = CatalogIndex::new(&snapshot(vec![first, second], Vec::new()))
            .expect_err("contradictory duplicate must be rejected");
        assert!(error.contains("candidate"));
    }

    #[test]
    fn identical_duplicate_candidates_are_deduplicated() {
        let first = candidate(1, "fresh", "accepted");
        let index = build_index(&snapshot(vec![first.clone(), first], Vec::new()));
        assert_eq!(index.candidate_count("person:alberto-stasi"), 1);
    }
}
