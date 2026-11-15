//! Versioned request envelope for the 13d single-call overlay-plan cutover.
//!
//! This is the first contract slice: plan identity, language, canvas toggles,
//! timed scenes, the editorial manifest and the chapter manifest. Later phases
//! add entity/card/admission/seal inputs to the same envelope; the version is
//! bumped only for a breaking change. Validation is fail-closed and runs before
//! any decision is projected.
//!
//! The envelope deliberately carries the *inputs* the Go planner currently
//! feeds to the four interleaved phases. The final single call will consume a
//! complete request of this shape and return the sealed plan, so the wire
//! contract can be reviewed and parity-tested before the planner port lands.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Schema version of the full-plan request/response envelope.
pub const EDITORIAL_PLAN_V2_SCHEMA_VERSION: &str = "editorial_plan.v2";
/// Version of the per-scene editorial manifest carried by the request.
pub const EDITORIAL_MANIFEST_SCHEMA_VERSION: &str = "editorial.v1";
/// Version of the chapter manifest carried by the request.
pub const CHAPTER_MANIFEST_SCHEMA_VERSION: &str = "chapter_manifest.v1";
/// Certified editorial profile the projection accepts.
pub const EDITORIAL_PROFILE_VERSION: &str = "highlights.v1";

const OVERLAY_CATEGORIES: [&str; 10] = [
    "phrases",
    "images",
    "numbers",
    "quotes",
    "brand_text",
    "maps",
    "charts",
    "recaps",
    "titles",
    "bullets",
];

/// One complete overlay-plan compilation request. Unknown fields are ignored
/// by serde so additive phases stay compatible; a missing or unknown
/// `schema_version` is rejected by [`EditorialPlanV2Request::validate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialPlanV2Request {
    pub schema_version: String,
    pub plan: PlanIdentityV2,
    pub language: String,
    #[serde(default)]
    pub source_language: String,
    #[serde(default)]
    pub canvas: CanvasV2,
    #[serde(default)]
    pub scenes: Vec<SceneV2>,
    #[serde(default)]
    pub editorial: Option<EditorialManifestV2>,
    #[serde(default)]
    pub chapters: Option<ChapterManifestV2>,
}

/// Immutable identity of one compiled plan. Motion entropy is derived from
/// this identity by the planner, never from wall-clock state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanIdentityV2 {
    pub plan_id: String,
    pub video_id: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub motion_seed: String,
    #[serde(default)]
    pub coherence_seed: String,
    pub width: i64,
    pub height: i64,
    pub fps_num: i64,
    pub fps_den: i64,
}

/// Run-level canvas controls that change overlay selection. Rendering metadata
/// (background, animation directives, opaque style payloads) stays opaque and
/// is intentionally not re-derived here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CanvasV2 {
    #[serde(default)]
    pub overlay_categories: BTreeMap<String, String>,
    #[serde(default)]
    pub max_overlays_per_scene: i64,
    #[serde(default)]
    pub max_phrase_overlays: i64,
    #[serde(default)]
    pub max_image_overlays: i64,
    #[serde(default)]
    pub max_map_overlays: i64,
    #[serde(default)]
    pub disable_editorial_overlays: bool,
    #[serde(default)]
    pub disable_recap_overlays: bool,
    #[serde(default)]
    pub enable_charts: bool,
    #[serde(default)]
    pub disable_chart_overlays: bool,
    #[serde(default)]
    pub maps_only: bool,
}

/// One timed scene. `timeline_start_us`/`duration_us` are the canonical plan
/// offsets; they are the only authority for clipping a projected phrase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneV2 {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub execution_mode: String,
    #[serde(default)]
    pub short_phrase_overlay: bool,
    #[serde(default)]
    pub overlay_categories: BTreeMap<String, String>,
    #[serde(default)]
    pub voiceover: Option<VoiceoverV2>,
    #[serde(default)]
    pub timeline_start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
}

/// The scene's certified voiceover projection. Only the timing is needed by
/// the editorial/chapter phase; audio paths and hashes stay in Go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceoverV2 {
    #[serde(default)]
    pub timing: Option<TimingV2>,
}

