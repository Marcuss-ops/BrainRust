//! Text -> grounded semantic queries -> existing catalog -> deterministic plan.

use std::collections::{BTreeSet, HashMap};

use semantic_core::{
    Candidate, CanonicalIdentity, SceneInput, ScriptRequest, SemanticManifest, SCHEMA_VERSION,
};

use crate::canonical::canonical_person_id;
use crate::catalog::{AssessedCandidate, CatalogIndex};
use crate::clock::parse_utc_timestamp;
use crate::types::{
    ImageCandidateAssessment, ImageCatalogRequest, ImageDecision, ImagePlan, ImagePlanCounts,
    ImageQueryPlan, ImageSelection, SceneImagePlan, PLAN_SCHEMA,
};

/// Maximum assessed candidates carried per query. Selection still considers
/// every row; the cap only keeps the response auditable and bounded.
pub const MAX_ASSESSED_CANDIDATES: usize = 10;

/// Runs the full pipeline for one worker request.
pub fn plan(request: &ImageCatalogRequest) -> Result<ImagePlan, String> {
    if request.language.trim().is_empty() {
        return Err("language is required".to_string());
    }
    if request.scenes.is_empty() {
        return Err("scenes must not be empty".to_string());
    }
    let now = match request.now.as_deref() {
        Some(value) => parse_utc_timestamp(value)
            .ok_or_else(|| format!("now is not a UTC timestamp: {value}"))?,
        None => system_now_seconds(),
    };
    let catalog = CatalogIndex::new(&request.catalog)?;
    if let Some(manifest) = &request.manifest {
        validate_reused_manifest(request, manifest)?;
        return Ok(plan_from_manifest(manifest, &catalog, now));
    }
    let manifest = semantic_engine::analyze_script(&ScriptRequest {
        language: request.language.clone(),
        scenes: request.scenes.clone(),
        editorial: None,
        entity_safety_cap: 0,
    })?;
    Ok(plan_from_manifest(&manifest, &catalog, now))
}

/// Convenience entry point for callers that already have a catalog snapshot.
pub fn plan_text(
    language: &str,
    scenes: Vec<SceneInput>,
    catalog: &crate::types::CatalogSnapshot,
    now: Option<&str>,
) -> Result<ImagePlan, String> {
    plan(&ImageCatalogRequest {
        version: Some(crate::types::REQUEST_SCHEMA.to_string()),
        operation: Some("plan".to_string()),
        language: language.to_string(),
        scenes,
        catalog: catalog.clone(),
        now: now.map(str::to_string),
        manifest: None,
    })
}

/// Reuses a manifest only when it provably belongs to this request. The
/// fingerprint covers the language, scenes, editorial options, safety cap and
/// analyzer versions; the image catalog has no editorial/safety-cap input, so
/// the accepted manifest must be the one produced from exactly these scenes
/// with those options absent.
fn validate_reused_manifest(
    request: &ImageCatalogRequest,
    manifest: &SemanticManifest,
) -> Result<(), String> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version must be {SCHEMA_VERSION}, got {}",
            manifest.schema_version
        ));
    }
    if manifest.language != request.language {
        return Err(format!(
            "manifest language {} does not match request language {}",
            manifest.language, request.language
        ));
    }
    if manifest.scenes.len() != request.scenes.len() {
        return Err(format!(
            "manifest has {} scenes, request has {}",
            manifest.scenes.len(),
            request.scenes.len()
        ));
    }
    for (index, (expected, actual)) in request.scenes.iter().zip(&manifest.scenes).enumerate() {
        if expected.scene_id != actual.scene_id {
            return Err(format!(
                "manifest scene {index} id {} does not match request id {}",
                actual.scene_id, expected.scene_id
            ));
        }
    }
    let expected = semantic_engine::request_fingerprint(
        &ScriptRequest {
            language: request.language.clone(),
            scenes: request.scenes.clone(),
            editorial: None,
            entity_safety_cap: 0,
        },
        &manifest.analyzers,
    );
    if manifest.fingerprint != expected {
        return Err(format!(
            "manifest fingerprint {} does not match request fingerprint {expected}",
            manifest.fingerprint
        ));
    }
    Ok(())
}

