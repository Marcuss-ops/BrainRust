//! Frame-accurate map collision resolution for PipelineGen overlay plans.
//!
//! The operation receives only fields needed by the timing algorithm and
//! returns timing edits keyed by original item index. PipelineGen retains the
//! complete overlay DTO and applies those edits without round-tripping opaque
//! render metadata through Rust.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use semantic_core::entity_capabilities::EntityKind;

mod chapter_bullet;
mod editorial_plan_v2;
mod numeric;
pub use chapter_bullet::*;
pub use editorial_plan_v2::*;

mod map;
pub use map::*;
/// One request for the editorial-plan batch. The caller supplies the planned
/// overlay items and the already-extracted entity/entrance facts; Rust owns
/// the timing, ranking and cascade decisions in one worker round-trip.
#[derive(Debug, Clone, Deserialize)]
pub struct CompileEditorialPlanRequest {
    #[serde(default)]
    pub fps_num: i64,
    #[serde(default)]
    pub fps_den: i64,
    #[serde(default)]
    pub scene_windows: BTreeMap<String, [i64; 2]>,
    #[serde(default)]
    pub items: Vec<TimingItem>,
    #[serde(default)]
    pub entity_scenes: Vec<EntityRankingScene>,
    #[serde(default)]
    pub entrance_items: Vec<CascadeEntranceItem>,
    /// Optional full-plan projection carried by the same batch operation. The
    /// chapter phase consumes the v2 scenes and chapter manifest; the caller
    /// still applies the returned phrases to its complete scene DTOs.
    #[serde(default)]
    pub v2: Option<EditorialPlanV2Request>,
}

/// The four batch sections PipelineGen used to request separately. Keeping
/// them as sparse edits means the caller still applies them to its complete
/// overlay DTO without round-tripping opaque render metadata through Rust.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompiledEditorialPlan {
    pub pre_budget_edits: Vec<TimingEdit>,
    pub timing_edits: Vec<TimingEdit>,
    pub ranked_entities: Vec<Vec<usize>>,
    pub entrance_edits: Vec<EntranceMotionEdit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v2: Option<EditorialPlanV2Response>,
}

/// Compiles the editorial/timing sections of an overlay plan in one call.
/// This is the batch entry point; the per-concern functions remain the pure
/// building blocks and keep their dedicated protocol handlers for compatibility.
pub fn compile_editorial_plan(request: CompileEditorialPlanRequest) -> CompiledEditorialPlan {
    let v2 = request.v2.as_ref().map(project_chapter_bullets);
    let pre_budget_edits = trim_map_windows_before_budget(ResolveMapOverlapsRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: request.items.clone(),
        map_presentation: None,
    });
    let timing_edits = resolve_map_overlaps(ResolveMapOverlapsRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        scene_windows: request.scene_windows,
        items: request.items,
        map_presentation: None,
    });
    let ranked_entities = rank_entity_overlays(RankEntityOverlaysRequest {
        scenes: request.entity_scenes,
    });
    let entrance_edits = cascade_clustered_entrances(CascadeEntrancesRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        items: request.entrance_items,
    });
    CompiledEditorialPlan {
        pre_budget_edits,
        timing_edits,
        ranked_entities,
        entrance_edits,
        v2,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