/// Certified word-level timing, copied verbatim from the audio authority.
/// It is never estimated from text length or scene duration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingV2 {
    pub duration_us: i64,
    #[serde(default)]
    pub words: Vec<WordTimingV2>,
}

/// One word boundary, in integer microseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTimingV2 {
    pub index: i64,
    #[serde(default)]
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
}

/// The editorial manifest is a pure projection of the Rust extractor. The
/// request carries it so the scene projection can ground titles and bullets
/// without a second extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialManifestV2 {
    pub schema_version: String,
    #[serde(default)]
    pub source_language: String,
    #[serde(default)]
    pub profile_version: String,
    #[serde(default)]
    pub scenes: Vec<EditorialSceneV2>,
    #[serde(default)]
    pub certified: bool,
    #[serde(default)]
    pub fingerprint: String,
}

/// One editorial scene, in manifest order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialSceneV2 {
    pub scene_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_status: String,
    #[serde(default)]
    pub title_source: String,
    #[serde(default)]
    pub bullets: Vec<EditorialBulletV2>,
}

/// One extractive editorial bullet. Sentence indices are transcript-global
/// when the manifest says so, otherwise scene-local; the projection uses the
/// text span and the scene bounds, never a guessed timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorialBulletV2 {
    pub sentence_start: i64,
    pub sentence_end: i64,
    pub text: String,
}

/// The chapter manifest is an extractive projection of the same transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterManifestV2 {
    pub schema_version: String,
    #[serde(default)]
    pub chapters: Vec<ChapterV2>,
}

/// One chapter; only its bullets participate in the overlay projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterV2 {
    #[serde(default)]
    pub bullets: Vec<ChapterBulletV2>,
}

/// One chapter bullet. The text must ground verbatim in the owning body scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterBulletV2 {
    pub text: String,
    #[serde(default)]
    pub start_sentence: i64,
    #[serde(default)]
    pub end_sentence: i64,
}

/// The projection response: only the phrases the scene projection added and
/// the scene IDs whose bullet slot is now owned by a chapter card. The caller
/// keeps its complete `SceneInput` DTOs and appends these annotations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialPlanV2Response {
    pub schema_version: String,
    #[serde(default)]
    pub scene_phrases: BTreeMap<String, Vec<ScenePhraseV2>>,
    #[serde(default)]
    pub covered_scenes: Vec<String>,
}

/// One projected editorial/chapter phrase, with certified timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePhraseV2 {
    pub text: String,
    pub start_us: i64,
    pub duration_us: i64,
    pub score: f64,
    pub presentation_role: String,
}

impl EditorialPlanV2Request {
    /// Structural, fail-closed validation. A request that fails here must not
    /// reach any decision code: a malformed manifest is a caller bug, not a
    /// silent no-op.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(format!(
                "editorial plan: unsupported schema_version {:?}, want {:?}",
                self.schema_version, EDITORIAL_PLAN_V2_SCHEMA_VERSION
            ));
        }
        if self.plan.plan_id.trim().is_empty() || self.plan.video_id.trim().is_empty() {
            return Err("editorial plan: plan_id and video_id are required".to_string());
        }
        if self.plan.width <= 0 || self.plan.height <= 0 {
            return Err("editorial plan: width and height must be positive".to_string());
        }
        if self.plan.fps_num <= 0 || self.plan.fps_den <= 0 {
            return Err("editorial plan: fps_num and fps_den must be positive".to_string());
        }
        if self.language.trim().is_empty() {
            return Err("editorial plan: language is required".to_string());
        }
        validate_categories("canvas", &self.canvas.overlay_categories)?;

        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.id.trim();
            if id.is_empty() {
                return Err("editorial plan: scene id is required".to_string());
            }
            if !scene_ids.insert(id) {
                return Err(format!("editorial plan: duplicate scene id {id:?}"));
            }
            if scene.duration_ms < 0 {
                return Err(format!(
                    "editorial plan: scene {id:?} has negative duration_ms"
                ));
            }
            if scene.timeline_start_us < 0 || scene.duration_us < 0 {
                return Err(format!(
                    "editorial plan: scene {id:?} has negative canonical bounds"
                ));
            }
            validate_categories(id, &scene.overlay_categories)?;
            if let Some(voiceover) = &scene.voiceover {
                if let Some(timing) = &voiceover.timing {
                    validate_timing(id, timing)?;
                }
            }
        }

        if let Some(editorial) = &self.editorial {
            editorial.validate()?;
        }
        if let Some(chapters) = &self.chapters {
            chapters.validate()?;
        }
        Ok(())
    }
}