/// Builds the image plan from an already-analyzed semantic manifest.
pub fn plan_from_manifest(
    manifest: &SemanticManifest,
    catalog: &CatalogIndex,
    now: i64,
) -> ImagePlan {
    let mut used: BTreeSet<i64> = BTreeSet::new();
    let mut counts = ImagePlanCounts {
        scenes: manifest.scenes.len(),
        ..ImagePlanCounts::default()
    };
    let mut scenes = Vec::with_capacity(manifest.scenes.len());

    for scene in &manifest.scenes {
        if scene.status == "rejected" {
            counts.rejected_scenes += 1;
            scenes.push(SceneImagePlan {
                scene_id: scene.scene_id.clone(),
                queries: Vec::new(),
            });
            continue;
        }
        let identities: HashMap<&str, &CanonicalIdentity> = scene
            .identities
            .iter()
            .map(|identity| (identity.id.as_str(), identity))
            .collect();
        let entities: HashMap<&str, &Candidate> = scene
            .entities
            .iter()
            .map(|entity| (entity.id.as_str(), entity))
            .collect();
        // Other canonical identities in the same scene promote candidates
        // whose semantic annotation lists them too (for example a group shot
        // containing both the queried person and a co-defendant).
        let scene_subjects: BTreeSet<String> = scene
            .identities
            .iter()
            .filter(|identity| identity.kind == "PERSON")
            .map(|identity| canonical_person_id(&identity.canonical_surface))
            .filter(|identity| !identity.is_empty())
            .collect();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut queries = Vec::new();

        for proposal in scene
            .visual_proposals
            .iter()
            .filter(|proposal| proposal.proposal_type == "entity_search")
        {
            let query = proposal
                .visual_query_suggestions
                .first()
                .map(|query| query.trim().to_string())
                .unwrap_or_default();
            if query.is_empty() {
                continue;
            }
            if !seen.insert(("entity_search".to_string(), query.clone())) {
                continue;
            }
            let identity = proposal
                .entity_ids
                .first()
                .and_then(|id| identities.get(id.as_str()).copied());
            let entity = proposal
                .entity_ids
                .first()
                .and_then(|id| entities.get(id.as_str()).copied());
            let entity_type = identity
                .map(|identity| identity.kind.clone())
                .or_else(|| entity.map(|entity| entity.kind.clone()));
            let entity_surface = identity
                .map(|identity| identity.canonical_surface.clone())
                .or_else(|| entity.map(|entity| entity.text.clone()));

            let canonical_entity_id = entity_surface
                .as_deref()
                .filter(|_| entity_type.as_deref() == Some("PERSON"))
                .map(canonical_person_id)
                .filter(|id| !id.is_empty());

            let (decision, reason, selection, candidates) = resolve_catalog_entity(
                canonical_entity_id.as_deref(),
                entity_type.as_deref(),
                catalog,
                now,
                &mut used,
                &scene_subjects,
            );

            match decision {
                ImageDecision::Selected => {
                    counts.selected += 1;
                    if selection.as_ref().is_some_and(|selection| selection.reused) {
                        counts.reused_assets += 1;
                    }
                }
                ImageDecision::CatalogMiss => counts.catalog_misses += 1,
                ImageDecision::NotCatalogEligible => counts.not_catalog_eligible += 1,
            }

            queries.push(ImageQueryPlan {
                query,
                query_kind: "entity_search".to_string(),
                proposal_id: Some(proposal.id.clone()),
                entity_ids: proposal.entity_ids.clone(),
                canonical_entity_id,
                entity_surface,
                entity_type,
                evidence: proposal.evidence.clone(),
                decision,
                reason,
                selection,
                candidates,
            });
        }

        for query in &scene.contextual_image_queries {
            if !seen.insert(("contextual_image_query".to_string(), query.query.clone())) {
                continue;
            }
            counts.not_catalog_eligible += 1;
            queries.push(deferred_query(
                "contextual_image_query",
                query.query.clone(),
                query.evidence.clone(),
                "contextual subject+place retrieval is not an identity lookup; \
                 the current catalog stores PERSON identity images only",
            ));
        }

        for query in &scene.composed_image_queries {
            if !seen.insert(("composed_image_query".to_string(), query.query.clone())) {
                continue;
            }
            counts.not_catalog_eligible += 1;
            queries.push(deferred_query(
                "composed_image_query",
                query.query.clone(),
                query.query.clone(),
                "composed queries span several visual components; \
                 the current catalog stores PERSON identity images only",
            ));
        }

        counts.queries += queries.len();
        scenes.push(SceneImagePlan {
            scene_id: scene.scene_id.clone(),
            queries,
        });
    }

    ImagePlan {
        schema_version: PLAN_SCHEMA,
        language: manifest.language.clone(),
        fingerprint: manifest.fingerprint.clone(),
        catalog_fingerprint: catalog.fingerprint().to_string(),
        evaluated_at: now,
        scenes,
        counts,
    }
}