impl EditorialManifestV2 {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != EDITORIAL_MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "editorial plan: unsupported editorial schema {:?}, want {:?}",
                self.schema_version, EDITORIAL_MANIFEST_SCHEMA_VERSION
            ));
        }
        if !self
            .profile_version
            .eq_ignore_ascii_case(EDITORIAL_PROFILE_VERSION)
        {
            return Err(format!(
                "editorial plan: unsupported editorial profile {:?}",
                self.profile_version
            ));
        }
        if !is_lower_hex_64(&self.fingerprint) {
            return Err(
                "editorial plan: editorial fingerprint must be 64 lowercase hex characters"
                    .to_string(),
            );
        }
        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.scene_id.trim();
            if id.is_empty() {
                return Err("editorial plan: editorial scene_id is required".to_string());
            }
            if !scene_ids.insert(id) {
                return Err(format!("editorial plan: duplicate editorial scene {id:?}"));
            }
            for bullet in &scene.bullets {
                if bullet.sentence_start < 0 || bullet.sentence_end <= bullet.sentence_start {
                    return Err(format!(
                        "editorial plan: editorial bullet in {id:?} has invalid sentence span"
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ChapterManifestV2 {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != CHAPTER_MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "editorial plan: unsupported chapter schema {:?}, want {:?}",
                self.schema_version, CHAPTER_MANIFEST_SCHEMA_VERSION
            ));
        }
        for chapter in &self.chapters {
            for bullet in &chapter.bullets {
                if bullet.text.trim().is_empty() {
                    return Err("editorial plan: chapter bullet text is required".to_string());
                }
                if bullet.start_sentence < 0 || bullet.end_sentence <= bullet.start_sentence {
                    return Err(
                        "editorial plan: chapter bullet has an invalid sentence span".to_string(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl EditorialPlanV2Response {
    /// Structural validation of the worker response. The caller must reject a
    /// malformed response before appending any phrase to its scene DTOs.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(format!(
                "editorial plan response: unsupported schema_version {:?}",
                self.schema_version
            ));
        }
        let mut covered = BTreeSet::new();
        for scene_id in &self.covered_scenes {
            if scene_id.trim().is_empty() {
                return Err("editorial plan response: covered scene id is empty".to_string());
            }
            if !covered.insert(scene_id) {
                return Err(format!(
                    "editorial plan response: duplicate covered scene {scene_id:?}"
                ));
            }
        }
        for (scene_id, phrases) in &self.scene_phrases {
            if scene_id.trim().is_empty() {
                return Err("editorial plan response: phrase scene id is empty".to_string());
            }
            for phrase in phrases {
                if phrase.text.trim().is_empty() {
                    return Err(format!(
                        "editorial plan response: empty phrase in scene {scene_id:?}"
                    ));
                }
                if phrase.start_us < 0 || phrase.duration_us <= 0 {
                    return Err(format!(
                        "editorial plan response: invalid phrase window in scene {scene_id:?}"
                    ));
                }
                if !matches!(
                    phrase.presentation_role.as_str(),
                    "scene_title" | "scene_bullet" | "chapter_bullet"
                ) {
                    return Err(format!(
                        "editorial plan response: invalid presentation_role {:?}",
                        phrase.presentation_role
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_categories(label: &str, categories: &BTreeMap<String, String>) -> Result<(), String> {
    for (category, toggle) in categories {
        if !OVERLAY_CATEGORIES.contains(&category.as_str()) {
            return Err(format!(
                "editorial plan: unknown overlay category {category:?} in {label:?}"
            ));
        }
        if !matches!(toggle.as_str(), "" | "default" | "enabled" | "disabled") {
            return Err(format!(
                "editorial plan: overlay category {category:?} in {label:?} has invalid toggle {toggle:?}"
            ));
        }
    }
    Ok(())
}

fn validate_timing(scene_id: &str, timing: &TimingV2) -> Result<(), String> {
    if timing.duration_us <= 0 {
        return Err(format!(
            "editorial plan: scene {scene_id:?} timing duration must be positive"
        ));
    }
    if timing.words.is_empty() {
        return Err(format!(
            "editorial plan: scene {scene_id:?} timing has no words"
        ));
    }
    let mut previous_end = 0_i64;
    for (position, word) in timing.words.iter().enumerate() {
        if word.index != position as i64 {
            return Err(format!(
                "editorial plan: scene {scene_id:?} word index {} at position {position}",
                word.index
            ));
        }
        if word.start_us < 0 || word.end_us < word.start_us {
            return Err(format!(
                "editorial plan: scene {scene_id:?} word {position} has invalid range"
            ));
        }
        if position > 0 && word.start_us < previous_end {
            return Err(format!(
                "editorial plan: scene {scene_id:?} word {position} is not monotonic"
            ));
        }
        if word.end_us > timing.duration_us {
            return Err(format!(
                "editorial plan: scene {scene_id:?} word {position} exceeds the audio duration"
            ));
        }
        previous_end = word.end_us;
    }
    Ok(())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{
        ChapterBulletV2, ChapterManifestV2, ChapterV2, EditorialBulletV2, EditorialManifestV2,
        EditorialPlanV2Request, EditorialPlanV2Response, EditorialSceneV2, PlanIdentityV2,
        ScenePhraseV2, SceneV2, TimingV2, VoiceoverV2, WordTimingV2,
        CHAPTER_MANIFEST_SCHEMA_VERSION, EDITORIAL_MANIFEST_SCHEMA_VERSION,
        EDITORIAL_PLAN_V2_SCHEMA_VERSION, EDITORIAL_PROFILE_VERSION,
    };
    use std::collections::BTreeMap;

    fn plan() -> PlanIdentityV2 {
        PlanIdentityV2 {
            plan_id: "plan-1".to_string(),
            video_id: "video-1".to_string(),
            project_id: "project-1".to_string(),
            motion_seed: "seed-1".to_string(),
            coherence_seed: String::new(),
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
        }
    }

    fn timing() -> TimingV2 {
        TimingV2 {
            duration_us: 2_000_000,
            words: vec![
                WordTimingV2 {
                    index: 0,
                    text: "Nike".to_string(),
                    start_us: 100_000,
                    end_us: 400_000,
                },
                WordTimingV2 {
                    index: 1,
                    text: "corre".to_string(),
                    start_us: 400_000,
                    end_us: 800_000,
                },
            ],
        }
    }

    fn scene() -> SceneV2 {
        SceneV2 {
            id: "scene-1".to_string(),
            text: "Nike corre. La statua è al museo.".to_string(),
            duration_ms: 2_000,
            execution_mode: "body".to_string(),
            short_phrase_overlay: false,
            overlay_categories: BTreeMap::new(),
            voiceover: Some(VoiceoverV2 {
                timing: Some(timing()),
            }),
            timeline_start_us: 0,
            duration_us: 2_000_000,
        }
    }

    fn editorial() -> EditorialManifestV2 {
        EditorialManifestV2 {
            schema_version: EDITORIAL_MANIFEST_SCHEMA_VERSION.to_string(),
            source_language: "it".to_string(),
            profile_version: EDITORIAL_PROFILE_VERSION.to_string(),
            certified: true,
            fingerprint: "a".repeat(64),
            scenes: vec![EditorialSceneV2 {
                scene_id: "scene-1".to_string(),
                title: Some("Nike corre".to_string()),
                title_status: "resolved".to_string(),
                title_source: "sentence".to_string(),
                bullets: vec![EditorialBulletV2 {
                    sentence_start: 0,
                    sentence_end: 1,
                    text: "Nike corre.".to_string(),
                }],
            }],
        }
    }

    fn chapters() -> ChapterManifestV2 {
        ChapterManifestV2 {
            schema_version: CHAPTER_MANIFEST_SCHEMA_VERSION.to_string(),
            chapters: vec![ChapterV2 {
                bullets: vec![ChapterBulletV2 {
                    text: "Nike corre.".to_string(),
                    start_sentence: 0,
                    end_sentence: 1,
                }],
            }],
        }
    }

    fn request() -> EditorialPlanV2Request {
        EditorialPlanV2Request {
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            plan: plan(),
            language: "it".to_string(),
            source_language: "it".to_string(),
            canvas: Default::default(),
            scenes: vec![scene()],
            editorial: Some(editorial()),
            chapters: Some(chapters()),
        }
    }

    #[test]
    fn v2_request_round_trips_through_json() {
        let request = request();
        request.validate().expect("valid request");
        let encoded = serde_json::to_string(&request).expect("encode request");
        let decoded: EditorialPlanV2Request =
            serde_json::from_str(&encoded).expect("decode request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn v2_rejects_unknown_schema_version() {
        let mut request = request();
        request.schema_version = "editorial_plan.v1".to_string();
        assert!(request
            .validate()
            .expect_err("unknown schema")
            .contains("unsupported schema_version"));
    }

    #[test]
    fn v2_rejects_duplicate_scene_ids() {
        let mut request = request();
        request.scenes.push(request.scenes[0].clone());
        assert!(request
            .validate()
            .expect_err("duplicate scene")
            .contains("duplicate scene id"));
    }

    #[test]
    fn v2_rejects_non_monotonic_word_timing() {
        let mut request = request();
        let timing = request.scenes[0]
            .voiceover
            .as_mut()
            .expect("voiceover")
            .timing
            .as_mut()
            .expect("timing");
        timing.words[1].start_us = 50_000;
        assert!(request
            .validate()
            .expect_err("non-monotonic")
            .contains("not monotonic"));
    }

    #[test]
    fn v2_rejects_malformed_editorial_fingerprint() {
        let mut request = request();
        request.editorial.as_mut().expect("editorial").fingerprint = "A".repeat(64);
        assert!(request
            .validate()
            .expect_err("fingerprint")
            .contains("64 lowercase hex"));
    }

    #[test]
    fn v2_rejects_invalid_overlay_category() {
        let mut request = request();
        request
            .canvas
            .overlay_categories
            .insert("unknown".to_string(), "enabled".to_string());
        assert!(request
            .validate()
            .expect_err("category")
            .contains("unknown overlay category"));
    }

    #[test]
    fn v2_response_validates_projected_phrases() {
        let mut phrases = BTreeMap::new();
        phrases.insert(
            "scene-1".to_string(),
            vec![ScenePhraseV2 {
                text: "Nike corre.".to_string(),
                start_us: 0,
                duration_us: 1_500_000,
                score: 1.0,
                presentation_role: "scene_title".to_string(),
            }],
        );
        let response = EditorialPlanV2Response {
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            scene_phrases: phrases,
            covered_scenes: vec!["scene-1".to_string()],
        };
        response.validate().expect("valid response");
    }

    #[test]
    fn v2_response_rejects_unknown_role() {
        let mut phrases = BTreeMap::new();
        phrases.insert(
            "scene-1".to_string(),
            vec![ScenePhraseV2 {
                text: "Nike corre.".to_string(),
                start_us: 0,
                duration_us: 1_500_000,
                score: 1.0,
                presentation_role: "title".to_string(),
            }],
        );
        let response = EditorialPlanV2Response {
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            scene_phrases: phrases,
            covered_scenes: Vec::new(),
        };
        assert!(response
            .validate()
            .expect_err("role")
            .contains("invalid presentation_role"));
    }
}