fn resolve_catalog_entity(
    canonical_entity_id: Option<&str>,
    entity_type: Option<&str>,
    catalog: &CatalogIndex,
    now: i64,
    used: &mut BTreeSet<i64>,
    context_subjects: &BTreeSet<String>,
) -> (
    ImageDecision,
    String,
    Option<ImageSelection>,
    Vec<ImageCandidateAssessment>,
) {
    let Some(entity_type) = entity_type else {
        return (
            ImageDecision::NotCatalogEligible,
            "proposal carries no resolvable entity".to_string(),
            None,
            Vec::new(),
        );
    };
    if entity_type != "PERSON" {
        return (
            ImageDecision::NotCatalogEligible,
            format!("entity type {entity_type} is not covered by the PERSON image catalog"),
            None,
            Vec::new(),
        );
    }
    let Some(canonical_entity_id) = canonical_entity_id else {
        return (
            ImageDecision::NotCatalogEligible,
            "PERSON surface cannot produce a canonical entity id".to_string(),
            None,
            Vec::new(),
        );
    };
    if catalog.entity(canonical_entity_id).is_none() {
        return (
            ImageDecision::CatalogMiss,
            format!("no catalog entity for {canonical_entity_id}"),
            None,
            Vec::new(),
        );
    }

    let assessed = catalog.assess_with_context(canonical_entity_id, now, used, context_subjects);
    let selected_index = assessed.iter().position(|candidate| candidate.eligible);
    let candidates: Vec<ImageCandidateAssessment> = assessed
        .iter()
        .take(MAX_ASSESSED_CANDIDATES)
        .enumerate()
        .map(|(index, candidate)| assessment(candidate, selected_index == Some(index)))
        .collect();

    match selected_index {
        Some(index) => {
            let selected = &assessed[index];
            used.insert(selected.candidate.candidate_id);
            (
                ImageDecision::Selected,
                selected.reason.clone(),
                Some(selection(selected)),
                candidates,
            )
        }
        None => {
            let reason = assessed
                .first()
                .map(|candidate| candidate.reason.clone())
                .unwrap_or_else(|| format!("no catalog candidates for {canonical_entity_id}"));
            (ImageDecision::CatalogMiss, reason, None, candidates)
        }
    }
}

fn assessment(candidate: &AssessedCandidate, selected: bool) -> ImageCandidateAssessment {
    ImageCandidateAssessment {
        candidate_id: candidate.candidate.candidate_id,
        state: candidate.state.clone(),
        eligible: candidate.eligible,
        selected,
        score: candidate.score,
        reason: candidate.reason.clone(),
        content_sha256: if candidate.durable {
            candidate
                .materialization
                .as_ref()
                .map(|materialization| materialization.content_sha256.trim().to_string())
                .filter(|hash| !hash.is_empty())
        } else {
            None
        },
        description: candidate
            .annotation
            .as_ref()
            .map(|annotation| annotation.description.trim().to_string())
            .filter(|description| !description.is_empty()),
        identity_status: candidate
            .annotation
            .as_ref()
            .map(|annotation| annotation.identity_status.trim().to_string())
            .filter(|status| !status.is_empty()),
    }
}

fn selection(candidate: &AssessedCandidate) -> ImageSelection {
    let materialization = if candidate.durable {
        candidate.materialization.as_ref()
    } else {
        None
    };
    let annotation = candidate.annotation.as_ref();
    ImageSelection {
        candidate_id: candidate.candidate.candidate_id,
        asset_id: materialization
            .map(|materialization| materialization.asset_id.trim().to_string())
            .filter(|value| !value.is_empty()),
        content_sha256: materialization
            .map(|materialization| materialization.content_sha256.trim().to_string())
            .filter(|value| !value.is_empty()),
        drive_link: materialization
            .map(|materialization| materialization.drive_link.trim().to_string())
            .filter(|value| !value.is_empty()),
        materialized: candidate.durable,
        source_url: candidate.candidate.source_url.trim().to_string(),
        provider: candidate.candidate.provider.trim().to_string(),
        provider_rank: candidate.candidate.rank.max(1),
        width: candidate.candidate.width,
        height: candidate.candidate.height,
        state: candidate.state.clone(),
        semantic_score: candidate.candidate.semantic_score,
        technical_score: candidate.candidate.technical_score,
        score: candidate.score,
        reused: candidate.reused,
        description: annotation
            .map(|annotation| annotation.description.trim().to_string())
            .filter(|description| !description.is_empty()),
        subjects: annotation
            .map(|annotation| annotation.subjects.clone())
            .unwrap_or_default(),
        tags: annotation
            .map(|annotation| annotation.tags.clone())
            .unwrap_or_default(),
        depiction: annotation
            .map(|annotation| annotation.depiction.trim().to_string())
            .filter(|depiction| !depiction.is_empty()),
        identity_status: annotation
            .map(|annotation| annotation.identity_status.trim().to_string())
            .filter(|status| !status.is_empty()),
        review_status: annotation
            .map(|annotation| annotation.review_status.trim().to_string())
            .filter(|status| !status.is_empty()),
    }
}

fn deferred_query(kind: &str, query: String, evidence: String, reason: &str) -> ImageQueryPlan {
    ImageQueryPlan {
        query,
        query_kind: kind.to_string(),
        proposal_id: None,
        entity_ids: Vec::new(),
        canonical_entity_id: None,
        entity_surface: None,
        entity_type: None,
        evidence,
        decision: ImageDecision::NotCatalogEligible,
        reason: reason.to_string(),
        selection: None,
        candidates: Vec::new(),
    }
}

fn system_now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
